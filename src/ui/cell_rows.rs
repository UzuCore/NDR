//! Adaptive storage for immutable list display cells. Large ordinary lists
//! keep the name column dense and encode consecutive identical values in
//! other columns as runs. High-cardinality columns fall back to dense
//! storage; sensitive/Vault rows always use independent dense strings.
use std::{ops::Range,sync::Arc};
pub(super) type CellRow=[Arc<str>;7];
pub(super) type CellRowRef<'a>=[&'a Arc<str>;7];
const COMPRESS_ROWS:usize=4096;
const DENSE_CHECK_ROWS:usize=64;

#[derive(Clone,Debug)]
struct Run {
    end:usize, // exclusive row index
    text:Arc<str>,
}

#[derive(Clone,Debug)]
enum Column {
    Dense(Vec<Arc<str>>),
    Runs(Vec<Run>),
}

impl Column {
    fn get(&self,row:usize)->&Arc<str>{
        match self {
            Self::Dense(rows)=>&rows[row],
            Self::Runs(runs)=>&runs[runs.partition_point(|r|r.end<=row)].text,
        }
    }
    fn make_dense(&mut self,len:usize,reserve:usize){
        if let Self::Runs(runs)=self {
            let mut rows=Vec::with_capacity(len.max(reserve));
            let mut start=0;
            for r in runs.iter(){
                rows.extend(std::iter::repeat_with(||r.text.clone()).take(r.end-start));
                start=r.end;
            }
            debug_assert_eq!(rows.len(),len);
            *self=Self::Dense(rows);
        }
    }
    fn limit_density(&mut self,len:usize,reserve:usize){
        // A run is larger than a dense Arc reference. Even with vector
        // growth, the compressed representation should stay economical.
        if matches!(self,Self::Runs(runs) if len>=DENSE_CHECK_ROWS && runs.len()>len/4){
            self.make_dense(len,reserve);
        }
    }
    fn push(&mut self,text:Arc<str>,old_len:usize,reserve:usize){
        match self {
            Self::Dense(rows)=>rows.push(text),
            Self::Runs(runs)=>{
                if let Some(last)=runs.last_mut().filter(|r|r.text==text){
                    last.end=old_len+1;
                    return;
                }
                runs.push(Run{end:old_len+1,text});
                self.limit_density(old_len+1,reserve);
            }
        }
    }
    /// Split a run at a row boundary and return the index of the first
    /// run at/after that boundary. A boundary at len returns runs.len().
    fn split(runs:&mut Vec<Run>,at:usize)->usize{
        let index=runs.partition_point(|r|r.end<=at);
        let start=if index==0{0}else{runs[index-1].end};
        if index<runs.len() && start<at {
            let after=runs[index].clone();
            runs[index].end=at;
            runs.insert(index+1,after);
            index+1
        }else{index}
    }
    fn merge(runs:&mut Vec<Run>,index:usize){
        if index>0 && index<runs.len() && runs[index-1].text==runs[index].text{
            runs[index-1].end=runs[index].end;
            runs.remove(index);
        }
    }
    fn insert(&mut self,at:usize,text:Arc<str>,len:usize,reserve:usize){
        if at==len{self.push(text,len,reserve);return;}
        match self {
            Self::Dense(rows)=>rows.insert(at,text),
            Self::Runs(runs)=>{
                let i=Self::split(runs,at);
                for r in &mut runs[i..]{r.end+=1;}
                runs.insert(i,Run{end:at+1,text});
                Self::merge(runs,i+1);
                Self::merge(runs,i);
                self.limit_density(len+1,reserve);
            }
        }
    }
    fn remove(&mut self,at:usize,len:usize){
        match self {
            Self::Dense(rows)=>{rows.remove(at);},
            Self::Runs(runs)=>{
                let i=Self::split(runs,at);
                let end=Self::split(runs,at+1);
                debug_assert_eq!(end,i+1);
                runs.remove(i);
                for r in &mut runs[i..]{r.end-=1;}
                Self::merge(runs,i);
                self.limit_density(len-1,len-1);
            }
        }
    }
    fn set(&mut self,at:usize,text:Arc<str>,len:usize){
        if self.get(at)==&text{return;}
        match self {
            Self::Dense(rows)=>rows[at]=text,
            Self::Runs(_)=>{
                self.remove(at,len);
                self.insert(at,text,len-1,len);
            }
        }
    }
    #[cfg(test)]
    fn allocation_bytes(&self)->usize{
        match self{
            Self::Dense(v)=>v.capacity()*std::mem::size_of::<Arc<str>>(),
            Self::Runs(v)=>v.capacity()*std::mem::size_of::<Run>(),
        }
    }
}

