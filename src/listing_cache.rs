//! Bounded, read-only directory snapshot reuse on local Windows volumes.
//! Never used to authorize an operation: writers still validate each path.
use crate::{fsops::{self,Entry}, native};
use std::{collections::{HashMap,HashSet}, ffi::OsString, fs, os::windows::{ffi::OsStringExt, fs::{MetadataExt,OpenOptionsExt}, io::AsRawHandle}, path::{Component,Path,PathBuf,Prefix}, ptr, sync::{Mutex,atomic::{AtomicBool,AtomicU64,Ordering}}, time::{Duration,Instant}};
use windows_sys::Win32::{Foundation::{CloseHandle,GetLastError,HANDLE,ERROR_IO_INCOMPLETE,ERROR_IO_PENDING}, Storage::FileSystem::*, System::{IO::{CancelIoEx,GetOverlappedResult,OVERLAPPED}, Threading::{CreateEventW,ResetEvent}}};

const CACHE_BYTES: usize=32*1024*1024;
const MAX_SNAPSHOTS:usize=4;
const MAX_ROWS:usize=100_000;
const MAX_CHANGED:usize=128;
const MAX_AGE:Duration=Duration::from_secs(30);
const NOTIFY_FILTER:u32=FILE_NOTIFY_CHANGE_FILE_NAME|FILE_NOTIFY_CHANGE_DIR_NAME|
 FILE_NOTIFY_CHANGE_ATTRIBUTES|FILE_NOTIFY_CHANGE_SIZE|FILE_NOTIFY_CHANGE_LAST_WRITE|
 FILE_NOTIFY_CHANGE_CREATION|FILE_NOTIFY_CHANGE_SECURITY;

#[derive(Debug,PartialEq,Eq)]
enum Changes { Modified(HashSet<OsString>), Full }

// FILE_NOTIFY_INFORMATION uses a 12-byte header followed by a UTF-16 name.
// A malformed record, overflow, rename or add/remove always requests a
// full scan. Even a valid short-name alias must match an existing raw row.
fn parse_changes(bytes:&[u8])->Changes {
    if bytes.is_empty(){return Changes::Full}
    let mut offset=0usize;
    let mut names=HashSet::new();
    let word=|slice:&[u8]|u32::from_le_bytes(slice.try_into().unwrap());
    loop {
        let Some(h)=bytes.get(offset..offset.saturating_add(12)) else{return Changes::Full};
        let next=word(&h[..4]) as usize;
        let action=word(&h[4..8]);
        let len=word(&h[8..12]) as usize;
        if action!=FILE_ACTION_MODIFIED || len==0 || len%2!=0{return Changes::Full}
        let Some(raw)=bytes.get(offset+12..(offset+12).saturating_add(len)) else{return Changes::Full};
        let name=OsString::from_wide(&raw.chunks_exact(2).map(|v|u16::from_le_bytes([v[0],v[1]])).collect::<Vec<_>>());
        let mut components=Path::new(&name).components();
        if !matches!(components.next(),Some(Component::Normal(n)) if n==name.as_os_str())
            || components.next().is_some() || raw.chunks_exact(2).any(|v|v==[0,0]||v==[58,0]) {
            return Changes::Full
        }
        names.insert(name);
        if names.len()>MAX_CHANGED{return Changes::Full}
        if next==0 {return Changes::Modified(names)}
        if next<12+len || next%4!=0 {return Changes::Full}
        let Some(pos)=offset.checked_add(next) else{return Changes::Full};
        if pos>=bytes.len(){return Changes::Full}
        offset=pos;
    }
}

