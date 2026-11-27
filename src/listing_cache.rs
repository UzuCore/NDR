//! Bounded, read-only directory snapshot reuse on local Windows volumes.
//! Never used to authorize an operation: writers still validate each path.
use crate::{fsops::{self,Entry}, native};
use std::{collections::{HashMap,HashSet}, ffi::OsString, fs, os::windows::{ffi::{OsStrExt,OsStringExt}, fs::{MetadataExt,OpenOptionsExt}, io::AsRawHandle}, path::{Component,Path,PathBuf,Prefix}, ptr, sync::{Arc,Condvar,Mutex,OnceLock,atomic::{AtomicBool,AtomicU64,Ordering}}, time::{Duration,Instant}};
use windows_sys::Win32::{Foundation::{CloseHandle,GetLastError,HANDLE,ERROR_IO_INCOMPLETE,ERROR_IO_PENDING,ERROR_NO_MORE_FILES}, Storage::FileSystem::*, System::{IO::{CancelIoEx,GetOverlappedResult,OVERLAPPED}, Threading::{CreateEventW,ResetEvent}}};

const CACHE_BYTES: usize=16*1024*1024;
const MAX_SNAPSHOTS:usize=4;
const MAX_ROWS:usize=100_000;
const MAX_CHANGED:usize=128;
const MAX_AGE:Duration=Duration::from_secs(30);
// Bounded discovery of hard links created through another directory after
// the original snapshot. Large lists retain the existing known-link checks
// and explicit fresh reads rather than repeatedly stat every file.
pub const ALIAS_AUDIT_ROWS:usize=2048;
pub const ALIAS_FULL_AUDIT_ROWS:usize=MAX_ROWS;
const ALIAS_AUDIT_AGE:Duration=Duration::from_secs(10);
const NOTIFY_FILTER:u32=FILE_NOTIFY_CHANGE_FILE_NAME|FILE_NOTIFY_CHANGE_DIR_NAME|
 FILE_NOTIFY_CHANGE_ATTRIBUTES|FILE_NOTIFY_CHANGE_SIZE|FILE_NOTIFY_CHANGE_LAST_WRITE|
 FILE_NOTIFY_CHANGE_CREATION|FILE_NOTIFY_CHANGE_SECURITY;

#[derive(Debug,PartialEq,Eq)]
enum Changes { Modified(HashSet<OsString>), Structural(Vec<(u32,OsString)>), Full }

