//! Shared concurrency budget for exact local metadata validation.
//! Each listing already has a caller worker. Reserve only extra threads,
//! so two panes cannot each start a full independent CPU-sized group.
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct VerificationBudget {
    used: AtomicUsize,
}
impl VerificationBudget {
    pub(crate) const fn new() -> Self { Self { used: AtomicUsize::new(0) } }
    pub(crate) fn acquire(&self, requested: usize, parallelism: usize) -> VerificationPermit<'_> {
        // The caller always contributes one worker. Preserve capacity for
        // a second listing caller and cap all extra workers across panes.
        let limit = parallelism.saturating_sub(2).min(10);
        let wanted = requested.saturating_sub(1).min(limit);
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