struct ChangeWatch {
    file:fs::File,
    event:HANDLE,
    overlapped:Box<OVERLAPPED>,
    buffer:Box<[u32;4096]>,
    pending:bool,
}
// The kernel references the boxed allocations, whose addresses remain
// stable when the exclusively owned watch moves between listing workers.
// The raw event belongs to this watch. Access is serialized, and Drop
// waits for cancellation completion before either allocation is freed.
unsafe impl Send for ChangeWatch {}
impl ChangeWatch {
    fn open(path:&Path)->Option<Self>{
        let file=fs::OpenOptions::new().access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT|FILE_FLAG_OVERLAPPED)
            .open(path).ok()?;
        let event=unsafe{CreateEventW(ptr::null(),1,0,ptr::null())};
        if event.is_null(){return None}
        let mut w=Self{file,event,overlapped:Box::new(unsafe{std::mem::zeroed()}),buffer:Box::new([0;4096]),pending:false};
        w.arm().then_some(w)
    }
    fn arm(&mut self)->bool{
        debug_assert!(!self.pending);
        unsafe{
            ResetEvent(self.event);
            *self.overlapped=std::mem::zeroed();
            self.overlapped.hEvent=self.event;
            let result=ReadDirectoryChangesW(self.file.as_raw_handle(),self.buffer.as_mut_ptr().cast(),
                16384,0,NOTIFY_FILTER,ptr::null_mut(),&mut *self.overlapped,None);
            self.pending=result!=0 || GetLastError()==ERROR_IO_PENDING;
        }
        self.pending
    }
    // Ok(None): still armed, no completed batch. Never read buffer while
    // the kernel owns it. After taking a completed batch, re-arm before
    // doing any directory metadata reads.
    fn poll(&mut self)->Result<Option<Changes>,()>{
        if !self.pending{return Err(())}
        let mut bytes=0;
        let ok=unsafe{GetOverlappedResult(self.file.as_raw_handle(),&*self.overlapped,&mut bytes,0)};
        if ok==0{
            if unsafe{GetLastError()}==ERROR_IO_INCOMPLETE{return Ok(None)}
            self.pending=false;return Err(())
        }
        self.pending=false;
        if bytes as usize>16384{return Err(())}
        let data=unsafe{std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(),bytes as usize)};
        let changed=parse_changes(data);
        if !self.arm(){return Err(())}
        Ok(Some(changed))
    }
    fn clean(&mut self)->bool{matches!(self.poll(),Ok(None))}
}
impl Drop for ChangeWatch{
    fn drop(&mut self){
        unsafe{
            if self.pending{
                CancelIoEx(self.file.as_raw_handle(),&*self.overlapped);
                let mut transferred=0;
                GetOverlappedResult(self.file.as_raw_handle(),&*self.overlapped,&mut transferred,1);
            }
            CloseHandle(self.event);
        }
    }
}

