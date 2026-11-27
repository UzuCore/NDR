//! Shared concurrency budget for exact local metadata validation.
//! Each listing already has a caller worker. Reserve only extra threads,
//! so two panes cannot each start a full independent CPU-sized group.
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct VerificationBudget {
    used: AtomicUsize,
    scans: AtomicUsize,
    light_scans: AtomicUsize,
}
impl VerificationBudget {
    pub(crate) const fn new() -> Self {
        Self { used: AtomicUsize::new(0), scans: AtomicUsize::new(0), light_scans: AtomicUsize::new(0) }
    }
    /// Register before directory enumeration so overlapping scans can
    /// see the other reader even before it begins exact metadata checks.
    pub(crate) fn begin_scan(&self) -> VerificationScan<'_> {
        self.scans.fetch_add(1, Ordering::AcqRel);
        VerificationScan { budget:self,light:false }
    }
    #[cfg(test)]
    fn acquire(&self,requested:usize,parallelism:usize)->VerificationPermit<'_>{
        self.acquire_for(requested,parallelism,false)
    }
    fn acquire_for(&self, requested: usize, parallelism: usize, light:bool) -> VerificationPermit<'_> {
        // The caller always contributes one worker. Preserve capacity for
        // a second listing caller and cap all extra workers across panes.
        let scans = self.scans.load(Ordering::Acquire).max(1);
        let limit = parallelism.saturating_sub(scans.max(2)).min(10);
        // Keep the original single-reader limit. If several enumerators
        // already overlap, reserve a fair share rather than letting the
        // first verifier consume the whole budget. Existing permits are
        // never revoked; late arrivals may receive fewer workers.
        let known_light=self.light_scans.load(Ordering::Acquire).min(scans);
        let heavy=scans-known_light;
        // An enumerator with no completed count is still treated as
        // heavy. For a verified smaller/slow-media scan, leave capacity
        // for a large scan rather than capping both at half. With two
        // scans on 12 logical CPUs this is 4+8 total workers (callers
        // included). Equal large scans still get 6+6. No revocation.
        let fair=if known_light>0 && heavy>0 {
            let total=known_light.saturating_mul(3).saturating_add(heavy.saturating_mul(7));
            (limit * if light{3}else{7}).div_ceil(total)
        }else{limit.div_ceil(scans)};
        let wanted = requested.saturating_sub(1).min(fair);
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            let reserve = wanted.min(limit.saturating_sub(current));
            if reserve == 0 { return VerificationPermit { budget: self, extra: 0 }; }
            match self.used.compare_exchange_weak(current, current + reserve, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return VerificationPermit { budget: self, extra: reserve },
                Err(next) => current = next,
            }
        }
    }
}
pub(crate) struct VerificationScan<'a> {
    budget: &'a VerificationBudget,
    light:bool,
}
impl<'a> VerificationScan<'a>{
    pub(crate) fn acquire(&mut self,requested:usize,parallelism:usize)->VerificationPermit<'a>{
        // The requested worker count already accounts for row count,
        // exact native stat availability and storage media.
        let light=requested<=8;
        if light!=self.light{
            if light{self.budget.light_scans.fetch_add(1,Ordering::AcqRel);}
            else{self.budget.light_scans.fetch_sub(1,Ordering::AcqRel);}
            self.light=light;
        }
        self.budget.acquire_for(requested,parallelism,light)
    }
}
impl Drop for VerificationScan<'_> {
    fn drop(&mut self) {
        if self.light{self.budget.light_scans.fetch_sub(1,Ordering::AcqRel);}
        self.budget.scans.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct VerificationPermit<'a> {
    budget: &'a VerificationBudget,
    extra: usize,
}
impl VerificationPermit<'_> {
    pub(crate) fn workers(&self) -> usize { self.extra + 1 }
}
impl Drop for VerificationPermit<'_> {
    fn drop(&mut self) {
        if self.extra != 0 { self.budget.used.fetch_sub(self.extra, Ordering::Relaxed); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_caps_two_panes_and_releases_on_drop() {
        let b = VerificationBudget::new();
        let a = b.acquire(12,12);
        let c = b.acquire(12,12);
        assert_eq!(a.workers(),11);
        assert_eq!(c.workers(),1);
        assert_eq!(a.workers()+c.workers(),12);
        drop(a);
        let next=b.acquire(8,12);
        assert_eq!(next.workers(),8);
        let remaining=b.acquire(8,12);
        assert_eq!(remaining.workers(),4);
        drop((next,remaining,c));
        assert_eq!(b.used.load(Ordering::Relaxed),0);
        assert_eq!(b.acquire(12,1).workers(),1);
        assert_eq!(b.acquire(12,2).workers(),1);
        assert_eq!(b.acquire(1,128).workers(),1);
    }
    #[test]
    fn overlapping_enumerations_share_capacity_before_first_verifier() {
        let b=VerificationBudget::new();
        let first=b.begin_scan();
        let second=b.begin_scan();
        let a=b.acquire(12,12);
        let c=b.acquire(12,12);
        assert_eq!(a.workers(),6);
        assert_eq!(c.workers(),6);
        drop((a,c,second));
        assert_eq!(b.acquire(12,12).workers(),11);
        drop(first);
        assert_eq!(b.scans.load(Ordering::Relaxed),0);
    }

    #[test]
    fn late_scan_does_not_revoke_existing_exact_metadata_workers() {
        let b=VerificationBudget::new();
        let first=b.begin_scan();
        let a=b.acquire(12,12);
        let second=b.begin_scan();
        let c=b.acquire(12,12);
        assert_eq!((a.workers(),c.workers()),(11,1));
        drop((a,c,first,second));
        assert_eq!(b.scans.load(Ordering::Relaxed),0);
        assert_eq!(b.used.load(Ordering::Relaxed),0);
    }

    #[test]
    fn small_scan_leaves_capacity_for_large_scan_and_releases_classification(){
        let b=VerificationBudget::new();
        let mut small=b.begin_scan();let mut large=b.begin_scan();
        let a=small.acquire(8,12);let c=large.acquire(12,12);
        assert_eq!((a.workers(),c.workers()),(4,8));
        assert_eq!(b.used.load(Ordering::Relaxed),10);
        drop((a,c,small,large));
        assert_eq!(b.used.load(Ordering::Relaxed),0);
        assert_eq!(b.light_scans.load(Ordering::Relaxed),0);
        assert_eq!(b.scans.load(Ordering::Relaxed),0);
        let mut only=b.begin_scan();
        assert_eq!(only.acquire(8,12).workers(),8);
    }
    #[test]
    fn weighted_late_arrival_never_revokes_or_overbooks(){
        let b=VerificationBudget::new();
        let mut large=b.begin_scan();let mut small=b.begin_scan();
        let a=large.acquire(12,12);let c=small.acquire(8,12);
        assert_eq!((a.workers(),c.workers()),(6,4));
        assert!(b.used.load(Ordering::Relaxed)<=10);
        drop((a,c,large,small));
        let mut first=b.begin_scan();let mut second=b.begin_scan();
        let a=first.acquire(12,12);let c=second.acquire(12,12);
        assert_eq!((a.workers(),c.workers()),(6,6));
    }

    #[test]
    fn concurrent_reservations_remain_bounded() {
        let b=VerificationBudget::new();
        let entered=std::sync::Barrier::new(9);
        let leaving=std::sync::Barrier::new(9);
        std::thread::scope(|scope|{
            for _ in 0..8{
                scope.spawn(||{
                    let permit=b.acquire(12,12);
                    entered.wait();
                    leaving.wait();
                    drop(permit);
                });
            }
            entered.wait();
            assert_eq!(b.used.load(Ordering::Relaxed),10);
            leaving.wait();
        });
        assert_eq!(b.used.load(Ordering::Relaxed),0);
    }
    #[test]
    fn budget_does_not_overflow_or_revoke_in_flight_work_when_affinity_changes() {
        let b=VerificationBudget::new();
        let first=b.acquire(usize::MAX,usize::MAX);
        assert_eq!(first.workers(),11);
        let reduced=b.acquire(8,4);
        assert_eq!(reduced.workers(),1);
        drop(first);
        assert_eq!(b.acquire(8,4).workers(),3);
        assert_eq!(b.used.load(Ordering::Relaxed),0);
    }
}