// FILE_NOTIFY_INFORMATION uses a 12-byte header followed by a UTF-16 name.
// A malformed record, overflow or incomplete rename requests a full scan.
// Ordinary bounded name changes are reconciled against the exact snapshot.
fn parse_changes(bytes:&[u8])->Changes {
    if bytes.is_empty(){return Changes::Full}
    let mut offset=0usize;
    let mut names=HashSet::new();let mut events=Vec::new();let mut structural=false;
    let word=|slice:&[u8]|u32::from_le_bytes(slice.try_into().unwrap());
    loop {
        let Some(h)=bytes.get(offset..offset.saturating_add(12)) else{return Changes::Full};
        let next=word(&h[..4]) as usize;
        let action=word(&h[4..8]);
        let len=word(&h[8..12]) as usize;
        if !matches!(action,FILE_ACTION_ADDED|FILE_ACTION_REMOVED|FILE_ACTION_MODIFIED|FILE_ACTION_RENAMED_OLD_NAME|FILE_ACTION_RENAMED_NEW_NAME) || len==0 || len%2!=0{return Changes::Full}
        let Some(raw)=bytes.get(offset+12..(offset+12).saturating_add(len)) else{return Changes::Full};
        let name=OsString::from_wide(&raw.chunks_exact(2).map(|v|u16::from_le_bytes([v[0],v[1]])).collect::<Vec<_>>());
        let mut components=Path::new(&name).components();
        if !matches!(components.next(),Some(Component::Normal(n)) if n==name.as_os_str())
            || components.next().is_some() || raw.chunks_exact(2).any(|v|matches!(v,[0,0]|[58,0]|[42,0]|[63,0])) {
            return Changes::Full
        }
        structural|=action!=FILE_ACTION_MODIFIED;
        names.insert(name.clone());events.push((action,name));
        if names.len()>MAX_CHANGED||events.len()>512{return Changes::Full}
        if next==0 {
            if !structural{return Changes::Modified(names)}
            let mut rename_pending=false;
            for (action,_) in &events{
                match (*action,rename_pending){
                    (FILE_ACTION_RENAMED_OLD_NAME,false)=>rename_pending=true,
                    (FILE_ACTION_RENAMED_NEW_NAME,true)=>rename_pending=false,
                    (FILE_ACTION_RENAMED_NEW_NAME,false)|(_,true)=>return Changes::Full,
                    _=>{}
                }
            }
            return if rename_pending{Changes::Full}else{Changes::Structural(events)}
        }
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

// Store the native filename once, without repeating the directory path or
// keeping a second lossy display name. Reconstruct the exact original path
// from the native OsString only when a caller actually needs a full list.
#[derive(Debug)]
struct CachedEntry {
    name:OsString,
    size:u64,
    modified:Option<std::time::SystemTime>,
    attributes:u32,
    is_dir:bool,
}
impl CachedEntry {
    fn from_entry(entry:&Entry)->Option<Self>{
        Some(Self{name:entry.path.file_name()?.to_os_string(),size:entry.size,
            modified:entry.modified,attributes:entry.attributes,is_dir:entry.is_dir})
    }
    fn materialize(&self,path:&Path)->Entry{
        Entry{path:fsops::listing_child_path(path,&self.name),name:self.name.to_string_lossy().into_owned(),
            size:self.size,modified:self.modified,attributes:self.attributes,
            is_dir:self.is_dir,hidden:self.attributes&2!=0,drive:false}
    }
}
struct Snapshot {
    entries:Vec<CachedEntry>,
    watch:ChangeWatch,
    identity:(u32,u64,u64),
    verified:Instant,
    created:Instant,
    bytes:usize,
    revision:u64,
    linked:HashSet<OsString>,
    alias_audited:Instant,
    order_dirty:bool,
    last_delta:Option<LastDelta>,
}
struct LastDelta{
    base:u64,
    changed:HashSet<OsString>,
    removed:Vec<OsString>,
    structural:bool,
}
impl LastDelta{
    fn bytes(&self)->usize{
        link_weight(&self.changed).saturating_add(self.removed.capacity()*std::mem::size_of::<OsString>())
            .saturating_add(self.removed.iter().map(|n|n.capacity()).sum::<usize>())
    }
}
type CacheKey=(PathBuf,bool);
#[derive(Default)]
struct Store{items:HashMap<CacheKey,Snapshot>,bytes:usize,leased:usize,active:HashSet<CacheKey>,epoch:u64,peak:usize}
struct Lease<'a>{cache:&'a ListingCache,key:CacheKey,epoch:u64,bytes:usize}
impl Drop for Lease<'_>{
    fn drop(&mut self){
        if let Ok(mut store)=self.cache.store.lock(){store.active.remove(&self.key);store.leased=store.leased.saturating_sub(self.bytes);}
        self.cache.ready.notify_all();
    }
}
#[derive(Default)]
pub struct ListingCache {
    store:Mutex<Store>,
    ready:Condvar,
    coalesced:AtomicU64,
    hits:AtomicU64,
    deltas:AtomicU64,
    changed:AtomicU64,
    full:AtomicU64,
    serial:AtomicU64,
    partial:AtomicU64,
    alias_rows:AtomicU64,
    unverified:AtomicU64,
    structural:AtomicU64,
    replays:AtomicU64,
    alias_audits:AtomicU64,
    enumerate_us:AtomicU64,
    compact_us:AtomicU64,
    reuse_us:AtomicU64,
}
pub struct ListingRead{
    pub entries:Vec<Entry>,
    pub revision:Option<u64>,
    /// Entries contain ONLY notified files when the caller supplied the
    /// precise revision of its currently displayed list.
    pub partial:bool,
    pub removed:Vec<OsString>,
    pub hardlinks:bool,
    pub structural:bool,
}
impl ListingRead{
    fn full(entries:Vec<Entry>)->Self{Self{entries,revision:None,partial:false,removed:Vec::new(),hardlinks:false,structural:false}}
}
pub fn supported_local_path(path:&Path)->bool{
    let mut components=path.components();
    let Some(Component::Prefix(prefix))=components.next() else{return false};
    let letter=match prefix.kind(){Prefix::Disk(d)|Prefix::VerbatimDisk(d)=>d,_=>return false};
    if !path.is_absolute(){return false}
    let root=[u16::from(letter),58,92,0];
    unsafe{GetDriveTypeW(root.as_ptr())==3}
}
// NTFS directory indexes store filename metadata for every hard link.
// A write through one link can leave another filename's size/time stale.
// Enumerate IDs in the same pass, then open only duplicated IDs to obtain
// authoritative metadata. No write permissions, path mutations, or per-file
// handles for the normal (unique-ID) listing. Unsupported filesystems keep
// the existing enumeration path. IDs only compare within this directory.
fn windows_ticks(ticks:i64)->Option<std::time::SystemTime>{
    let unix=116_444_736_000_000_000i64;
    let diff=ticks.checked_sub(unix)?;
    let magnitude=diff.unsigned_abs();
    let d=Duration::new(magnitude/10_000_000,((magnitude%10_000_000)*100) as u32);
    if diff>=0{std::time::UNIX_EPOCH.checked_add(d)}else{std::time::UNIX_EPOCH.checked_sub(d)}
}
#[repr(C)]
#[derive(Default)]
struct ByNameStat {
    id:i64, creation:i64, access:i64, write:i64, change:i64, allocation:i64, size:i64,
    attributes:u32, tag:u32, links:u32, effective_access:u32,
}
type ByNameFn=unsafe extern "system" fn(*const u16,i32,*mut ByNameStat,u32)->i32;
fn by_name_fn()->Option<ByNameFn>{
    use windows::{core::{w,s},Win32::System::LibraryLoader::{GetModuleHandleW,GetProcAddress}};
    static API:OnceLock<Option<ByNameFn>>=OnceLock::new();
    *API.get_or_init(||unsafe{
        let module=GetModuleHandleW(w!("kernelbase.dll")).ok()?;
        let f=GetProcAddress(module,s!("GetFileInformationByName"))?;
        Some(std::mem::transmute::<unsafe extern "system" fn()->isize,ByNameFn>(f))
    })
}
struct FreshFile{id:u64,size:u64,modified:Option<std::time::SystemTime>,attributes:u32,links:u32}
fn regular_by_name_wide(wide:&[u16])->Option<ByNameStat>{
    let f=by_name_fn()?;
    if wide.last()!=Some(&0){return None}
    let mut stat=ByNameStat::default();
    (unsafe{f(wide.as_ptr(),0,&mut stat,std::mem::size_of::<ByNameStat>() as u32)}!=0
        &&stat.size>=0&&stat.links>0&&stat.attributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)==0)
        .then_some(stat)
}
fn regular_by_name(path:&Path,wide:&mut Vec<u16>)->Option<ByNameStat>{
    if by_name_fn().is_none(){return None}
    wide.clear();wide.extend(path.as_os_str().encode_wide());wide.push(0);
    regular_by_name_wide(wide)
}
/// A recursive walk enumerates siblings under one directory prefix.
/// Avoid converting or allocating that prefix for each file. Exact
/// native size and link-count verification are still mandatory.
pub(crate) struct FolderPathProbe{
    wide:Vec<u16>,
    prefix:usize,
}
impl FolderPathProbe{
    pub(crate) fn new(parent:&Path)->Option<Self>{
        by_name_fn()?;
        // Limit the fast path to ordinary absolute drive paths. Other
        // namespace forms use the previous full-path/handle route.
        let mut components=parent.components();
        if !matches!(components.next(),Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::Disk(_)|Prefix::VerbatimDisk(_)))
            ||!parent.is_absolute(){return None}
        let mut wide:Vec<u16>=parent.as_os_str().encode_wide().collect();
        if !matches!(wide.last(),Some(92|47)){wide.push(92)}
        let prefix=wide.len();
        Some(Self{wide,prefix})
    }
    fn stat_raw(&mut self,name:&std::ffi::OsStr)->Option<ByNameStat>{
        self.wide.truncate(self.prefix);
        self.wide.extend(name.encode_wide());self.wide.push(0);
        regular_by_name_wide(&self.wide)
    }
    pub(crate) fn stat(&mut self,name:&std::ffi::OsStr)->Option<(u64,u64,u32)>{
        let stat=self.stat_raw(name)?;
        Some((stat.id as u64,stat.size as u64,stat.links))
    }
    fn fresh(&mut self,name:&std::ffi::OsStr)->Option<FreshFile>{
        self.stat_raw(name).map(fresh_from_stat)
    }
}
fn fresh_from_stat(stat:ByNameStat)->FreshFile{
    FreshFile{id:stat.id as u64,size:stat.size as u64,modified:windows_ticks(stat.write),
        attributes:stat.attributes,links:stat.links}
}
fn fresh_regular(path:&Path,wide:&mut Vec<u16>)->Result<FreshFile,std::io::Error>{
    if let Some(stat)=regular_by_name(path,wide){return Ok(fresh_from_stat(stat))}
    fresh_regular_handle(path)
}
fn fresh_regular_handle(path:&Path)->Result<FreshFile,std::io::Error>{
    // Older Windows versions and unsupported requests retain the normal
    // read-attributes handle path; never open a reparse target.
    let file=fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
    let mut info:BY_HANDLE_FILE_INFORMATION=unsafe{std::mem::zeroed()};
    if unsafe{GetFileInformationByHandle(file.as_raw_handle(),&mut info)}==0{return Err(std::io::Error::last_os_error())}
    if info.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)!=0||info.nNumberOfLinks==0{
        return Err(std::io::Error::other("Not a verified regular file"))
    }
    Ok(FreshFile{id:(u64::from(info.nFileIndexHigh)<<32)|u64::from(info.nFileIndexLow),
        size:(u64::from(info.nFileSizeHigh)<<32)|u64::from(info.nFileSizeLow),
        modified:windows_ticks(((u64::from(info.ftLastWriteTime.dwHighDateTime)<<32)|u64::from(info.ftLastWriteTime.dwLowDateTime)) as i64),
        attributes:info.dwFileAttributes,links:info.nNumberOfLinks})
}
/// The NTFS filename index can retain a stale size for a hard-link alias.
pub(crate) fn folder_file_stat(path:&Path,wide:&mut Vec<u16>)->Result<(u64,u64,u32),std::io::Error>{
    // Recursive size accounting does not need FILETIME conversion or
    // presentation fields. Preserve the authoritative handle fallback.
    if let Some(stat)=regular_by_name(path,wide){
        return Ok((stat.id as u64,stat.size as u64,stat.links))
    }
    let fresh=fresh_regular_handle(path)?;
    Ok((fresh.id,fresh.size,fresh.links))
}
fn apply_fresh(entry:&mut Entry,fresh:&FreshFile){
    entry.size=fresh.size;entry.modified=fresh.modified;
    entry.attributes=fresh.attributes;entry.hidden=fresh.attributes&FILE_ATTRIBUTE_HIDDEN!=0;
}
struct Scan{entries:Vec<Entry>,linked:HashSet<OsString>,unverified:usize}
// The directory index cannot identify links in a different directory.
// Verify regular rows without retaining a file handle per row. Keep
// inaccessible/racing rows visible as before but do not cache them.
fn verify_chunk(parent:&Path,rows:&mut [Entry],ids:&[u64],cancel:&AtomicBool)->Result<(HashSet<OsString>,usize),String>{
    let mut links=HashSet::new();let mut unverified=0;let mut wide=Vec::with_capacity(512);
    let mut probe=FolderPathProbe::new(parent);
    for (row,&id) in rows.iter_mut().zip(ids){
        fsops::check_cancel(cancel)?;
        if row.is_dir||row.attributes&FILE_ATTRIBUTE_REPARSE_POINT!=0{continue}
        let fresh=if let Some(probe)=probe.as_mut(){
            row.path.file_name().and_then(|name|probe.fresh(name)).map_or_else(||fresh_regular_handle(&row.path),Ok)
        }else{fresh_regular(&row.path,&mut wide)};
        match fresh{
            Ok(fresh) if id==0||fresh.id==id=>{
                apply_fresh(row,&fresh);
                if fresh.links>1{if let Some(name)=row.path.file_name(){links.insert(name.to_os_string());}}
            }
            _=>unverified+=1,
        }
    }
    Ok((links,unverified))
}
fn verify_regulars(path:&Path,entries:&mut [Entry],ids:&[u64],cancel:&AtomicBool)->Result<(HashSet<OsString>,usize),String>{
    static BUDGET:crate::listing_budget::VerificationBudget=crate::listing_budget::VerificationBudget::new();
    // Keep exact native ID/link-count verification for every regular file.
    // On fast media the measured workload benefits from additional workers;
    // unknown/seek media retain conservative limits. The caller participates.
    let requested=if entries.len()>=4096&&by_name_fn().is_some(){
        match native::folder_size_media(path){
            native::FolderSizeMedia::Seek=>1,
            native::FolderSizeMedia::Fast if entries.len()>=16_384=>12,
            native::FolderSizeMedia::Fast=>8,
            _=>4,
        }
    }else{1};
    let parallelism=std::thread::available_parallelism().map_or(1,usize::from);
    let permit=BUDGET.acquire(requested,parallelism);
    let workers=permit.workers();
    if workers<=1{return verify_chunk(path,entries,ids,cancel)}
    let chunk=entries.len().div_ceil(workers).max(1);
    std::thread::scope(|scope|{
        let mut chunks=entries.chunks_mut(chunk).zip(ids.chunks(chunk));
        let (local_rows,local_ids)=chunks.next().expect("parallel verification has rows");
        let tasks:Vec<_>=chunks.map(|(rows,ids)|scope.spawn(move||verify_chunk(path,rows,ids,cancel))).collect();
        let local=verify_chunk(path,local_rows,local_ids,cancel);
        // Join every worker even when the caller or another worker failed.
        let joined:Vec<_>=tasks.into_iter().map(|t|t.join()).collect();
        let (mut linked,mut unverified)=local?;
        for result in joined{
            let (names,count)=result.map_err(|_|"File metadata worker failed".to_owned())??;
            linked.extend(names);unverified+=count;
        }
        Ok((linked,unverified))
    })
}
fn scan_ids(path:&Path,show_hidden:bool,cancel:&AtomicBool)->Result<Option<Scan>,String>{
    if !supported_local_path(path){return Ok(None)}
    fsops::check_cancel(cancel)?;
    fsops::ensure_plain_path(path)?;
    let directory=match fs::OpenOptions::new().access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT).open(path){
        Ok(file)=>file,Err(_)=>return Ok(None)
    };
    // FileIdBothDirectoryInfo is supported on NTFS. Other volume formats
    // may synthesize IDs; do not equate unrelated files on those formats.
    let mut fs_name=[0u16;32];
    if unsafe{GetVolumeInformationByHandleW(directory.as_raw_handle(),ptr::null_mut(),0,ptr::null_mut(),ptr::null_mut(),ptr::null_mut(),fs_name.as_mut_ptr(),fs_name.len() as u32)}==0
        || fs_name[..5]!=[78,84,70,83,0]{return Ok(None)}
    let mut buffer=vec![0u64;8192];
    let capacity=buffer.len()*std::mem::size_of::<u64>();
    let header=std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO,FileName);
    let mut entries=Vec::new();
    let mut ids=Vec::<u64>::new();
    loop{
        fsops::check_cancel(cancel)?;
        let ok=unsafe{GetFileInformationByHandleEx(directory.as_raw_handle(),FileIdBothDirectoryInfo,
            buffer.as_mut_ptr().cast(),capacity as u32)};
        if ok==0{
            let error=unsafe{GetLastError()};
            if error==ERROR_NO_MORE_FILES{break}
            // Do not return a partial listing on an unsupported/failed query.
            return Ok(None)
        }
        let mut offset=0usize;
        loop{
            if entries.len()%256==0{fsops::check_cancel(cancel)?;}
            if offset%8!=0||offset.saturating_add(std::mem::size_of::<FILE_ID_BOTH_DIR_INFO>())>capacity{return Ok(None)}
            let raw=unsafe{buffer.as_ptr().cast::<u8>().add(offset)};
            let info=unsafe{std::ptr::read_unaligned(raw.cast::<FILE_ID_BOTH_DIR_INFO>())};
            let length=info.FileNameLength as usize;
            if length==0||length%2!=0||offset.saturating_add(header).saturating_add(length)>capacity{return Ok(None)}
            let units=unsafe{std::slice::from_raw_parts(raw.add(header).cast::<u16>(),length/2)};
            if units!=[46] && units!=[46,46]{
                let filename=OsString::from_wide(units);
                let item_path=fsops::listing_child_path(path,&filename);
                let name=filename.into_string().unwrap_or_else(|name|name.to_string_lossy().into_owned());
                // Win32 directory attributes alone do not distinguish a
                // directory symlink from a normal directory. Keep Rust's
                // no-follow FileType semantics for uncommon reparse rows.
                let (attr,is_dir,size,modified)=if info.FileAttributes&FILE_ATTRIBUTE_REPARSE_POINT!=0{
                    let m=match fs::symlink_metadata(&item_path){Ok(m)=>m,Err(_)=>return Ok(None)};
                    (m.file_attributes(),m.is_dir(),if m.is_dir(){0}else{m.len()},m.modified().ok())
                }else{
                    let attr=info.FileAttributes;
                    let is_dir=attr&FILE_ATTRIBUTE_DIRECTORY!=0;
                    (attr,is_dir,if is_dir{0}else{info.EndOfFile.max(0) as u64},windows_ticks(info.LastWriteTime))
                };
                ids.push(info.FileId as u64);
                entries.push(Entry{path:item_path,name,is_dir,size,modified,
                    hidden:attr&FILE_ATTRIBUTE_HIDDEN!=0,attributes:attr,drive:false});
            }
            let next=info.NextEntryOffset as usize;
            if next==0{break}
            if next<header+length||next%8!=0||offset.saturating_add(next)>=capacity{return Ok(None)}
            offset+=next;
        }
    }
    let (linked,unverified)=verify_regulars(path,&mut entries,&ids,cancel)?;
    if !show_hidden{entries.retain(|e|e.attributes&(FILE_ATTRIBUTE_HIDDEN|FILE_ATTRIBUTE_SYSTEM)==0);}
    Ok(Some(Scan{entries,linked,unverified}))
}
fn list_with_alias_check(path:&Path,show_hidden:bool,cancel:&AtomicBool)->Result<Scan,String>{
    match scan_ids(path,show_hidden,cancel)?{
        Some(scan)=>Ok(scan),
        None=>fsops::list_dir_with_cancel(path,show_hidden,Some(cancel)).map(|entries|Scan{entries,linked:HashSet::new(),unverified:0})
    }
}
fn scan_read(scan:Scan)->ListingRead{
    ListingRead{hardlinks:!scan.linked.is_empty(),..ListingRead::full(scan.entries)}
}
fn weight(entries:&Vec<CachedEntry>)->usize{
    entries.iter().fold(32768usize.saturating_add(entries.capacity().saturating_mul(std::mem::size_of::<CachedEntry>())),|sum,e|
        sum.saturating_add(e.name.capacity()+32))
}
fn may_list(path:&Path)->bool{
    // Old notification handles cannot authorize access after ACL changes.
    fs::OpenOptions::new().access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path).is_ok()
}
fn update_modified(path:&Path, entries:&mut [CachedEntry],names:&HashSet<OsString>,
    linked:&mut HashSet<OsString>,show_hidden:bool,cancel:&AtomicBool)->Result<HashSet<OsString>,()>{
    let mut found=HashSet::new();let mut changed=HashSet::new();let mut wide=Vec::new();
    for (i,entry) in entries.iter_mut().enumerate(){
        if i%256==0 && cancel.load(Ordering::Relaxed){return Err(())}
        if !names.contains(&entry.name){continue}
        if !found.insert(entry.name.clone())||entry.is_dir||entry.attributes&FILE_ATTRIBUTE_REPARSE_POINT!=0{return Err(())}
        let m=fresh_regular(&path.join(&entry.name),&mut wide).map_err(|_|())?;
        if !show_hidden&&m.attributes&6!=0{return Err(())}
        if m.links>1{linked.insert(entry.name.clone());}
        else{linked.remove(&entry.name);}
        if entry.size!=m.size||entry.modified!=m.modified||entry.attributes!=m.attributes{
            entry.size=m.size;entry.modified=m.modified;entry.attributes=m.attributes;
            changed.insert(entry.name.clone());
        }
    }
    for name in names.difference(&found){
        if !linked.contains(name){return Err(())}
        let m=fresh_regular(&path.join(name),&mut wide).map_err(|_|())?;
        if show_hidden||m.attributes&6==0{return Err(())}
    }
    Ok(changed)
}
// For structural edits we must know the exact enumerated long name,
// rather than accepting case folding or a short-name alias.
fn exact_present(path:&Path,name:&OsString)->Result<bool,()>{
    let target=path.join(name);
    let mut wide:Vec<u16>=target.as_os_str().encode_wide().chain(Some(0)).collect();
    if wide.len()>=260&&!target.as_os_str().to_string_lossy().starts_with(r"\\?\"){
        let mut prefix:Vec<u16>=r"\\?\".encode_utf16().collect();prefix.append(&mut wide);wide=prefix;
    }
    let mut data:WIN32_FIND_DATAW=unsafe{std::mem::zeroed()};
    let h=unsafe{FindFirstFileW(wide.as_ptr(),&mut data)};
    if h==windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE{
        return match unsafe{GetLastError()}{2|3=>Ok(false),_=>Err(())}
    }
    unsafe{FindClose(h);}
    let len=data.cFileName.iter().position(|&u|u==0).ok_or(())?;
    if OsString::from_wide(&data.cFileName[..len])!=*name{return Err(())}
    if data.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)!=0{return Err(())}
    Ok(true)
}
fn ordinary_cached(e:&CachedEntry)->bool{
    !e.is_dir&&e.attributes&FILE_ATTRIBUTE_REPARSE_POINT==0
        &&!Path::new(&e.name).extension().is_some_and(|x|x.eq_ignore_ascii_case("ndr"))
}
// Reconcile the final state of a bounded event batch. Never mutate the
// snapshot until every touched path has been validated. Unknown case/short
// names, directory/vault/reparse changes and hidden transitions relist.
fn reconcile_structural(path:&Path,old:&mut Snapshot,events:&[(u32,OsString)],
    show_hidden:bool,cancel:&AtomicBool)->Result<(HashSet<OsString>,Vec<OsString>),()>{
    let touched:HashSet<&OsString>=events.iter().map(|(_,name)|name).collect();
    if touched.len()>MAX_CHANGED||events.len()>512{return Err(())}
    let mut states:HashMap<OsString,(bool,bool)>=touched.into_iter().map(|name|(name.clone(),(false,false))).collect();
    for (i,row) in old.entries.iter().enumerate(){
        if i%256==0&&cancel.load(Ordering::Relaxed){return Err(())}
        if let Some(state)=states.get_mut(&row.name){
            if state.0||!ordinary_cached(row)||old.linked.contains(&row.name){return Err(())}
            *state=(true,true);
        }
    }
    for (action,name) in events{
        let state=states.get_mut(name).ok_or(())?;
        match (*action,state.1){
            (FILE_ACTION_ADDED|FILE_ACTION_RENAMED_NEW_NAME,false)=>state.1=true,
            (FILE_ACTION_REMOVED|FILE_ACTION_RENAMED_OLD_NAME,true)=>state.1=false,
            (FILE_ACTION_MODIFIED,true)=>{},
            _=>return Err(()),
        }
    }
    let mut removed=Vec::new();let mut updates=HashMap::new();let mut wide=Vec::new();
    for (name,(was,exists)) in &states{
        if cancel.load(Ordering::Relaxed)||old.linked.contains(name)
            ||Path::new(name).extension().is_some_and(|x|x.eq_ignore_ascii_case("ndr")){return Err(())}
        if exact_present(path,name)?!=*exists{return Err(())}
        if *exists{
            let fresh=fresh_regular(&path.join(name),&mut wide).map_err(|_|())?;
            if fresh.links!=1||(!show_hidden&&fresh.attributes&6!=0){return Err(())}
            updates.insert(name.clone(),CachedEntry{name:name.clone(),size:fresh.size,
                modified:fresh.modified,attributes:fresh.attributes,is_dir:false});
        }else if *was{removed.push(name.clone());}
    }
    let changed:HashSet<_>=updates.keys().cloned().collect();
    let final_len=old.entries.len().checked_sub(removed.len()).and_then(|n|n.checked_add(states.values().filter(|&&(was,exists)|!was&&exists).count())).ok_or(())?;
    if final_len>MAX_ROWS{return Err(())}
    old.entries.retain(|row|!states.get(&row.name).is_some_and(|state|!state.1));
    for row in &mut old.entries{if let Some(new)=updates.remove(&row.name){*row=new;}}
    old.entries.extend(updates.into_values());
    old.order_dirty=true;
    Ok((changed,removed))
}
fn link_weight(linked:&HashSet<OsString>)->usize{
    linked.capacity().saturating_mul(std::mem::size_of::<OsString>()+16)
        .saturating_add(linked.iter().map(|n|n.capacity()).sum::<usize>())
}
impl ListingCache {
    /// Only one cache owner per path. Overlapping requests still enumerate
    /// independently, but cannot overwrite a newer snapshot or duplicate it.
    fn begin(&self,key:CacheKey)->Option<(Lease<'_>,Option<Snapshot>)>{
        self.expire();
        let mut store=self.store.lock().ok()?;
        if !store.active.insert(key.clone()){return None}
        let old=store.items.remove(&key);
        let bytes=old.as_ref().map_or(0,|s|s.bytes);
        store.bytes=store.bytes.saturating_sub(bytes);
        store.leased=store.leased.saturating_add(bytes);
        store.peak=store.peak.max(store.bytes.saturating_add(store.leased));
        Some((Lease{cache:self,key,epoch:store.epoch,bytes},old))
    }
    fn expire(&self){
        let mut retired=Vec::new();
        if let Ok(mut store)=self.store.lock(){
            let expired:Vec<_>=store.items.iter().filter(|(_,s)|s.created.elapsed()>=MAX_AGE)
                .map(|(k,_)|k.clone()).collect();
            for key in expired{
                if let Some(old)=store.items.remove(&key){
                    store.bytes=store.bytes.saturating_sub(old.bytes);retired.push(old);
                }
            }
        }
        drop(retired);
    }
    fn clear(&self){
        let retired=if let Ok(mut store)=self.store.lock(){
            store.epoch=store.epoch.wrapping_add(1);store.bytes=0;
            std::mem::take(&mut store.items)
        }else{return};
        drop(retired);
    }
    pub fn managed()->Arc<Self>{
        let cache=Arc::new(Self::default());
        if !cfg!(test){
            let weak=Arc::downgrade(&cache);
            let _=std::thread::Builder::new().name("ndr-list-cache-expiry".into()).spawn(move||loop{
                std::thread::sleep(Duration::from_secs(5));
                let Some(cache)=weak.upgrade()else{break};
                cache.expire();
            });
        }
        cache
    }
    fn put(&self,lease:&Lease<'_>,snapshot:Snapshot){
        if snapshot.bytes>CACHE_BYTES{return}
        let mut retired=Vec::new();
        if let Ok(mut store)=self.store.lock(){
            if store.epoch!=lease.epoch{return}
            let key=lease.key.clone();
            if let Some(old)=store.items.remove(&key){store.bytes=store.bytes.saturating_sub(old.bytes);retired.push(old)}
            let other_leased=store.leased.saturating_sub(lease.bytes);
            if other_leased.saturating_add(snapshot.bytes)>CACHE_BYTES{return}
            while store.items.len()>=MAX_SNAPSHOTS || store.bytes.saturating_add(other_leased).saturating_add(snapshot.bytes)>CACHE_BYTES{
                let Some(old)=store.items.iter().min_by_key(|(_,s)|s.verified).map(|(k,_)|k.clone())else{break};
                if let Some(old)=store.items.remove(&old){store.bytes=store.bytes.saturating_sub(old.bytes);retired.push(old)}
            }
            store.bytes+=snapshot.bytes;
            // The lease accounts for an old snapshot until the caller
            // exits; exclude its bytes here when it is published again.
            store.peak=store.peak.max(store.bytes.saturating_add(store.leased.saturating_sub(lease.bytes)));
            store.items.insert(key,snapshot);
        }
        // Cancel retired I/O outside the shared lock.
        drop(retired);
    }
    pub fn read(&self,path:&Path,show_hidden:bool,allow_reuse:bool,enabled:bool,cancel:&AtomicBool)->Result<Vec<Entry>,String>{
        self.read_revision(path,show_hidden,allow_reuse,enabled,cancel,None).map(|r|r.entries)
    }
    pub fn read_revision(&self,path:&Path,show_hidden:bool,allow_reuse:bool,enabled:bool,cancel:&AtomicBool,base:Option<u64>)->Result<ListingRead,String>{
        let started=Instant::now();
        fsops::check_cancel(cancel)?;
        if !enabled{
            self.clear();
            return list_with_alias_check(path,show_hidden,cancel).map(scan_read)
        }
        if !supported_local_path(path){return list_with_alias_check(path,show_hidden,cancel).map(scan_read)}
        fsops::ensure_plain_path(path)?;
        let key=(path.to_owned(),show_hidden);
        // Only reuse-eligible navigation/watch requests may wait for an
        // already-running same-directory owner. Manual refresh still reads
        // fresh; no waiting on the UI thread or indefinite worker wait.
        let mut lease=self.begin(key.clone());
        if lease.is_none()&&allow_reuse{
            let deadline=Instant::now()+Duration::from_secs(2);
            if let Ok(mut store)=self.store.lock(){
                while store.active.contains(&key)&&!cancel.load(Ordering::Relaxed){
                    let left=deadline.saturating_duration_since(Instant::now());
                    if left.is_zero(){break;}
                    match self.ready.wait_timeout(store,left.min(Duration::from_millis(20))){
                        Ok((next,_))=>store=next,Err(_)=>return Err("Listing cache lock failed".into()),
                    }
                }
                drop(store);
                fsops::check_cancel(cancel)?;
                lease=self.begin(key);
                if lease.as_ref().is_some_and(|(_,s)|s.is_some()){self.coalesced.fetch_add(1,Ordering::Relaxed);}
            }
        }
        let Some((lease,old))=lease else{
            self.full.fetch_add(1,Ordering::Relaxed);
            return list_with_alias_check(path,show_hidden,cancel).map(scan_read)
        };
        if allow_reuse{
            if let Some(mut old)=old{
                if old.created.elapsed()<MAX_AGE && may_list(path) && native::folder_size_identity(path)==Some(old.identity){
                    // Writes often produce multiple separate notifications
                    // (size and last-write time). Drain a bounded number of
                    // batches and reconcile again, rather than mistaking the
                    // second notification for a journal overflow.
                    let linked_before=link_weight(&old.linked);
                    let delta_before=old.last_delta.as_ref().map_or(0,LastDelta::bytes);
                    let mut changed_names=HashSet::new();let mut removed=Vec::new();
                    let mut events=Vec::new();let mut structural=false;
                    let mut clean=false;
                    for _ in 0..8{
                        match old.watch.poll(){
                            Ok(None)=>{clean=true;break;}
                            Ok(Some(Changes::Modified(names)))=>{
                                events.extend(names.into_iter().map(|name|(FILE_ACTION_MODIFIED,name)));
                            }
                            Ok(Some(Changes::Structural(batch)))=>{structural=true;events.extend(batch);}
                            _=>break,
                        }
                        if events.len()>512{break;}
                    }
                    if clean&&structural{
                        if base!=Some(old.revision){clean=false;}
                        else{
                            match reconcile_structural(path,&mut old,&events,show_hidden,cancel){
                                Ok((changed,deleted))=>{changed_names=changed;removed=deleted;}
                                Err(_)=>clean=false,
                            }
                        }
                    }else if clean&&!events.is_empty(){
                        let names:HashSet<_>=events.into_iter().map(|(_,name)|name).collect();
                        if names.len()>MAX_CHANGED{clean=false;}
                        else{
                            match update_modified(path,&mut old.entries,&names,&mut old.linked,show_hidden,cancel){
                                Ok(changed)=>changed_names.extend(changed),Err(_)=>clean=false,
                            }
                        }
                    }
                    // A different folder can acquire the FIRST additional
                    // alias without notifying this directory. For small
                    // cached listings, periodically verify all displayed
                    // ordinary files, not just already-known aliases.
                    if clean&&old.entries.len()<=ALIAS_AUDIT_ROWS&&old.alias_audited.elapsed()>=ALIAS_AUDIT_AGE{
                        let names:HashSet<_>=old.entries.iter()
                            .filter(|e|!e.is_dir&&e.attributes&FILE_ATTRIBUTE_REPARSE_POINT==0)
                            .map(|e|e.name.clone()).collect();
                        match update_modified(path,&mut old.entries,&names,&mut old.linked,show_hidden,cancel){
                            Ok(changed)=>{
                                changed_names.extend(changed);
                                old.alias_audited=Instant::now();
                                self.alias_audits.fetch_add(1,Ordering::Relaxed);
                            }
                            Err(_)=>clean=false,
                        }
                    }
                    if clean&&!old.linked.is_empty(){
                        let names=old.linked.clone();
                        match update_modified(path,&mut old.entries,&names,&mut old.linked,show_hidden,cancel){
                            Ok(changed)=>changed_names.extend(changed),Err(_)=>clean=false,
                        }
                    }
                    if clean&&(!old.watch.clean()||changed_names.len().saturating_add(removed.len())>MAX_CHANGED){
                        clean=false;
                    }
                    let replay=clean&&changed_names.is_empty()&&removed.is_empty()&&!structural&&
                        base.is_some_and(|b|old.last_delta.as_ref().is_some_and(|d|d.base==b));
                    if old.order_dirty&&base!=Some(old.revision)&&!replay{clean=false;}
                    if clean && !cancel.load(Ordering::Relaxed) &&
                        fsops::ensure_plain_path(path).is_ok() && native::folder_size_identity(path)==Some(old.identity){
                        let partial=base==Some(old.revision)||replay;
                        if replay{
                            let d=old.last_delta.as_ref().unwrap();
                            changed_names=d.changed.clone();removed=d.removed.clone();structural=d.structural;
                            self.replays.fetch_add(1,Ordering::Relaxed);
                        }else if !changed_names.is_empty()||!removed.is_empty(){
                            let previous=old.revision;
                            old.revision=self.serial.fetch_add(1,Ordering::Relaxed)+1;
                            old.last_delta=Some(LastDelta{base:previous,changed:changed_names.clone(),removed:removed.clone(),structural});
                        }
                        let entries=if partial&&changed_names.is_empty(){Vec::new()}
                            else{old.entries.iter().filter(|e|!partial||changed_names.contains(&e.name)).map(|e|e.materialize(path)).collect()};
                        old.verified=Instant::now();
                        let revision=old.revision;
                        let hardlinks=!old.linked.is_empty();
                        let base_weight=if structural{weight(&old.entries)}
                            else{old.bytes.saturating_sub(linked_before).saturating_sub(delta_before)};
                        old.bytes=base_weight.saturating_add(link_weight(&old.linked))
                            .saturating_add(old.last_delta.as_ref().map_or(0,LastDelta::bytes));
                        self.put(&lease,old);
                        if partial{self.partial.fetch_add(1,Ordering::Relaxed);}
                        if !partial{removed.clear();}
                        if replay||changed_names.is_empty()&&removed.is_empty(){self.hits.fetch_add(1,Ordering::Relaxed);}
                        else{self.deltas.fetch_add(1,Ordering::Relaxed);self.changed.fetch_add((changed_names.len()+removed.len()) as u64,Ordering::Relaxed);}
                        self.reuse_us.fetch_add(started.elapsed().as_micros() as u64,Ordering::Relaxed);
                        if structural{self.structural.fetch_add(1,Ordering::Relaxed);}
                        return Ok(ListingRead{entries,revision:Some(revision),partial,removed,hardlinks,structural})
                    }
                }
            }
        }else{drop(old)}
        fsops::check_cancel(cancel)?;
        let created=Instant::now();
        let identity=native::folder_size_identity(path);
        let watch=identity.and_then(|_|ChangeWatch::open(path));
        let enumerating=Instant::now();
        let Scan{entries,linked,unverified}=list_with_alias_check(path,show_hidden,cancel)?;
        let hardlinks=!linked.is_empty();
        self.alias_rows.fetch_add(linked.len() as u64,Ordering::Relaxed);
        self.unverified.fetch_add(unverified as u64,Ordering::Relaxed);
        self.enumerate_us.fetch_add(enumerating.elapsed().as_micros() as u64,Ordering::Relaxed);
        let compacting=Instant::now();
        self.full.fetch_add(1,Ordering::Relaxed);
        let mut revision=None;
        if entries.len()<=MAX_ROWS && unverified==0{
            if let (Some(mut watch),Some(identity))=(watch,identity){
                if !cancel.load(Ordering::Relaxed) && watch.clean() &&
                    fsops::ensure_plain_path(path).is_ok() && native::folder_size_identity(path)==Some(identity){
                    // Reject very long-name collections before allocating a
                    // second representation. The estimate is conservative.
                    let estimate=entries.iter().fold(32768usize,|sum,e|sum.saturating_add(
                        std::mem::size_of::<CachedEntry>()+32+e.path.file_name().map_or(0,|n|n.as_encoded_bytes().len())));
                    let compact:Option<Vec<_>>=if estimate<=CACHE_BYTES{
                        entries.iter().map(CachedEntry::from_entry).collect()
                    }else{None};
                    if let Some(compact)=compact{
                        let bytes=weight(&compact).saturating_add(link_weight(&linked));
                        if bytes<=CACHE_BYTES && !cancel.load(Ordering::Relaxed) && watch.clean(){
                            let id=self.serial.fetch_add(1,Ordering::Relaxed)+1;
                            revision=Some(id);
                            self.put(&lease,Snapshot{entries:compact,watch,identity,verified:Instant::now(),created,bytes,revision:id,linked,alias_audited:Instant::now(),order_dirty:false,last_delta:None});
                        }
                    }
                }
            }
        }
        self.compact_us.fetch_add(compacting.elapsed().as_micros() as u64,Ordering::Relaxed);
        Ok(ListingRead{entries,revision,partial:false,removed:Vec::new(),hardlinks,structural:false})
    }
    pub fn report(&self)->serde_json::Value{
        let store=self.store.lock().unwrap_or_else(|e|e.into_inner());
        serde_json::json!({"hits":self.hits.load(Ordering::Relaxed),"deltas":self.deltas.load(Ordering::Relaxed),
            "changed":self.changed.load(Ordering::Relaxed),"full":self.full.load(Ordering::Relaxed),
            "entries":store.items.len(),"estimatedBytes":store.bytes,"activeReads":store.active.len(),"coalescedReads":self.coalesced.load(Ordering::Relaxed),"partialReads":self.partial.load(Ordering::Relaxed),
            "leasedBytes":store.leased,"peakStoredAndLeasedEstimate":store.peak,
            "enumerateUs":self.enumerate_us.load(Ordering::Relaxed),"compactUs":self.compact_us.load(Ordering::Relaxed),
            "reuseUs":self.reuse_us.load(Ordering::Relaxed),"aliasRows":self.alias_rows.load(Ordering::Relaxed),"aliasAudits":self.alias_audits.load(Ordering::Relaxed),"unverifiedRows":self.unverified.load(Ordering::Relaxed),"structuralReads":self.structural.load(Ordering::Relaxed),"deltaReplays":self.replays.load(Ordering::Relaxed)})
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
    fn native_directory_ids_correct_stale_hardlink_metadata(){
        let root=std::env::temp_dir().join(format!("ndr-link-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&root).unwrap();
        let a=root.join("a.bin");let b=root.join("alias.bin");let c=root.join("c.bin");
        fs::write(&a,b"short").unwrap();fs::hard_link(&a,&b).unwrap();fs::write(&c,b"other").unwrap();
        fs::write(&a,vec![7;500]).unwrap();
        let cancel=AtomicBool::new(false);
        let first=scan_ids(&root,true,&cancel).unwrap().expect("NTFS test fixture").entries;
        assert_eq!(first.len(),3);
        for e in first.iter().filter(|e|e.path==a||e.path==b){
            let fresh=fs::symlink_metadata(&e.path).unwrap();
            assert_eq!(e.size,500);assert_eq!(e.modified,fresh.modified().ok());assert_eq!(e.attributes,fresh.file_attributes());
        }
        let legacy=fsops::list_dir(&root,true).unwrap();
        assert_eq!(first.iter().map(|e|&e.name).collect::<Vec<_>>(),legacy.iter().map(|e|&e.name).collect::<Vec<_>>());
        let cache=ListingCache::default();
        let initial=cache.read_revision(&root,false,false,true,&cancel,None).unwrap();
        fs::write(&b,vec![1;702]).unwrap();std::thread::sleep(Duration::from_millis(180));
        let changed=cache.read_revision(&root,false,true,true,&cancel,initial.revision).unwrap();
        assert!(changed.hardlinks);
        assert_eq!(changed.entries.iter().filter(|e|e.path==a||e.path==b).map(|e|e.size).collect::<Vec<_>>(),vec![702,702]);
        let uncached=cache.read_revision(&root,false,false,false,&cancel,None).unwrap();
        assert_eq!(uncached.entries.iter().filter(|e|e.path==a||e.path==b).map(|e|e.size).collect::<Vec<_>>(),vec![702,702]);
        drop(cache);fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cross_directory_aliases_are_fresh_on_initial_and_cached_read(){
        let root=std::env::temp_dir().join(format!("ndr-cross-links-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let left=root.join("left");let right=root.join("right");
        fs::create_dir_all(&left).unwrap();fs::create_dir_all(&right).unwrap();
        let a=left.join("a.bin");let b=right.join("alias.bin");
        fs::write(&a,vec![0;100]).unwrap();fs::hard_link(&a,&b).unwrap();fs::write(&a,vec![1;32768]).unwrap();
        let cache=ListingCache::default();let cancel=AtomicBool::new(false);
        let first=cache.read_revision(&right,true,true,true,&cancel,None).unwrap();
        assert!(first.hardlinks);assert_eq!(first.entries[0].size,32768);
        fs::write(&a,vec![2;65536]).unwrap();
        let next=cache.read_revision(&right,true,true,true,&cancel,first.revision).unwrap();
        assert_eq!(next.entries[0].size,65536);assert!(next.hardlinks);
        drop(cache);fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn periodic_alias_audit_discovers_a_new_external_hardlink(){
        let root=std::env::temp_dir().join(format!("ndr-late-link-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let left=root.join("left");let outside=root.join("outside");
        fs::create_dir_all(&left).unwrap();fs::create_dir_all(&outside).unwrap();
        let a=left.join("a.bin");let b=outside.join("alias.bin");fs::write(&a,[0u8;100]).unwrap();
        let cache=ListingCache::default();let cancel=AtomicBool::new(false);
        let initial=cache.read_revision(&left,true,true,true,&cancel,None).unwrap();
        assert!(!initial.hardlinks);assert_eq!(initial.entries[0].size,100);
        fs::hard_link(&a,&b).unwrap();fs::write(&b,vec![1u8;65536]).unwrap();
        // Age the audit timestamp, not the directory snapshot. This is the
        // same path as the real 10-second background verification.
        {let mut store=cache.store.lock().unwrap();
         store.items.get_mut(&(left.clone(),true)).unwrap().alias_audited=Instant::now()-ALIAS_AUDIT_AGE;}
        let changed=cache.read_revision(&left,true,true,true,&cancel,initial.revision).unwrap();
        assert!(changed.hardlinks);assert_eq!(changed.entries[0].size,65536);
        assert_eq!(cache.report()["aliasAudits"],1);
        drop(cache);fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compact_native_name_and_single_owner(){
        use std::os::windows::ffi::OsStringExt;
        let root=PathBuf::from(r"C:\fixture");
        let native=OsString::from_wide(&[0x0061,0xd800,0x002e,0x0074]);
        let entry=Entry{path:root.join(&native),name:native.to_string_lossy().into_owned(),
            is_dir:false,size:42,modified:None,hidden:true,attributes:2,drive:false};
        let compact=CachedEntry::from_entry(&entry).unwrap();
        let rebuilt=compact.materialize(&root);
        assert_eq!(rebuilt.path,entry.path);assert_eq!(rebuilt.name,entry.name);
        assert_eq!(rebuilt.size,42);assert!(rebuilt.hidden);
        let cache=ListingCache::default();
        let key=(root,false);
        let (lease,_)=cache.begin(key.clone()).unwrap();
        assert!(cache.begin(key.clone()).is_none());
        cache.clear();
        assert_ne!(lease.epoch,cache.store.lock().unwrap().epoch);
        drop(lease);
        assert!(cache.begin(key).is_some());
    }

    #[test]
    fn revision_delta_expiry_and_structural_patch(){
        let root=std::env::temp_dir().join(format!("ndr-delta-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&root).unwrap();
        let a=root.join("a.txt");let b=root.join("b.txt");
        fs::write(&a,b"a").unwrap();fs::write(&b,b"bb").unwrap();
        let cache=ListingCache::default();let cancel=AtomicBool::new(false);
        let first=cache.read_revision(&root,false,false,true,&cancel,None).unwrap();
        let Some(base)=first.revision else{fs::remove_dir_all(&root).unwrap();return};
        fs::write(&a,b"longer").unwrap();std::thread::sleep(Duration::from_millis(180));
        let partial=cache.read_revision(&root,false,true,true,&cancel,Some(base)).unwrap();
        assert!(partial.partial);assert_eq!(partial.entries.len(),1);assert_eq!(partial.entries[0].size,6);
        assert_ne!(partial.revision,Some(base));
        let stale=cache.read_revision(&root,false,true,true,&cancel,Some(base)).unwrap();
        assert!(stale.partial);assert_eq!(stale.entries.len(),1);
        assert_eq!(stale.revision,partial.revision);assert_eq!(cache.report()["deltaReplays"],1);
        fs::rename(&a,root.join("c.txt")).unwrap();std::thread::sleep(Duration::from_millis(180));
        let renamed=cache.read_revision(&root,false,true,true,&cancel,stale.revision).unwrap();
        assert!(renamed.partial);assert!(renamed.structural);assert_eq!(renamed.entries.len(),1);
        assert_eq!(renamed.removed,vec![OsString::from("a.txt")]);
        assert!(renamed.entries.iter().any(|e|e.name=="c.txt"));
        if let Some(snapshot)=cache.store.lock().unwrap().items.get_mut(&(root.clone(),false)){
            snapshot.created=Instant::now()-MAX_AGE-Duration::from_secs(1);
        }
        cache.expire();assert_eq!(cache.report()["entries"],0);
        drop(cache);fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_create_remove_and_raw_order_fallback(){
        let root=std::env::temp_dir().join(format!("ndr-structure-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&root).unwrap();fs::write(root.join("m.txt"),b"x").unwrap();
        let cache=ListingCache::default();let cancel=AtomicBool::new(false);
        let first=cache.read_revision(&root,true,true,true,&cancel,None).unwrap();
        fs::write(root.join("a.txt"),b"new").unwrap();std::thread::sleep(Duration::from_millis(200));
        let add=cache.read_revision(&root,true,true,true,&cancel,first.revision).unwrap();
        assert!(add.partial&&add.structural);assert_eq!(add.entries[0].name,"a.txt");
        assert!(add.removed.is_empty());
        let twin=cache.read_revision(&root,true,true,true,&cancel,first.revision).unwrap();
        assert!(twin.partial&&twin.structural);assert_eq!(twin.revision,add.revision);assert_eq!(twin.entries[0].name,"a.txt");
        fs::remove_file(root.join("m.txt")).unwrap();std::thread::sleep(Duration::from_millis(200));
        let del=cache.read_revision(&root,true,true,true,&cancel,add.revision).unwrap();
        assert!(del.partial&&del.structural);assert!(del.entries.is_empty());assert_eq!(del.removed,vec![OsString::from("m.txt")]);
        let raw=cache.read_revision(&root,true,true,true,&cancel,None).unwrap();
        assert!(!raw.partial);assert_eq!(raw.entries.len(),1);assert_eq!(raw.entries[0].name,"a.txt");
        drop(cache);fs::remove_dir_all(root).unwrap();
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
