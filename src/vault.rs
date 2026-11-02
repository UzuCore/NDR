//! NDR single-file encrypted filesystem, format v2.
//! Inspired by the appendable single-file block index of CoverFS, but not
//! derived from its crypto implementation. AES-256-GCM is provided by OpenSSL.
//! No SQL, no virtual-drive driver, no plaintext staging on normal operations.
use openssl::{hash::MessageDigest, pkcs5::pbkdf2_hmac, rand::rand_bytes, symm::{decrypt_aead, encrypt_aead, Cipher}};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashSet}, fs::{self, File, OpenOptions}, io::{Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}, time::{Duration, Instant}};
use zeroize::{Zeroize, Zeroizing};

pub const EXTENSION: &str = "ndr";
pub const STREAM_CHUNK_SIZE: usize = 8 * 1024 * 1024;
pub const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"NDRV2FS!";
const HEADER_SIZE: u64 = 128;
const SLOT_SIZE: u64 = 32;
const KDF_ITERATIONS: usize = 300_000;
const MAX_META: usize = 64 * 1024 * 1024;
const RECORD_HEADER: usize = 33; // kind, ciphertext length, random nonce, GCM tag
const KIND_META: u8 = 0x4d;
const KIND_DATA: u8 = 0x44;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPolicy {
    CloseOnLeave, Inactive(Duration), Elapsed(Duration), UntilExit,
}
impl SessionPolicy {
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let minutes = settings.values.get("VaultSessionMinutes").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 1440) as u64;
        match settings.text("VaultSessionPolicy", "elapsed").as_str() {
            "exit" => Self::UntilExit,
            _ => Self::Elapsed(Duration::from_secs(minutes * 60)),
        }
    }
    pub fn expired(self, opened: Instant, last_used: Instant, now: Instant) -> bool {
        match self {
            Self::Inactive(duration) => now.saturating_duration_since(last_used) >= duration,
            Self::Elapsed(duration) => now.saturating_duration_since(opened) >= duration,
            Self::CloseOnLeave | Self::UntilExit => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultItem { pub id: i64, pub name: String, pub folder: bool, pub size: i64 }
impl Drop for VaultItem { fn drop(&mut self) { self.name.zeroize(); } }

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Block { offset: u64, size: u32 }
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Item {
    id: i64, parent: i64, name: String, folder: bool, size: u64,
    blocks: Vec<Block>,
}
impl Item {
    fn public(&self) -> Result<VaultItem, String> {
        Ok(VaultItem { id: self.id, name: self.name.clone(), folder: self.folder,
            size: i64::try_from(self.size).map_err(|_| "File size overflow")? })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Index { next_id: i64, items: BTreeMap<i64, Item> }
impl Index {
    fn empty() -> Self {
        let mut items = BTreeMap::new();
        items.insert(1, Item { id: 1, parent: 0, name: String::new(), folder: true, size: 0, blocks: vec![] });
        Self { next_id: 2, items }
    }
    fn check(&self) -> Result<(), String> {
        if self.items.len() > 2_000_000 { return Err("Encrypted index exceeds safety limit".into()); }
        if !self.items.get(&1).is_some_and(|r| r.folder && r.parent == 0) { return Err("Encrypted root is invalid".into()); }
        let mut pairs = HashSet::new();
        for item in self.items.values() {
            if item.id == 1 { continue; }
            valid_name(&item.name)?;
            if !self.items.get(&item.parent).is_some_and(|p| p.folder) { return Err("Invalid encrypted parent reference".into()); }
            if !pairs.insert((item.parent, item.name.to_lowercase())) { return Err("Duplicate encrypted names".into()); }
            if item.folder && (item.size != 0 || !item.blocks.is_empty()) { return Err("Invalid encrypted folder contents".into()); }
        }
        Ok(())
    }
}
pub struct Vault {
    handle: File, pub file: PathBuf, key: Zeroizing<[u8;32]>, index: Index,
    generation: u64, pending: Option<Index>, opened: Instant, last_used: Instant,
}
fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.len() > 255 || name.trim() != name
        || name.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)) {
        return Err("Invalid container entry name".into());
    }
    Ok(())
}
fn reserved(name: &str) -> bool {
    if name.ends_with(['.', ' ']) { return true; }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(),"CON"|"PRN"|"AUX"|"NUL"|"COM1"|"COM2"|"COM3"|"COM4"|"COM5"|"COM6"|"COM7"|"COM8"|"COM9"|"LPT1"|"LPT2"|"LPT3"|"LPT4"|"LPT5"|"LPT6"|"LPT7"|"LPT8"|"LPT9")
}
fn derive(password: String, salt: &[u8]) -> Result<Zeroizing<[u8;32]>, String> {
    if password.chars().count() < 6 { return Err("Password must contain at least 6 characters".into()); }
    let password = Zeroizing::new(password);
    let mut key = Zeroizing::new([0u8;32]);
    pbkdf2_hmac(password.as_bytes(), salt, KDF_ITERATIONS, MessageDigest::sha256(), &mut *key)
        .map_err(|e| format!("Key derivation failed: {e}"))?;
    Ok(key)
}
fn record_size(len: usize) -> Result<u64, String> {
    u64::try_from(RECORD_HEADER.checked_add(len).ok_or("Encrypted record length overflow")?)
        .map_err(|e| e.to_string())
}
fn put_record(handle: &mut File, key: &[u8;32], kind: u8, plaintext: &[u8]) -> Result<Block, String> {
    let length = u32::try_from(plaintext.len()).map_err(|_| "Encrypted record is too large")?;
    let offset = handle.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    let mut nonce = [0u8;12];
    rand_bytes(&mut nonce).map_err(|e| e.to_string())?;
    let mut aad = [0u8;5];
    aad[0] = kind;
    aad[1..5].copy_from_slice(&length.to_le_bytes());
    // Bind authenticated ciphertext to its immutable physical record offset.
    // In-vault references may share the record, but swapping record bytes
    // between offsets cannot substitute an authenticated plaintext chunk.
    let mut record_aad = [0u8;13];
    record_aad[..5].copy_from_slice(&aad);
    record_aad[5..].copy_from_slice(&offset.to_le_bytes());
    let mut tag = [0u8;16];
    let cipher = encrypt_aead(Cipher::aes_256_gcm(), key, Some(&nonce), &record_aad, plaintext, &mut tag)
        .map_err(|e| format!("Encryption failed: {e}"))?;
    handle.write_all(&aad).and_then(|_| handle.write_all(&nonce))
        .and_then(|_| handle.write_all(&tag)).and_then(|_| handle.write_all(&cipher))
        .map_err(|e| e.to_string())?;
    Ok(Block { offset, size: length })
}
fn get_record(handle: &File, key: &[u8;32], block: &Block, expected: u8) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut handle = handle.try_clone().map_err(|e| e.to_string())?;
    let size = handle.metadata().map_err(|e| e.to_string())?.len();
    if block.offset < HEADER_SIZE || block.offset.checked_add(record_size(block.size as usize)?).is_none_or(|end| end > size) {
        return Err("Encrypted block points outside container".into());
    }
    handle.seek(SeekFrom::Start(block.offset)).map_err(|e| e.to_string())?;
    let mut header = [0u8;RECORD_HEADER];
    handle.read_exact(&mut header).map_err(|e| e.to_string())?;
    if header[0] != expected || u32::from_le_bytes(header[1..5].try_into().unwrap()) != block.size {
        return Err("Encrypted record header mismatch".into());
    }
    let mut cipher = vec![0u8; block.size as usize];
    handle.read_exact(&mut cipher).map_err(|e| e.to_string())?;
    let mut record_aad = [0u8;13];
    record_aad[..5].copy_from_slice(&header[..5]);
    record_aad[5..].copy_from_slice(&block.offset.to_le_bytes());
    let plain = decrypt_aead(Cipher::aes_256_gcm(), key, Some(&header[5..17]), &record_aad, &cipher, &header[17..33])
        .map_err(|_| "Wrong password, damaged encrypted container, or authentication failure".to_owned())?;
    Ok(Zeroizing::new(plain))
}
fn checksum(slot: &[u8]) -> u32 {
    let digest = openssl::sha::sha256(&slot[..24]);
    u32::from_le_bytes(digest[..4].try_into().unwrap())
}
fn slot_bytes(generation: u64, block: &Block) -> [u8;32] {
    let mut out = [0u8;32];
    out[..8].copy_from_slice(&generation.to_le_bytes());
    out[8..16].copy_from_slice(&block.offset.to_le_bytes());
    out[16..24].copy_from_slice(&(record_size(block.size as usize).unwrap_or(0)).to_le_bytes());
    let digest=checksum(&out);
    out[24..28].copy_from_slice(&digest.to_le_bytes());
    out
}
fn read_slot(header: &[u8;128], index: usize, len: u64) -> Option<(u64,Block)> {
    let at = 32 + index * SLOT_SIZE as usize;
    let slot = &header[at..at + SLOT_SIZE as usize];
    if u32::from_le_bytes(slot[24..28].try_into().ok()?) != checksum(slot) { return None; }
    let generation = u64::from_le_bytes(slot[..8].try_into().ok()?);
    let offset = u64::from_le_bytes(slot[8..16].try_into().ok()?);
    let record_len = u64::from_le_bytes(slot[16..24].try_into().ok()?);
    if generation == 0 || record_len < RECORD_HEADER as u64 || record_len > MAX_META as u64 + RECORD_HEADER as u64
        || offset < HEADER_SIZE || offset.checked_add(record_len)? > len { return None; }
    Some((generation,Block { offset, size: u32::try_from(record_len - RECORD_HEADER as u64).ok()? }))
}