#[derive(Clone,Debug)]
enum Storage{
    Dense(Vec<CellRow>),
    Columns{len:usize,reserve:usize,columns:[Column;7]},
}

/// The whole snapshot is shared between identical panes. Mutation uses
/// COW, preserving the Arc identity of unaffected names. Row references
/// are ordinary borrows: no extra owned text or per-paint allocation.
#[derive(Clone,Debug)]
pub(super) struct CellRows(Arc<Storage>);
impl Default for CellRows{
    fn default()->Self{Self(Arc::new(Storage::Dense(Vec::new())))}
}
impl PartialEq for CellRows{
    fn eq(&self,other:&Self)->bool{
        Arc::ptr_eq(&self.0,&other.0) ||
            (self.len()==other.len() && self.iter().zip(other.iter()).all(|(a,b)|a==b))
    }
}
impl Eq for CellRows{}

impl CellRows {
    pub(super) fn reset_for_build(&mut self,count:usize,ordinary_local:bool){
        if ordinary_local && count>=COMPRESS_ROWS {
            self.0=Arc::new(Storage::Columns{
                len:0,reserve:count,columns:std::array::from_fn(|i|
                    if i==0{Column::Dense(Vec::with_capacity(count))}else{Column::Runs(Vec::new())}),
            });
        }else if let Some(Storage::Dense(rows))=Arc::get_mut(&mut self.0){
            if rows.capacity()>count.saturating_mul(2).max(1024){
                *rows=Vec::with_capacity(count);
            }else{rows.clear();rows.reserve(count);}
        }else{
            self.0=Arc::new(Storage::Dense(Vec::with_capacity(count)));
        }
    }
    pub(super) fn len(&self)->usize{
        match self.0.as_ref(){Storage::Dense(rows)=>rows.len(),Storage::Columns{len,..}=>*len}
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self)->bool{self.len()==0}
    pub(super) fn row(&self,row:usize)->CellRowRef<'_>{
        match self.0.as_ref(){
            Storage::Dense(rows)=>std::array::from_fn(|i|&rows[row][i]),
            Storage::Columns{len,columns,..}=>{
                assert!(row<*len);
                std::array::from_fn(|i|columns[i].get(row))
            }
        }
    }
    pub(super) fn cell(&self,row:usize,column:usize)->&Arc<str>{
        match self.0.as_ref(){
            Storage::Dense(rows)=>&rows[row][column],
            Storage::Columns{len,columns,..}=>{
                assert!(row<*len);
                columns[column].get(row)
            }
        }
    }
    pub(super) fn get(&self,row:usize)->Option<CellRowRef<'_>>{
        (row<self.len()).then(||self.row(row))
    }
    pub(super) fn first(&self)->Option<CellRowRef<'_>>{self.get(0)}
    pub(super) fn range(&self,rows:Range<usize>)->CellRange<'_>{
        assert!(rows.end<=self.len() && rows.start<=rows.end);
        match self.0.as_ref(){
            Storage::Dense(dense)=>CellRange(CellRangeInner::Dense(dense[rows].iter())),
            Storage::Columns{columns,..} if columns.iter().all(|c|matches!(c,Column::Dense(_)))=>
                CellRange(CellRangeInner::DenseColumns{
                    columns:std::array::from_fn(|i|match &columns[i]{
                        Column::Dense(v)=>v.as_slice(),
                        Column::Runs(_)=>unreachable!("all columns were dense"),
                    }),rows}),
            Storage::Columns{columns,..}=>CellRange(CellRangeInner::Columns(ColumnRange::new(columns,rows))),
        }
    }
    pub(super) fn iter(&self)->impl DoubleEndedIterator<Item=CellRowRef<'_>>+ExactSizeIterator+'_{
        self.range(0..self.len())
    }
    pub(super) fn push(&mut self,row:CellRow){
        match Arc::make_mut(&mut self.0){
            Storage::Dense(rows)=>rows.push(row),
            Storage::Columns{len,reserve,columns}=>{
                for (c,text) in columns.iter_mut().zip(row){c.push(text,*len,*reserve);}
                *len+=1;
            }
        }
    }
    pub(super) fn insert(&mut self,at:usize,row:CellRow){
        match Arc::make_mut(&mut self.0){
            Storage::Dense(rows)=>rows.insert(at,row),
            Storage::Columns{len,reserve,columns}=>{
                assert!(at<=*len);
                for (c,text) in columns.iter_mut().zip(row){c.insert(at,text,*len,*reserve);}
                *len+=1;
            }
        }
    }
    pub(super) fn remove(&mut self,at:usize){
        match Arc::make_mut(&mut self.0){
            Storage::Dense(rows)=>{rows.remove(at);},
            Storage::Columns{len,columns,..}=>{
                assert!(at<*len);
                for c in columns{c.remove(at,*len);}
                *len-=1;
            }
        }
    }
    pub(super) fn set(&mut self,row:usize,column:usize,text:Arc<str>){
        match Arc::make_mut(&mut self.0){
            Storage::Dense(rows)=>rows[row][column]=text,
            Storage::Columns{len,columns,..}=>{
                assert!(row<*len);
                columns[column].set(row,text,*len);
            }
        }
    }
    pub(super) fn owned_row(&self,row:usize)->CellRow{self.row(row).map(Arc::clone)}
    pub(super) fn into_rows(self)->impl Iterator<Item=CellRow>{
        // Only bounded partial listing edits consume this iterator.
        (0..self.len()).map(move |i|self.owned_row(i))
    }
    pub(super) fn wipe_sensitive(&mut self){
        let Storage::Dense(rows)=Arc::make_mut(&mut self.0)else{
            // Compression must never be enabled for a sensitive pane.
            unreachable!("sensitive cells must be dense")
        };
        use zeroize::Zeroize;
        for row in rows{
            for cell in row{
                if let Some(text)=Arc::get_mut(cell){
                    // SAFETY: writing NUL preserves UTF-8 validity.
                    unsafe{text.as_bytes_mut().zeroize();}
                }else{debug_assert!(cell.is_empty(),"shared sensitive cell");}
            }
        }
    }
    #[cfg(test)]
    pub(super) fn as_ptr(&self)->*const (){Arc::as_ptr(&self.0).cast()}
    #[cfg(test)]
    pub(super) fn capacity(&self)->usize{
        match self.0.as_ref(){
            Storage::Dense(rows)=>rows.capacity(),
            Storage::Columns{reserve,len,..}=>*reserve.max(len),
        }
    }
    #[cfg(test)]
    fn allocation_bytes(&self)->usize{
        match self.0.as_ref(){
            Storage::Dense(rows)=>rows.capacity()*std::mem::size_of::<CellRow>(),
            Storage::Columns{columns,..}=>columns.iter().map(Column::allocation_bytes).sum(),
        }
    }
}
/// Sequential readers (field measurement and equality) visit each run
/// once instead of doing a binary search for every row and every column.
/// Independent forward/backward cursors allow mixed-ended iteration;
/// random visible-row access still uses the original exact binary search.
pub(super) struct CellRange<'a>(CellRangeInner<'a>);
enum CellRangeInner<'a>{
    Dense(std::slice::Iter<'a,CellRow>),
    // Every value was unique: use direct indexing with no run cursor
    // or enum branch per column.
    DenseColumns{columns:[&'a [Arc<str>];7],rows:Range<usize>},
    Columns(ColumnRange<'a>),
}
struct ColumnRange<'a>{
    columns:&'a [Column;7],
    rows:Range<usize>,
    front:[usize;7],
    back:[usize;7],
}
impl<'a> ColumnRange<'a>{
    fn new(columns:&'a [Column;7],rows:Range<usize>)->Self{
        let cursor=|row|std::array::from_fn(|i|match &columns[i]{
            Column::Dense(_)=>0,
            Column::Runs(runs)=>runs.partition_point(|r|r.end<=row),
        });
        let front=cursor(rows.start);
        let back=cursor(rows.end.saturating_sub(1));
        Self{columns,rows,front,back}
    }
    fn next(&mut self)->Option<CellRowRef<'a>>{
        let row=self.rows.next()?;
        let columns=self.columns;
        Some(std::array::from_fn(|i|match &columns[i]{
            Column::Dense(dense)=>&dense[row],
            Column::Runs(runs)=>{
                let cursor=&mut self.front[i];
                // The range advances one row at a time, so at most one
                // nonempty run boundary can be crossed.
                if runs[*cursor].end<=row{*cursor+=1;}
                &runs[*cursor].text
            }
        }))
    }
    fn next_back(&mut self)->Option<CellRowRef<'a>>{
        let row=self.rows.next_back()?;
        let columns=self.columns;
        Some(std::array::from_fn(|i|match &columns[i]{
            Column::Dense(dense)=>&dense[row],
            Column::Runs(runs)=>{
                let cursor=&mut self.back[i];
                if *cursor>0 && runs[*cursor-1].end>row{*cursor-=1;}
                &runs[*cursor].text
            }
        }))
    }
}
impl<'a> Iterator for CellRange<'a>{
    type Item=CellRowRef<'a>;
    fn next(&mut self)->Option<Self::Item>{
        match &mut self.0{
            CellRangeInner::Dense(rows)=>rows.next().map(|row|row.each_ref()),
            CellRangeInner::DenseColumns{columns,rows}=>{
                let row=rows.next()?;
                Some(std::array::from_fn(|i|&columns[i][row]))
            }
            CellRangeInner::Columns(rows)=>rows.next(),
        }
    }
    fn size_hint(&self)->(usize,Option<usize>){
        let len=match &self.0{
            CellRangeInner::Dense(rows)=>rows.len(),
            CellRangeInner::DenseColumns{rows,..}=>rows.len(),
            CellRangeInner::Columns(rows)=>rows.rows.len(),
        };
        (len,Some(len))
    }
}
impl DoubleEndedIterator for CellRange<'_>{
    fn next_back(&mut self)->Option<Self::Item>{
        match &mut self.0{
            CellRangeInner::Dense(rows)=>rows.next_back().map(|row|row.each_ref()),
            CellRangeInner::DenseColumns{columns,rows}=>{
                let row=rows.next_back()?;
                Some(std::array::from_fn(|i|&columns[i][row]))
            }
            CellRangeInner::Columns(rows)=>rows.next_back(),
        }
    }
}
impl ExactSizeIterator for CellRange<'_>{}
impl std::iter::FusedIterator for CellRange<'_>{}

