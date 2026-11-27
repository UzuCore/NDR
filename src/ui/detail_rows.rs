//! Visible detail-row indexing. An ordinary list is an identity range;
//! allocate explicit rows only when group headers actually need them.
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum DetailRow {
    Header(String),
    Item(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetailRowRef<'a> {
    Header(&'a str),
    Item(usize),
}

#[derive(Clone)]
pub(crate) enum DetailRows {
    Identity(usize),
    Explicit(Arc<Vec<DetailRow>>),
}
impl Default for DetailRows {
    fn default() -> Self { Self::Identity(0) }
}
impl DetailRows {
    pub(crate) fn reset(&mut self, count: usize, grouped: bool) {
        if !grouped {
            *self = Self::Identity(count);
        } else if let Self::Explicit(rows) = self {
            if let Some(rows) = Arc::get_mut(rows) {
                if rows.capacity() > count.saturating_mul(2).max(1024) {
                    *rows = Vec::with_capacity(count);
                } else {
                    rows.clear();
                    rows.reserve(count);
                }
            } else {
                *rows = Arc::new(Vec::with_capacity(count));
            }
        } else {
            *self = Self::Explicit(Arc::new(Vec::with_capacity(count)));
        }
    }
    pub(crate) fn push(&mut self, row: DetailRow) {
        // Only the grouped builder may push. Ordinary rows are already
        // represented by the full identity range.
        let Self::Explicit(rows) = self else { unreachable!("explicit detail rows required") };
        Arc::make_mut(rows).push(row);
    }
    pub(crate) fn len(&self) -> usize {
        match self { Self::Identity(n) => *n, Self::Explicit(rows) => rows.len() }
    }
    pub(crate) fn get(&self, line: usize) -> Option<DetailRowRef<'_>> {
        match self {
            Self::Identity(n) => (line < *n).then_some(DetailRowRef::Item(line)),
            Self::Explicit(rows) => rows.get(line).map(|r| match r {
                DetailRow::Header(label) => DetailRowRef::Header(label),
                DetailRow::Item(i) => DetailRowRef::Item(*i),
            }),
        }
    }
    pub(crate) fn line_for_item(&self, item: usize) -> Option<usize> {
        match self {
            Self::Identity(n) => (item < *n).then_some(item),
            Self::Explicit(rows) => rows.iter().position(|r| matches!(r,DetailRow::Item(i) if *i==item)),
        }
    }
    pub(crate) fn first_item_from(&self, line: usize) -> Option<usize> {
        match self {
            Self::Identity(n) => (line < *n).then_some(line),
            Self::Explicit(rows) => rows.get(line..)?.iter().find_map(item_index),
        }
    }
    pub(crate) fn last_item_through(&self, line: usize) -> Option<usize> {
        match self {
            Self::Identity(n) => n.checked_sub(1).map(|last| last.min(line)),
            Self::Explicit(rows) => rows[..rows.len().min(line.saturating_add(1))].iter().rev().find_map(item_index),
        }
    }
    pub(crate) fn is_identity(&self, len: usize) -> bool {
        match self {
            Self::Identity(n) => *n == len,
            Self::Explicit(rows) => rows.len() == len && rows.iter().enumerate()
                .all(|(i,r)| matches!(r,DetailRow::Item(j) if i==*j)),
        }
    }
    pub(crate) fn for_each_header_mut(&mut self, mut f: impl FnMut(&mut String)) {
        if let Self::Explicit(rows) = self {
            for row in Arc::make_mut(rows) {
                if let DetailRow::Header(label) = row { f(label); }
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        match self { Self::Identity(_) => 0, Self::Explicit(rows) => rows.capacity() }
    }
    #[cfg(test)]
    pub(crate) fn explicit_ptr(&self) -> Option<*const DetailRow> {
        match self { Self::Identity(_) => None, Self::Explicit(rows) => Some(rows.as_ptr()) }
    }
    #[cfg(test)]
    pub(crate) fn headers(&self) -> impl Iterator<Item=&str> {
        let rows: &[DetailRow] = match self { Self::Identity(_) => &[], Self::Explicit(rows) => rows };
        rows.iter().filter_map(|r| match r { DetailRow::Header(label) => Some(label.as_str()), _ => None })
    }
}
fn item_index(row: &DetailRow) -> Option<usize> {
    match row { DetailRow::Item(i) => Some(*i), _ => None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn million_identity_rows_allocate_no_vector_and_index_in_constant_time() {
        let mut rows=DetailRows::default();
        rows.reset(1_000_001,false);
        assert_eq!(rows.capacity(),0);
        assert_eq!(rows.len(),1_000_001);
        for i in [0,1,499_999,1_000_000] {
            assert_eq!(rows.get(i),Some(DetailRowRef::Item(i)));
            assert_eq!(rows.line_for_item(i),Some(i));
            assert_eq!(rows.first_item_from(i),Some(i));
            assert_eq!(rows.last_item_through(i),Some(i));
        }
        assert_eq!(rows.get(1_000_001),None);
        assert_eq!(rows.line_for_item(usize::MAX),None);
        assert_eq!(rows.first_item_from(usize::MAX),None);
        assert_eq!(rows.last_item_through(usize::MAX),Some(1_000_000));
        assert!(rows.is_identity(1_000_001));
        rows.reset(0,false);
        assert_eq!(rows.last_item_through(usize::MAX),None);
    }

    #[test]
    fn grouped_rows_match_reference_across_headers_and_out_of_range_positions() {
        let old=vec![
            DetailRow::Header("A".into()),DetailRow::Item(0),DetailRow::Item(1),
            DetailRow::Header("B".into()),DetailRow::Item(2),DetailRow::Header("C".into()),
            DetailRow::Item(3),DetailRow::Item(4),
        ];
        let mut rows=DetailRows::default();
        rows.reset(5,true);
        for row in old.iter().cloned(){rows.push(row);}
        assert_eq!(rows.len(),old.len());
        assert_eq!(rows.headers().collect::<Vec<_>>(),["A","B","C"]);
        for line in 0..=old.len()+2 {
            assert_eq!(rows.first_item_from(line),old.iter().skip(line).find_map(item_index));
            assert_eq!(rows.last_item_through(line),old.iter().take(line+1).rev().find_map(item_index));
            let expected=old.get(line).map(|r|match r{
                DetailRow::Header(label)=>DetailRowRef::Header(label),
                DetailRow::Item(i)=>DetailRowRef::Item(*i),
            });
            assert_eq!(rows.get(line),expected);
        }
        for item in 0..7{assert_eq!(rows.line_for_item(item),old.iter().position(|r|item_index(r)==Some(item)));}
        assert!(!rows.is_identity(5));
    }

    #[test]
    fn explicit_shared_rows_detach_and_release_on_identity_navigation() {
        let mut a=DetailRows::default();
        a.reset(100_000,true);
        a.push(DetailRow::Header("private label".into()));
        for i in 0..100_000{a.push(DetailRow::Item(i));}
        let mut b=a.clone();
        assert_eq!(a.explicit_ptr(),b.explicit_ptr());
        b.for_each_header_mut(|s|s.clear());
        assert_ne!(a.explicit_ptr(),b.explicit_ptr());
        assert_eq!(a.headers().collect::<Vec<_>>(),["private label"]);
        assert_eq!(b.headers().collect::<Vec<_>>(),[""]);
        b.reset(3,false);
        assert_eq!(b.capacity(),0);
        assert_eq!(b.len(),3);
        assert_eq!(a.len(),100_001);
        a.reset(1,true);
        assert!(a.capacity()<=1024);
        assert_eq!(a.len(),0);
    }
}