impl Vault {
    pub fn create(path: &Path, password: String) -> Result<Self, String> {
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) { return Err("Encrypted containers must use .ndr extension".into()); }
        let mut handle = OpenOptions::new().read(true).write(true).create_new(true).open(path).map_err(|e| e.to_string())?;
        let result = (|| {
            let mut header = [0u8;128];
            header[..8].copy_from_slice(MAGIC);
            rand_bytes(&mut header[8..24]).map_err(|e| e.to_string())?;
            header[24..28].copy_from_slice(&(KDF_ITERATIONS as u32).to_le_bytes());
            handle.write_all(&header).map_err(|e| e.to_string())?;
            let key = derive(password, &header[8..24])?;
            let mut result = Self {handle,file:path.to_path_buf(),key,index:Index::empty(),
                generation:0,pending:None,opened:Instant::now(),last_used:Instant::now()};
            result.commit_index()?;
            Ok::<_,String>(result)
        })();
        if result.is_err() { let _ = fs::remove_file(path); }
        result
    }
    pub fn unlock(path: &Path, password: String) -> Result<Self, String> {
        let mut handle = OpenOptions::new().read(true).write(true).open(path).map_err(|e| e.to_string())?;
        let mut header = [0u8;128];
        handle.read_exact(&mut header).map_err(|_| "Not an NDR v2 container (legacy SQLCipher format is unsupported)".to_owned())?;
        if &header[..8] != MAGIC { return Err("Not an NDR v2 container (legacy SQLCipher format is unsupported)".into()); }
        if u32::from_le_bytes(header[24..28].try_into().unwrap()) as usize != KDF_ITERATIONS { return Err("Unsupported NDR key derivation format".into()); }
        let key = derive(password, &header[8..24])?;
        let len = handle.metadata().map_err(|e| e.to_string())?.len();
        let mut slots: Vec<_> = (0..2).filter_map(|i| read_slot(&header,i,len)).collect();
        slots.sort_by(|a,b| b.0.cmp(&a.0));
        for (generation,block) in slots {
            if let Ok(bytes) = get_record(&handle,&key,&block,KIND_META) {
                if let Ok(index) = serde_json::from_slice::<Index>(&bytes) {
                    if index.check().is_ok() {
                        return Ok(Self { handle,file:path.to_path_buf(),key,index,generation,pending:None,
                            opened:Instant::now(),last_used:Instant::now() });
                    }
                }
            }
        }
        Err("Wrong password or encrypted container metadata is corrupt".into())
    }
    fn commit_index(&mut self) -> Result<(), String> {
        self.index.check()?;
        let data = Zeroizing::new(serde_json::to_vec(&self.index).map_err(|e| e.to_string())?);
        if data.len() > MAX_META { return Err("Encrypted folder index exceeds 64 MiB".into()); }
        let block = put_record(&mut self.handle,&self.key,KIND_META,&data)?;
        self.handle.sync_data().map_err(|e| e.to_string())?;
        let generation = self.generation.checked_add(1).ok_or("Encrypted generation overflow")?;
        let slot = slot_bytes(generation,&block);
        self.handle.seek(SeekFrom::Start(32 + (generation % 2) * SLOT_SIZE)).map_err(|e| e.to_string())?;
        self.handle.write_all(&slot).and_then(|_| self.handle.sync_all()).map_err(|e| e.to_string())?;
        self.generation = generation;
        self.touch();
        Ok(())
    }
    fn transaction<T>(&mut self, operation: impl FnOnce(&mut Self) -> Result<T,String>) -> Result<T,String> {
        if self.pending.is_some() { return Err("Encrypted file stream is in progress".into()); }
        let old = self.index.clone();
        match operation(self).and_then(|value| {self.commit_index()?;Ok(value)}) {
            Ok(value) => Ok(value),
            Err(error) => {self.index = old;Err(error)}
        }
    }
    pub fn expired(&self, policy: SessionPolicy) -> bool { policy.expired(self.opened,self.last_used,Instant::now()) }
    pub fn touch(&mut self) { self.last_used = Instant::now(); }
    fn folder(&self,id:i64) -> Result<(),String> {
        if self.index.items.get(&id).is_some_and(|i|i.folder) {Ok(())} else {Err("Encrypted folder was not found".into())}
    }
    fn item(&self,id:i64) -> Result<&Item,String> {self.index.items.get(&id).ok_or_else(||"Encrypted item not found".into())}
    fn child(&self,parent:i64,name:&str) -> Option<&Item> {
        self.index.items.values().find(|i|i.parent==parent&&i.name.eq_ignore_ascii_case(name))
    }
    fn new_item(&mut self,parent:i64,name:&str,folder:bool) -> Result<i64,String> {
        valid_name(name)?;self.folder(parent)?;
        if self.child(parent,name).is_some(){return Err(format!("Destination already exists: {name}"));}
        let id=self.index.next_id;
        self.index.next_id=id.checked_add(1).ok_or("Encrypted item ID overflow")?;
        self.index.items.insert(id,Item{id,parent,name:name.into(),folder,size:0,blocks:Vec::new()});
        Ok(id)
    }
    fn subtree(&self,id:i64) -> Result<Vec<i64>,String> {
        self.item(id)?;
        if id==1{return Err("Cannot delete container root".into());}
        let mut result=vec![id];
        for at in 0..result.len(){
            let children:Vec<_>=self.index.items.values().filter(|i|i.parent==result[at]).map(|i|i.id).collect();
            if result.len()+children.len()>2_000_000{return Err("Encrypted tree exceeds item limit".into());}
            result.extend(children);
        }
        Ok(result)
    }
    fn erase(&mut self,id:i64)->Result<(),String> {
        for at in self.subtree(id)? { self.index.items.remove(&at); }
        Ok(())
    }
    fn replace(&mut self,parent:i64,name:&str,replace:bool)->Result<(),String>{
        if let Some(existing)=self.child(parent,name).map(|i|i.id){
            if !replace{return Err(format!("Destination already exists: {name}"));}
            self.erase(existing)?;
        }
        Ok(())
    }
    pub fn list(&mut self,parent:i64)->Result<Vec<VaultItem>,String>{
        self.folder(parent)?;
        let mut items:Vec<_>=self.index.items.values().filter(|i|i.parent==parent).map(Item::public).collect::<Result<_,_>>()?;
        items.sort_by(|a,b|b.folder.cmp(&a.folder).then_with(||a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(items)
    }
    pub fn find_child(&self,parent:i64,name:&str)->Result<Option<VaultItem>,String>{
        self.folder(parent)?;self.child(parent,name).map(Item::public).transpose()
    }
    pub fn parent(&self,id:i64)->Result<i64,String>{
        let item=self.item(id)?;if !item.folder{return Err("Encrypted item is not a folder".into());}Ok(item.parent)
    }
    pub fn relative_folder_path(&self,mut folder:i64)->Result<PathBuf,String>{
        let mut parts=Vec::new();
        while folder!=1 {
            if parts.len()>64 {return Err("Encrypted folder depth exceeded".into());}
            let item=self.item(folder)?;if !item.folder{return Err("Encrypted path is not a folder".into());}
            parts.push(item.name.clone());folder=item.parent;
        }
        let mut path=PathBuf::new();for part in parts.into_iter().rev(){path.push(part);}Ok(path)
    }
    pub fn resolve_folder(&mut self,path:&str)->Result<i64,String>{
        let mut id=1;
        for part in path.trim_matches('/').split('/'){
            if part.is_empty(){continue;}
            valid_name(part)?;
            id=self.child(id,part).filter(|i|i.folder).ok_or_else(||format!("Encrypted destination folder not found: {part}"))?.id;
        }
        Ok(id)
    }
    fn clone_tree(&mut self,source_id:i64,to:i64,name:&str,depth:usize)->Result<usize,String>{
        if depth>64{return Err("Encrypted folder depth exceeded".into());}
        let source=self.item(source_id)?.clone();
        let id=self.new_item(to,name,source.folder)?;
        if !source.folder{
            let item=self.index.items.get_mut(&id).unwrap();
            item.size=source.size;
            // Immutable authenticated ciphertext is safely shared within one container.
            item.blocks=source.blocks;
            return Ok(1);
        }
        let children:Vec<_>=self.index.items.values().filter(|i|i.parent==source_id)
            .map(|i|(i.id,i.name.clone())).collect();
        let mut count=1;
        for (child,name) in children{count+=self.clone_tree(child,id,&name,depth+1)?;}
        Ok(count)
    }
    fn other_tree(&mut self,source:&Vault,source_id:i64,to:i64,name:&str,depth:usize)->Result<usize,String>{
        if depth>64{return Err("Encrypted folder depth exceeded".into());}
        let entry=source.item(source_id)?;
        let id=self.new_item(to,name,entry.folder)?;
        if !entry.folder{
            let mut copied=Vec::with_capacity(entry.blocks.len());
            for block in &entry.blocks{
                let plain=get_record(&source.handle,&source.key,block,KIND_DATA)?;
                copied.push(put_record(&mut self.handle,&self.key,KIND_DATA,&plain)?);
            }
            let target=self.index.items.get_mut(&id).unwrap();target.size=entry.size;target.blocks=copied;
            return Ok(1);
        }
        let children:Vec<_>=source.index.items.values().filter(|i|i.parent==source_id)
            .map(|i|(i.id,i.name.clone())).collect();
        let mut count=1;
        for (child,name) in children{count+=self.other_tree(source,child,id,&name,depth+1)?;}
        Ok(count)
    }
    pub fn transfer_items(&mut self,from:i64,names:&[String],to:i64,moving:bool)->Result<usize,String>{
        if names.is_empty(){return Ok(0);}
        self.folder(from)?;self.folder(to)?;
        let ids:Vec<_>=names.iter().map(|name|self.child(from,name).map(|i|i.id)
            .ok_or_else(||format!("Encrypted source item is missing: {name}"))).collect::<Result<_,_>>()?;
        if ids.iter().collect::<HashSet<_>>().len()!=ids.len(){return Err("Duplicate encrypted selection".into());}
        self.transaction(|this|{
            let mut count=0;
            for (id,name) in ids.iter().zip(names){
                if moving && from==to{continue;}
                if this.child(to,name).is_some(){return Err(format!("Destination already exists: {name}"));}
                if this.item(*id)?.folder && this.subtree(*id)?.contains(&to){
                    return Err("Cannot move a folder inside itself".into());
                }
                if moving{
                    let item=this.index.items.get_mut(id).unwrap();item.parent=to;count+=1;
                }else{count+=this.clone_tree(*id,to,name,0)?;}
            }
            Ok(count)
        })
    }
    pub fn transfer_one(&mut self,from:i64,name:&str,to:i64,target_name:&str,moving:bool,replace:bool)->Result<usize,String>{
        valid_name(target_name)?;self.folder(from)?;self.folder(to)?;
        let id=self.child(from,name).ok_or("Encrypted source item is missing")?.id;
        if self.item(id)?.folder && self.subtree(id)?.contains(&to){
            return Err("Cannot transfer a folder into its descendants".into());
        }
        if from==to && name.eq_ignore_ascii_case(target_name){
            if moving{return Ok(0);}else{return Err("Copying an item over itself is not permitted".into());}
        }
        self.transaction(|this|{
            this.replace(to,target_name,replace)?;
            if moving{
                let entry=this.index.items.get_mut(&id).ok_or("Source disappeared")?;
                entry.parent=to;entry.name=target_name.into();Ok(1)
            }else{this.clone_tree(id,to,target_name,0)}
        })
    }
    pub fn copy_from_vault(&mut self,source:&mut Vault,from:i64,names:&[String],to:i64)->Result<usize,String>{
        if names.is_empty(){return Ok(0);}
        if fs::canonicalize(&self.file).map_err(|e|e.to_string())? == fs::canonicalize(&source.file).map_err(|e|e.to_string())?{
            return Err("Use internal transfer for same container".into());
        }
        self.folder(to)?;source.folder(from)?;
        self.transaction(|this|{
            let mut count=0;
            for name in names{
                let id=source.child(from,name).ok_or("Encrypted source item is missing")?.id;
                count+=this.other_tree(source,id,to,name,0)?;
            }
            Ok(count)
        })
    }
    pub fn copy_from_vault_item(&mut self,source:&mut Vault,from:i64,name:&str,to:i64,target_name:&str,replace:bool)->Result<usize,String>{
        valid_name(target_name)?;self.folder(to)?;source.folder(from)?;
        if fs::canonicalize(&self.file).map_err(|e|e.to_string())? == fs::canonicalize(&source.file).map_err(|e|e.to_string())?{
            return Err("Use same-container transfer for same encrypted file".into());
        }
        let id=source.child(from,name).ok_or("Encrypted source item is missing")?.id;
        self.transaction(|this|{this.replace(to,target_name,replace)?;this.other_tree(source,id,to,target_name,0)})
    }
    pub fn add_folder(&mut self,parent:i64,name:&str)->Result<i64,String>{self.transaction(|this|this.new_item(parent,name,true))}
    pub fn add_file(&mut self,parent:i64,name:&str,data:&[u8])->Result<i64,String>{
        if data.len()>MAX_FILE_SIZE{return Err("File exceeds in-memory editing safety limit".into());}
        self.transaction(|this|{
            let id=this.new_item(parent,name,false)?;
            for chunk in data.chunks(STREAM_CHUNK_SIZE){
                let block=put_record(&mut this.handle,&this.key,KIND_DATA,chunk)?;
                let item=this.index.items.get_mut(&id).unwrap();
                item.blocks.push(block);item.size+=chunk.len() as u64;
            }
            Ok(id)
        })
    }
    pub fn begin_stream_file(&mut self,parent:i64,name:&str,replace:bool)->Result<i64,String>{
        if self.pending.is_some(){return Err("Another encrypted stream is open".into());}
        valid_name(name)?;self.folder(parent)?;
        let old=self.index.clone();
        let result=(||{self.replace(parent,name,replace)?;self.new_item(parent,name,false)})();
        match result{
            Ok(id)=>{self.pending=Some(old);Ok(id)}
            Err(error)=>{self.index=old;Err(error)}
        }
    }
    pub fn stream_write_chunk(&mut self,id:i64,ordinal:i64,bytes:&[u8])->Result<(),String>{
        if self.pending.is_none(){return Err("No encrypted stream open".into());}
        if ordinal<0 || bytes.len()>STREAM_CHUNK_SIZE{return Err("Invalid encrypted chunk".into());}
        let entry=self.item(id)?;
        if entry.folder || entry.blocks.len()!=ordinal as usize{return Err("Encrypted stream chunk order is invalid".into());}
        let block=put_record(&mut self.handle,&self.key,KIND_DATA,bytes)?;
        let item=self.index.items.get_mut(&id).unwrap();item.blocks.push(block);item.size+=bytes.len() as u64;Ok(())
    }
    pub fn stream_file_size(&self,id:i64)->Result<u64,String>{
        let item=self.item(id)?;
        if item.folder{return Err("Encrypted item is a folder".into());}Ok(item.size)
    }
    pub fn stream_read_chunk(&self,id:i64,ordinal:i64)->Result<Option<Zeroizing<Vec<u8>>>,String>{
        if ordinal<0{return Err("Invalid encrypted chunk ordinal".into());}
        let item=self.item(id)?;if item.folder{return Err("Encrypted item is a folder".into());}
        item.blocks.get(ordinal as usize).map(|block|get_record(&self.handle,&self.key,block,KIND_DATA)).transpose()
    }
    pub fn commit_stream_file(&mut self)->Result<(),String>{
        let before=self.pending.take().ok_or("No encrypted stream open")?;
        if let Err(error)=self.commit_index(){self.index=before;return Err(error);}Ok(())
    }
    pub fn rollback_stream_file(&mut self)->Result<(),String>{
        self.index=self.pending.take().ok_or("No encrypted stream open")?;self.touch();Ok(())
    }
    fn import_tree(&mut self,parent:i64,path:&Path,name:&str,depth:usize)->Result<usize,String>{
        if depth>64{return Err("Encrypted folder depth exceeded".into());}
        valid_name(name)?;
        let meta=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
        if is_reparse(&meta){return Err("Cannot import symbolic links, junctions or reparse points".into());}
        if meta.is_dir(){
            let id=self.new_item(parent,name,true)?;
            let mut count=1;
            for child in fs::read_dir(path).map_err(|e|e.to_string())?{
                let child=child.map_err(|e|e.to_string())?;
                let name=child.file_name().to_string_lossy().into_owned();
                count+=self.import_tree(id,&child.path(),&name,depth+1)?;
            }
            Ok(count)
        }else if meta.is_file(){
            let id=self.new_item(parent,name,false)?;
            let mut source=File::open(path).map_err(|e|e.to_string())?;
            let mut buffer=Zeroizing::new(vec![0u8;STREAM_CHUNK_SIZE]);
            loop{
                let length=source.read(&mut buffer).map_err(|e|e.to_string())?;
                if length==0{break;}
                let block=put_record(&mut self.handle,&self.key,KIND_DATA,&buffer[..length])?;
                let item=self.index.items.get_mut(&id).unwrap();
                item.blocks.push(block);item.size+=length as u64;
            }
            if self.item(id)?.size!=meta.len(){return Err("Source file changed during encrypted import".into());}
            Ok(1)
        }else{Err("Unsupported source file type".into())}
    }
    pub fn import_one(&mut self,to:i64,path:&Path,name:&str,replace:bool)->Result<usize,String>{
        valid_name(name)?;self.folder(to)?;
        let own=fs::canonicalize(&self.file).map_err(|e|e.to_string())?;
        let source=fs::canonicalize(path).map_err(|e|e.to_string())?;
        if own.starts_with(&source){return Err("Cannot import encrypted container or its parent folder".into());}
        self.transaction(|this|{this.replace(to,name,replace)?;this.import_tree(to,path,name,0)})
    }
    pub fn import_paths(&mut self,parent:i64,sources:&[PathBuf])->Result<usize,String>{
        if sources.is_empty(){return Ok(0);}
        self.folder(parent)?;
        let own=fs::canonicalize(&self.file).map_err(|e|e.to_string())?;
        for path in sources{
            let full=fs::canonicalize(path).map_err(|e|e.to_string())?;
            if own.starts_with(&full){return Err("Cannot import encrypted container or parent".into());}
        }
        self.transaction(|this|{
            let mut count=0;
            for path in sources{
                let name=path.file_name().ok_or("Import source lacks filename")?.to_string_lossy().into_owned();
                count+=this.import_tree(parent,path,&name,0)?;
            }
            Ok(count)
        })
    }
    fn export_tree(&self,id:i64,name:&str,target:&Path,created:&mut Vec<PathBuf>,depth:usize)->Result<usize,String>{
        if depth>64{return Err("Encrypted folder depth exceeded".into());}
        valid_name(name)?;if reserved(name){return Err("Reserved Windows export name".into());}
        let item=self.item(id)?;
        let path=target.join(name);
        if fs::symlink_metadata(&path).is_ok(){return Err(format!("Export target already exists: {}",path.display()));}
        if item.folder{
            fs::create_dir(&path).map_err(|e|e.to_string())?;
            created.push(path.clone());
            let children:Vec<_>=self.index.items.values().filter(|i|i.parent==id)
                .map(|i|(i.id,i.name.clone())).collect();
            let mut count=1;
            for (child,name) in children{count+=self.export_tree(child,&name,&path,created,depth+1)?;}
            Ok(count)
        }else{
            let mut output=OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|e.to_string())?;
            created.push(path);
            for block in &item.blocks{
                let data=get_record(&self.handle,&self.key,block,KIND_DATA)?;
                output.write_all(&data).map_err(|e|e.to_string())?;
            }
            output.flush().map_err(|e|e.to_string())?;
            Ok(1)
        }
    }
    pub fn export_items(&mut self,parent:i64,names:&[String],destination:&Path)->Result<usize,String>{
        self.folder(parent)?;ensure_real_dir(destination)?;
        let mut created=Vec::new();
        let result=(||{
            let mut count=0;
            for name in names{
                let id=self.child(parent,name).ok_or("Encrypted source item does not exist")?.id;
                count+=self.export_tree(id,name,destination,&mut created,0)?;
            }
            Ok(count)
        })();
        if result.is_err(){cleanup(&created);}result
    }
    pub fn export_one(&mut self,from:i64,name:&str,destination:&Path,target_name:&str,replace:bool)->Result<usize,String>{
        valid_name(target_name)?;ensure_real_dir(destination)?;self.folder(from)?;
        let id=self.child(from,name).ok_or("Encrypted source item does not exist")?.id;
        let final_path=destination.join(target_name);
        let mut backup=None;
        if let Ok(meta)=fs::symlink_metadata(&final_path){
            if !replace{return Err("Export destination already exists".into());}
            if is_reparse(&meta){return Err("Cannot replace symbolic link or junction".into());}
            let mut spare=final_path.clone();
            let mut salt=[0u8;8];rand_bytes(&mut salt).map_err(|e|e.to_string())?;
            spare.set_extension(format!("ndr-recovery-{}",u64::from_le_bytes(salt)));
            if spare.exists(){return Err("Recovery filename collision".into());}
            fs::rename(&final_path,&spare).map_err(|e|e.to_string())?;
            backup=Some(spare);
        }
        let mut created=Vec::new();
        let result=self.export_tree(id,target_name,destination,&mut created,0);
        if result.is_err(){
            cleanup(&created);
            if let Some(spare)=&backup{
                fs::rename(spare,&final_path).map_err(|e|format!("Export failed and recovery restore failed: {e}"))?;
            }
        }
        if result.is_ok(){
            if let Some(spare)=backup{
                if spare.is_dir(){fs::remove_dir_all(spare)}else{fs::remove_file(spare)}.map_err(|e|e.to_string())?;
            }
        }
        result
    }
    pub fn read_file(&mut self,id:i64)->Result<Vec<u8>,String>{
        let size=self.stream_file_size(id)?;
        if size>MAX_FILE_SIZE as u64{return Err("File too large for in-memory editing; export to edit".into());}
        let mut result=Vec::with_capacity(size as usize);
        for block in &self.item(id)?.blocks{
            let plain=get_record(&self.handle,&self.key,block,KIND_DATA)?;
            result.extend_from_slice(&plain);
        }
        Ok(result)
    }
    pub fn update_file(&mut self,id:i64,data:&[u8])->Result<(),String>{
        if data.len()>MAX_FILE_SIZE{return Err("File exceeds in-memory editing safety limit".into());}
        if self.item(id)?.folder{return Err("Encrypted item is a folder".into());}
        self.transaction(|this|{
            let mut blocks=Vec::new();
            for chunk in data.chunks(STREAM_CHUNK_SIZE){
                blocks.push(put_record(&mut this.handle,&this.key,KIND_DATA,chunk)?);
            }
            let item=this.index.items.get_mut(&id).unwrap();item.size=data.len() as u64;item.blocks=blocks;Ok(())
        })
    }
    pub fn rename(&mut self,id:i64,name:&str)->Result<(),String>{
        if id==1{return Err("Root folder cannot be renamed".into());}
        valid_name(name)?;
        let parent=self.item(id)?.parent;
        if self.child(parent,name).is_some_and(|item|item.id!=id){return Err("Destination name already exists".into());}
        self.transaction(|this|{this.index.items.get_mut(&id).unwrap().name=name.into();Ok(())})
    }
    pub fn remove(&mut self,id:i64)->Result<(),String>{self.remove_many(&[id])}
    pub fn remove_many(&mut self,ids:&[i64])->Result<(),String>{
        if ids.is_empty(){return Ok(());}
        if ids.iter().copied().collect::<HashSet<_>>().len()!=ids.len(){return Err("Duplicate delete selection".into());}
        self.transaction(|this|{
            for id in ids{this.item(*id)?;}
            for id in ids{if this.index.items.contains_key(id){this.erase(*id)?;}}
            Ok(())
        })
    }
    pub fn verify_integrity(&self)->Result<(),String>{
        self.index.check()?;
        for item in self.index.items.values(){
            if item.folder{continue;}
            let mut sum=0u64;
            for block in &item.blocks{
                get_record(&self.handle,&self.key,block,KIND_DATA)?;
                sum=sum.checked_add(block.size as u64).ok_or("Encrypted size overflow")?;
            }
            if sum!=item.size{return Err("Encrypted file length mismatch".into());}
        }
        Ok(())
    }
}
fn cleanup(paths:&[PathBuf]){
    for path in paths.iter().rev(){
        if path.is_dir(){let _=fs::remove_dir(path);}else{let _=fs::remove_file(path);}
    }
}
fn is_reparse(meta:&fs::Metadata)->bool{
    #[cfg(windows)]{
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes()&0x400!=0
    }
    #[cfg(not(windows))]{meta.file_type().is_symlink()}
}
fn ensure_real_dir(path:&Path)->Result<(),String>{
    let meta=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if !meta.is_dir()||is_reparse(&meta){return Err("Destination must be a real directory".into());}
    Ok(())
}
impl Drop for Vault {
    fn drop(&mut self){
        for item in self.index.items.values_mut(){item.name.zeroize();}
        if let Some(old)=self.pending.as_mut(){
            for item in old.items.values_mut(){item.name.zeroize();}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(label:&str)->PathBuf{
        let root=std::env::temp_dir().join(format!("ndr-v2-{label}-{}-{}",
            std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();root
    }
    #[test]
    fn create_unlock_and_password_rejection() {
        let root=fixture("auth");let path=root.join("auth.ndr");
        let mut v=Vault::create(&path,"correct secret".into()).unwrap();
        let sub=v.add_folder(1,"문서").unwrap();
        let file=v.add_file(sub,"hidden-name.txt",b"known secret payload").unwrap();
        assert_eq!(v.read_file(file).unwrap(),b"known secret payload");
        v.verify_integrity().unwrap();drop(v);
        assert!(Vault::unlock(&path,"wrong password".into()).is_err());
        let content=fs::read(&path).unwrap();
        assert!(!content.windows(11).any(|part|part==b"hidden-name"));
        assert!(!content.windows(b"known secret payload".len()).any(|part|part==b"known secret payload"));
        let mut reopened=Vault::unlock(&path,"correct secret".into()).unwrap();
        assert_eq!(reopened.resolve_folder("문서").unwrap(),sub);
        assert_eq!(reopened.read_file(file).unwrap(),b"known secret payload");
        drop(reopened);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn copy_move_rename_delete_and_reopen() {
        let root=fixture("ops");let path=root.join("data.ndr");
        let mut v=Vault::create(&path,"correct secret".into()).unwrap();
        let a=v.add_folder(1,"A").unwrap();
        let b=v.add_folder(1,"B").unwrap();
        let id=v.add_file(a,"story.txt",b"data").unwrap();
        let bytes=fs::metadata(&path).unwrap().len();
        assert_eq!(v.transfer_one(a,"story.txt",b,"copy.txt",false,false).unwrap(),1);
        assert!(fs::metadata(&path).unwrap().len()-bytes<30_000,"copy should share ciphertext, not recopy it");
        let copy=v.find_child(b,"copy.txt").unwrap().unwrap();
        assert_eq!(v.read_file(copy.id).unwrap(),b"data");
        v.rename(copy.id,"Renamed.txt").unwrap();
        v.remove(id).unwrap();
        let moved=v.find_child(b,"renamed.txt").unwrap().unwrap();
        v.update_file(moved.id,b"edited").unwrap();
        assert!(v.list(a).unwrap().is_empty());
        drop(v);
        let mut v=Vault::unlock(&path,"correct secret".into()).unwrap();
        assert_eq!(v.read_file(moved.id).unwrap(),b"edited");
        v.remove_many(&[b]).unwrap();
        assert_eq!(v.list(1).unwrap().len(),1);
        drop(v);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn streamed_24mib_rollback_and_export() {
        let root=fixture("stream");let path=root.join("stream.ndr");
        let mut v=Vault::create(&path,"correct secret".into()).unwrap();
        let id=v.begin_stream_file(1,"test.bin",false).unwrap();
        let data=vec![0x41;STREAM_CHUNK_SIZE];
        v.stream_write_chunk(id,0,&data).unwrap();
        v.rollback_stream_file().unwrap();
        assert!(v.list(1).unwrap().is_empty());
        let id=v.begin_stream_file(1,"test.bin",false).unwrap();
        for i in 0..3{v.stream_write_chunk(id,i,&data).unwrap();}
        v.commit_stream_file().unwrap();
        assert_eq!(v.stream_file_size(id).unwrap(),(3*STREAM_CHUNK_SIZE) as u64);
        assert_eq!(&v.stream_read_chunk(id,2).unwrap().unwrap()[..],&data[..]);
        assert!(v.stream_read_chunk(id,3).unwrap().is_none());
        let out=root.join("out");fs::create_dir_all(&out).unwrap();
        v.export_items(1,&["test.bin".into()],&out).unwrap();
        assert_eq!(fs::metadata(out.join("test.bin")).unwrap().len(),(3*STREAM_CHUNK_SIZE) as u64);
        drop(v);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_import_does_not_publish_partial_index() {
        let root=fixture("rollback");let path=root.join("files.ndr");
        let mut v=Vault::create(&path,"correct secret".into()).unwrap();
        assert!(v.import_one(1,&root.join("not-here.txt"),"missing.txt",false).is_err());
        assert!(v.list(1).unwrap().is_empty());
        drop(v);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corruption_is_detected() {
        let root=fixture("corruption");let path=root.join("corrupt.ndr");
        let mut v=Vault::create(&path,"correct secret".into()).unwrap();
        let id=v.add_file(1,"a.txt",b"secret").unwrap();
        let offset=v.item(id).unwrap().blocks[0].offset+RECORD_HEADER as u64;
        drop(v);
        let mut file=OpenOptions::new().read(true).write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();file.write_all(&[0xff]).unwrap();file.sync_all().unwrap();drop(file);
        let v=Vault::unlock(&path,"correct secret".into()).unwrap();
        assert!(v.verify_integrity().is_err());
        assert!(v.stream_read_chunk(id,0).is_err());
        drop(v);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn foreign_container_rejected_without_format_migration() {
        let root=fixture("legacy");let path=root.join("old.ndr");
        fs::write(&path,b"SQLCipher legacy test bytes").unwrap();
        assert!(Vault::unlock(&path,"correct secret".into()).err().unwrap().contains("legacy"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cross_container_copy_reencrypts_and_recursive_roundtrip() {
        let root=fixture("cross");
        let mut a=Vault::create(&root.join("a.ndr"),"alpha secret".into()).unwrap();
        let mut b=Vault::create(&root.join("b.ndr"),"bravo secret".into()).unwrap();
        let folder=a.add_folder(1,"Projects").unwrap();
        let payload=vec![0x71;1024*1024+17];
        a.add_file(folder,"report.bin",&payload).unwrap();
        assert_eq!(b.copy_from_vault(&mut a,1,&["Projects".into()],1).unwrap(),2);
        let dest=b.resolve_folder("Projects").unwrap();
        let id=b.find_child(dest,"report.bin").unwrap().unwrap().id;
        assert_eq!(b.read_file(id).unwrap(),payload);
        let out=root.join("export");fs::create_dir(&out).unwrap();
        b.export_items(1,&["Projects".into()],&out).unwrap();
        assert_eq!(fs::read(out.join("Projects/report.bin")).unwrap(),payload);
        b.verify_integrity().unwrap();
        drop(a);drop(b);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn substituted_block_from_another_position_fails_authentication() {
        let root=fixture("swap");let path=root.join("blocks.ndr");
        let mut vault=Vault::create(&path,"correct secret".into()).unwrap();
        let a=vault.add_file(1,"one.txt",b"123456").unwrap();
        let b=vault.add_file(1,"two.txt",b"abcdef").unwrap();
        let first=vault.item(a).unwrap().blocks[0].offset;
        let second=vault.item(b).unwrap().blocks[0].offset;
        drop(vault);
        let mut file=OpenOptions::new().read(true).write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(second)).unwrap();
        let mut ciphertext=[0u8;RECORD_HEADER+6];file.read_exact(&mut ciphertext).unwrap();
        file.seek(SeekFrom::Start(first)).unwrap();file.write_all(&ciphertext).unwrap();
        file.sync_all().unwrap();drop(file);
        let mut vault=Vault::unlock(&path,"correct secret".into()).unwrap();
        assert!(vault.read_file(a).is_err(),"GCM AAD must bind ciphertext to physical record offset");
        assert_eq!(vault.read_file(b).unwrap(),b"abcdef");
        drop(vault);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupted_latest_metadata_pointer_recovers_previous_commit() {
        let root=fixture("recovery");let path=root.join("recover.ndr");
        let mut vault=Vault::create(&path,"correct secret".into()).unwrap();
        vault.add_file(1,"older.txt",b"safe").unwrap();
        vault.add_file(1,"newer.txt",b"lost if last slot corrupt").unwrap();
        let current=vault.generation;
        drop(vault);
        let mut file=OpenOptions::new().read(true).write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(32+(current%2)*SLOT_SIZE)).unwrap();
        file.write_all(&[0u8;32]).unwrap();file.sync_all().unwrap();drop(file);
        let mut vault=Vault::unlock(&path,"correct secret".into()).unwrap();
        assert!(vault.find_child(1,"older.txt").unwrap().is_some());
        assert!(vault.find_child(1,"newer.txt").unwrap().is_none());
        drop(vault);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "Explicit single-file encrypted transfer and delete performance benchmark"]
    fn bench_single_file_copy_move_delete_64mib() {
        let root=fixture("perf");let path=root.join("bench.ndr");
        let mut vault=Vault::create(&path,"benchmark password".into()).unwrap();
        let payload=vec![0x5a;STREAM_CHUNK_SIZE];
        let start=Instant::now();
        let id=vault.begin_stream_file(1,"original.bin",false).unwrap();
        for ordinal in 0..8{vault.stream_write_chunk(id,ordinal,&payload).unwrap();}
        vault.commit_stream_file().unwrap();
        let encrypted_import=start.elapsed();
        let start=Instant::now();
        vault.transfer_one(1,"original.bin",1,"copy.bin",false,false).unwrap();
        let same_container_copy=start.elapsed();
        let start=Instant::now();
        vault.remove_many(&[id]).unwrap();
        let delete_64mib=start.elapsed();
        let remaining=vault.find_child(1,"copy.bin").unwrap().unwrap();
        assert_eq!(vault.stream_file_size(remaining.id).unwrap(),64*1024*1024);
        eprintln!("NDR_V2_PERF import64mib={encrypted_import:?}, in_container_copy64mib={same_container_copy:?}, delete64mib={delete_64mib:?}");
        drop(vault);fs::remove_dir_all(root).unwrap();
    }
}