#[cfg(test)]
mod tests{
    use super::*;
    fn row(i:usize)->CellRow{
        [Arc::from(format!("name-{i}")),Arc::from("bin"),Arc::from("File"),
         Arc::from(format!("{}",i%3)),Arc::from("Today"),Arc::from("A"),Arc::from("")]
    }
    #[test]
    fn compressed_columns_match_dense_rows_and_preserve_cow(){
        let n=16_384;
        let mut packed=CellRows::default();packed.reset_for_build(n,true);
        let mut plain=CellRows::default();plain.reset_for_build(n,false);
        for i in 0..n{let r=row(i);plain.push(r.clone());packed.push(r);}
        assert_eq!(packed,plain);
        assert!(packed.allocation_bytes()<plain.allocation_bytes()/2);
        let old=packed.clone();let old_name=packed.cell(n-1,0).clone();
        for index in [0,n/2,n-1]{
            packed.set(index,4,Arc::from("New date"));plain.set(index,4,Arc::from("New date"));
            packed.insert(index,row(n+index));plain.insert(index,row(n+index));
            packed.remove(index+1);plain.remove(index+1);
            assert_eq!(packed,plain);
        }
        assert!(Arc::ptr_eq(&old_name,old.cell(n-1,0)));
        assert_eq!(old.cell(0,4).as_ref(),"Today");
        assert_ne!(packed.as_ptr(),old.as_ptr());
        packed.reset_for_build(2,true);
        assert!(packed.is_empty()&&packed.capacity()<=1024);
        assert_eq!(old.len(),n);
    }
    #[test]
    fn cursor_ranges_match_random_access_in_both_directions(){
        let n=8192;
        for compressed in [false,true]{
            let mut p=CellRows::default();p.reset_for_build(n,compressed);
            for i in 0..n{
                p.push(std::array::from_fn(|c|Arc::from(format!("{c}-{}",i/(1+c*17)))));
            }
            for i in [3,63,1024,4096,8190]{p.set(i,4,Arc::from("edited"));}
            for (start,end) in [(0,0),(n,n),(1,2),(0,n),(62,137),(1019,4101),(n-91,n)]{
                let mut forward=p.range(start..end);
                for i in start..end{
                    assert_eq!(forward.len(),end-i);
                    assert_eq!(forward.next(),Some(p.row(i)));
                }
                assert_eq!(forward.next(),None);assert_eq!(forward.next_back(),None);
                assert_eq!(p.range(start..end).rev().collect::<Vec<_>>(),
                    (start..end).rev().map(|i|p.row(i)).collect::<Vec<_>>());
                let mut both=p.range(start..end);let (mut lo,mut hi)=(start,end);
                while lo<hi{
                    assert_eq!(both.next(),Some(p.row(lo)));lo+=1;
                    if lo<hi{hi-=1;assert_eq!(both.next_back(),Some(p.row(hi)));}
                    assert_eq!(both.len(),hi-lo);
                }
                assert_eq!(both.next_back(),None);
            }
        }
    }
    #[test]
    fn mixed_runs_random_edits_match_reference(){
        let n=4096;let mut packed=CellRows::default();packed.reset_for_build(n,true);
        let mut reference=Vec::new();
        for i in 0..n{
            let r=std::array::from_fn(|c|Arc::from(format!("column-{c}-{}",i/(1+c*32))));
            packed.push(r.clone());reference.push(r);
        }
        let mut seed=1u64;
        for i in 0..800{
            seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let at=seed as usize%reference.len();
            match i%3{
                0=>{let r=row(i);packed.insert(at,r.clone());reference.insert(at,r);}
                1=>{packed.remove(at);reference.remove(at);}
                _=>{let c=i%7;let t:Arc<str>=Arc::from(format!("change-{i}"));packed.set(at,c,t.clone());reference[at][c]=t;}
            }
            assert_eq!(packed.len(),reference.len());
            assert_eq!(packed.owned_row(at.min(reference.len()-1)),reference[at.min(reference.len()-1)]);
        }
        for (i,r) in reference.iter().enumerate(){assert_eq!(&packed.owned_row(i),r);}
    }
    #[test]
    fn unique_columns_are_dense_and_sensitive_never_compressed(){
        let n=8192;let mut p=CellRows::default();p.reset_for_build(n,true);
        for i in 0..n{p.push(std::array::from_fn(|c|Arc::from(format!("{c}-{i}"))));}
        assert!(matches!(p.0.as_ref(),Storage::Columns{columns,..} if columns.iter().all(|c|matches!(c,Column::Dense(_)))));
        assert_eq!(p.allocation_bytes(),n*std::mem::size_of::<CellRow>());
        assert!(matches!(p.range(0..n).0,CellRangeInner::DenseColumns{..}));
        assert_eq!(p.range(31..257).rev().collect::<Vec<_>>(),
            (31..257).rev().map(|i|p.row(i)).collect::<Vec<_>>());
        let mut both=p.range(31..257);
        assert_eq!(both.next(),Some(p.row(31)));assert_eq!(both.next_back(),Some(p.row(256)));
        assert_eq!(both.len(),224);
        p.reset_for_build(n,false);
        for _ in 0..n{p.push(std::array::from_fn(|_|Arc::from("secret")));}
        assert!(p.iter().flatten().all(|c|Arc::strong_count(c)==1));
        p.wipe_sensitive();
        assert!(p.iter().flatten().all(|c|c.bytes().all(|b|b==0)));
    }
}