struct Snapshot {
    entries:Vec<Entry>,
    watch:ChangeWatch,
    identity:(u32,u64,u64),
    verified:Instant,
    created:Instant,
    bytes:usize,
}
#[derive(Default)]
struct Store{items:HashMap<(PathBuf,bool),Snapshot>,bytes:usize}
#[derive(Default)]
pub struct ListingCache {
    store:Mutex<Store>,
    hits:AtomicU64,
    deltas:AtomicU64,
    changed:AtomicU64,
    full:AtomicU64,
}
pub fn supported_local_path(path:&Path)->bool{
    let mut components=path.components();
    let Some(Component::Prefix(prefix))=components.next() else{return false};
    let letter=match prefix.kind(){Prefix::Disk(d)|Prefix::VerbatimDisk(d)=>d,_=>return false};
    if !path.is_absolute(){return false}
    let root=[u16::from(letter),58,92,0];
    unsafe{GetDriveTypeW(root.as_ptr())==3}
}
fn weight(entries:&Vec<Entry>)->usize{
    // Account for the vector, its two heap strings per entry, allocator
    // overhead, and the fixed native notification buffer.
    entries.iter().fold(32768usize.saturating_add(entries.capacity().saturating_mul(std::mem::size_of::<Entry>())),|sum,e|
        sum.saturating_add(e.path.capacity()+e.name.capacity()+32))
}
fn may_list(path:&Path)->bool{
    // Old notification handles cannot authorize access after ACL changes.
    fs::OpenOptions::new().access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path).is_ok()
}
fn update_modified(path:&Path, entries:&mut [Entry],names:&HashSet<OsString>,show_hidden:bool,cancel:&AtomicBool)->Result<usize,()>{
    let mut found=HashSet::new();
    for (i,entry) in entries.iter_mut().enumerate(){
        if i%256==0 && cancel.load(Ordering::Relaxed){return Err(())}
        let Some(name)=entry.path.file_name() else{return Err(())};
        if !names.contains(name){continue}
        if entry.path.parent()!=Some(path)||!found.insert(name.to_os_string()){return Err(())}
        // Directory/reparse transitions, visibility changes, and ambiguous
        // aliases are deliberately not inferred. A normal relist handles them.
        if entry.is_dir || entry.attributes&0x400!=0{return Err(())}
        let m=fs::symlink_metadata(&entry.path).map_err(|_|())?;
        let attr=m.file_attributes();
        if !m.is_file()||fsops::is_reparse(&m)||(!show_hidden && attr&6!=0){return Err(())}
        entry.size=m.len();entry.modified=m.modified().ok();entry.hidden=attr&2!=0;entry.attributes=attr;
    }
    if found.len()!=names.len(){return Err(())}
    Ok(found.len())
}
impl ListingCache {
    fn take(&self,key:&(PathBuf,bool))->Option<Snapshot>{
        let mut store=self.store.lock().ok()?;
        let snapshot=store.items.remove(key)?;
        store.bytes=store.bytes.saturating_sub(snapshot.bytes);
        Some(snapshot)
    }
    fn put(&self,key:(PathBuf,bool),snapshot:Snapshot){
        if snapshot.bytes>CACHE_BYTES{return}
        let mut retired=Vec::new();
        if let Ok(mut store)=self.store.lock(){
            if store.items.get(&key).is_some_and(|s|s.created>snapshot.created){return}
            if let Some(old)=store.items.remove(&key){store.bytes=store.bytes.saturating_sub(old.bytes);retired.push(old)}
            while store.items.len()>=MAX_SNAPSHOTS || store.bytes.saturating_add(snapshot.bytes)>CACHE_BYTES{
                let Some(old)=store.items.iter().min_by_key(|(_,s)|s.verified).map(|(k,_)|k.clone())else{break};
                if let Some(old)=store.items.remove(&old){store.bytes=store.bytes.saturating_sub(old.bytes);retired.push(old)}
            }
            store.bytes+=snapshot.bytes;
            store.items.insert(key,snapshot);
        }
        // Cancel retired I/O outside the shared lock.
        drop(retired);
    }
    pub fn read(&self,path:&Path,show_hidden:bool,allow_reuse:bool,enabled:bool,cancel:&AtomicBool)->Result<Vec<Entry>,String>{
        fsops::check_cancel(cancel)?;
        if !enabled||!supported_local_path(path){return fsops::list_dir_with_cancel(path,show_hidden,Some(cancel))}
        fsops::ensure_plain_path(path)?;
        let key=(path.to_owned(),show_hidden);
        let old=self.take(&key);
        if allow_reuse{
            if let Some(mut old)=old{
                if old.created.elapsed()<MAX_AGE && may_list(path) && native::folder_size_identity(path)==Some(old.identity){
                    // Writes often produce multiple separate notifications
                    // (size and last-write time). Drain a bounded number of
                    // batches and reconcile again, rather than mistaking the
                    // second notification for a journal overflow.
                    let mut changed_names=HashSet::new();
                    let mut clean=false;
                    for _ in 0..8 {
                        match old.watch.poll(){
                            Ok(None)=>{clean=true;break;}
                            Ok(Some(Changes::Modified(names)))=>{
                                if update_modified(path,&mut old.entries,&names,show_hidden,cancel).is_err(){break;}
                                changed_names.extend(names);
                                if changed_names.len()>MAX_CHANGED{break;}
                            }
                            _=>break,
                        }
                    }
                    if clean && !cancel.load(Ordering::Relaxed) &&
                        fsops::ensure_plain_path(path).is_ok() && native::folder_size_identity(path)==Some(old.identity){
                        let entries=old.entries.clone();
                        old.verified=Instant::now();
                        self.put(key,old);
                        if changed_names.is_empty(){self.hits.fetch_add(1,Ordering::Relaxed);}
                        else{self.deltas.fetch_add(1,Ordering::Relaxed);self.changed.fetch_add(changed_names.len() as u64,Ordering::Relaxed);}
                        return Ok(entries)
                    }
                }
            }
        }else{drop(old)}
        fsops::check_cancel(cancel)?;
        let created=Instant::now();
        let identity=native::folder_size_identity(path);
        let watch=identity.and_then(|_|ChangeWatch::open(path));
        let entries=fsops::list_dir_with_cancel(path,show_hidden,Some(cancel))?;
        self.full.fetch_add(1,Ordering::Relaxed);
        if entries.len()<=MAX_ROWS{
            let bytes=weight(&entries);
            if let (Some(mut watch),Some(identity))=(watch,identity){
                if bytes<=CACHE_BYTES && !cancel.load(Ordering::Relaxed) && watch.clean() &&
                    fsops::ensure_plain_path(path).is_ok() && native::folder_size_identity(path)==Some(identity){
                    self.put(key,Snapshot{entries:entries.clone(),watch,identity,verified:Instant::now(),created,bytes});
                }
            }
        }
        Ok(entries)
    }
    pub fn report(&self)->serde_json::Value{
        let store=self.store.lock().unwrap_or_else(|e|e.into_inner());
        serde_json::json!({"hits":self.hits.load(Ordering::Relaxed),"deltas":self.deltas.load(Ordering::Relaxed),
            "changed":self.changed.load(Ordering::Relaxed),"full":self.full.load(Ordering::Relaxed),
            "entries":store.items.len(),"estimatedBytes":store.bytes})
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    fn record(action:u32,name:&str)->Vec<u8>{
        let wide:Vec<u8>=name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        [0u32.to_le_bytes().as_slice(),action.to_le_bytes().as_slice(),(wide.len() as u32).to_le_bytes().as_slice(),&wide].concat()
    }
    #[test]
    fn parser_rejects_overflow_invalid_paths_and_rename(){
        assert_eq!(parse_changes(&[]),Changes::Full);
        assert_eq!(parse_changes(&record(FILE_ACTION_RENAMED_OLD_NAME,"a.txt")),Changes::Full);
        for name in ["", "..", r"dir\child",r"C:\a","a:stream","a\0b"]{assert_eq!(parse_changes(&record(FILE_ACTION_MODIFIED,name)),Changes::Full,"{name}")}
        let b=record(FILE_ACTION_MODIFIED,"\u{d55c}\u{ae00}.txt");
        assert!(matches!(parse_changes(&b),Changes::Modified(n) if n.contains(&OsString::from("\u{d55c}\u{ae00}.txt"))));
        for n in 0..b.len(){assert_eq!(parse_changes(&b[..n]),Changes::Full)}
    }
    #[test]
    fn actual_directory_cache_reuses_and_updates_only_modified_file(){
        let root=std::env::temp_dir().join(format!("ndr-list-cache-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&root).unwrap();
        let a=root.join("a.txt");let b=root.join("b.txt");
        fs::write(&a,b"a").unwrap();fs::write(&b,b"bb").unwrap();
        let cache=ListingCache::default();let cancel=AtomicBool::new(false);
        assert_eq!(cache.read(&root,false,false,true,&cancel).unwrap().len(),2);
        if !supported_local_path(&root)||cache.report()["entries"]!=1{
            println!("SKIP unsupported directory notification environment");fs::remove_dir_all(&root).unwrap();return
        }
        assert_eq!(cache.read(&root,false,true,true,&cancel).unwrap().len(),2);
        assert_eq!(cache.report()["hits"],1);
        fs::write(&a,b"updated").unwrap();
        std::thread::sleep(Duration::from_millis(180));
        let entries=cache.read(&root,false,true,true,&cancel).unwrap();
        assert_eq!(entries.iter().find(|e|e.path==a).unwrap().size,7);
        assert_eq!(cache.report()["deltas"],1,"{}",cache.report());
        assert_eq!(cache.report()["changed"],1);
        fs::rename(&a,root.join("c.txt")).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        let entries=cache.read(&root,false,true,true,&cancel).unwrap();
        assert!(entries.iter().any(|e|e.name=="c.txt")&&!entries.iter().any(|e|e.name=="a.txt"));
        assert_eq!(cache.report()["full"],2,"{}",cache.report());
        drop(cache);fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replaced_directory_is_never_replayed_and_cache_is_bounded(){
        let root=std::env::temp_dir().join(format!("ndr-cache-replace-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&root).unwrap();
        let dir=root.join("current");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("old.txt"),b"old").unwrap();
        let cache=ListingCache::default();
        let cancel=AtomicBool::new(false);
        cache.read(&dir,false,false,true,&cancel).unwrap();
        fs::rename(&dir,root.join("previous")).unwrap();
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("new.txt"),b"new").unwrap();
        let rows=cache.read(&dir,false,true,true,&cancel).unwrap();
        assert_eq!(rows.len(),1);
        assert_eq!(rows[0].name,"new.txt");
        for i in 0..8 {
            let dir=root.join(format!("dir-{i}"));
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("file.txt"),b"fixture").unwrap();
            cache.read(&dir,false,false,true,&cancel).unwrap();
        }
        let report=cache.report();
        assert!(report["entries"].as_u64().unwrap()<=MAX_SNAPSHOTS as u64);
        assert!(report["estimatedBytes"].as_u64().unwrap()<=CACHE_BYTES as u64);
        drop(cache);fs::remove_dir_all(&root).unwrap();
    }
}
