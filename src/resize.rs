use crate::grid::Grid;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    Live,
    Coalesce { quiet: Duration, max_wait: Duration },
}

impl Default for Commit {
    fn default() -> Self {
        Self::Coalesce {
            quiet: Duration::from_millis(40),
            max_wait: Duration::from_millis(250),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    grid: Grid,
    first_seen: Instant,
    last_seen: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct GridReconciler {
    policy: Commit,
    committed: Option<Grid>,
    pending: Option<Pending>,
}

impl GridReconciler {
    #[must_use]
    pub fn new(policy: Commit) -> Self {
        Self {
            policy,
            committed: None,
            pending: None,
        }
    }

    #[must_use]
    pub fn committed(&self) -> Option<Grid> {
        self.committed
    }

    pub fn set_policy(&mut self, policy: Commit) {
        self.policy = policy;
    }

    #[must_use]
    pub fn observe(&mut self, target: Grid, now: Instant) -> Option<Grid> {
        if self.committed == Some(target) {
            self.pending = None;
            return None;
        }
        if self.committed.is_none() {
            return Some(self.commit(target));
        }
        match self.policy {
            Commit::Live => Some(self.commit(target)),
            Commit::Coalesce { .. } => {
                let first_seen = self.pending.map_or(now, |p| p.first_seen);
                self.pending = Some(Pending {
                    grid: target,
                    first_seen,
                    last_seen: now,
                });
                self.due(now)
            }
        }
    }

    #[must_use]
    pub fn due(&mut self, now: Instant) -> Option<Grid> {
        let pending = self.pending?;
        (now >= self.deadline_of(pending)).then(|| self.commit(pending.grid))
    }

    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|p| self.deadline_of(p))
    }

    fn deadline_of(&self, p: Pending) -> Instant {
        match self.policy {
            Commit::Live => p.last_seen,
            Commit::Coalesce { quiet, max_wait } => {
                (p.last_seen + quiet).min(p.first_seen + max_wait)
            }
        }
    }

    fn commit(&mut self, grid: Grid) -> Grid {
        self.pending = None;
        self.committed = Some(grid);
        grid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn grid(c: u16, r: u16) -> Grid {
        Grid::new(c, r).unwrap()
    }

    fn drag() -> impl Strategy<Value = Vec<(u16, u16, u64)>> {
        proptest::collection::vec((1u16..6, 1u16..4, 0u64..120), 1..60)
    }

    fn coalesce() -> Commit {
        Commit::Coalesce {
            quiet: Duration::from_millis(40),
            max_wait: Duration::from_millis(250),
        }
    }

    proptest! {
        #[test]
        fn live_commits_exactly_each_change(steps in drag()) {
            let t0 = Instant::now();
            let mut r = GridReconciler::new(Commit::Live);
            let mut now = t0;
            let mut expected = Vec::new();
            let mut got = Vec::new();
            for (c, w, dt) in steps {
                now += Duration::from_millis(dt);
                let g = grid(c, w);
                if expected.last() != Some(&g) {
                    expected.push(g);
                }
                got.extend(r.observe(g, now));
            }
            prop_assert_eq!(got, expected);
        }

        #[test]
        fn coalesce_settles_on_the_last_grid_and_respects_its_bounds(steps in drag()) {
            let t0 = Instant::now();
            let mut r = GridReconciler::new(coalesce());
            let mut now = t0;
            let mut last = None;
            let mut commits: Vec<(Grid, Instant)> = Vec::new();
            for (c, w, dt) in steps {
                now += Duration::from_millis(dt);
                commits.extend(r.due(now).map(|g| (g, now)));
                let g = grid(c, w);
                last = Some(g);
                commits.extend(r.observe(g, now).map(|g| (g, now)));
                if let Some(d) = r.deadline() {
                    prop_assert!(d <= now + Duration::from_millis(250));
                }
            }
            let settle = now + Duration::from_millis(250);
            commits.extend(r.due(settle).map(|g| (g, settle)));
            prop_assert_eq!(r.committed(), last);
            prop_assert_eq!(r.deadline(), None);
            for pair in commits.windows(2) {
                prop_assert_ne!(pair[0].0, pair[1].0);
            }
        }
    }

    #[test]
    fn the_first_grid_commits_at_once_under_every_policy() {
        let now = Instant::now();
        for policy in [Commit::Live, coalesce()] {
            let mut r = GridReconciler::new(policy);
            assert_eq!(r.observe(grid(80, 24), now), Some(grid(80, 24)));
        }
    }

    #[test]
    fn a_continuous_drag_still_commits_within_max_wait() {
        let t0 = Instant::now();
        let mut r = GridReconciler::new(coalesce());
        let _ = r.observe(grid(80, 24), t0);
        let mut committed_at = None;
        for i in 1..100u64 {
            let now = t0 + Duration::from_millis(i * 10);
            let g = grid(80 + (i % 2) as u16 + 1, 24);
            if r.observe(g, now).or_else(|| r.due(now)).is_some() {
                committed_at = Some(i * 10);
                break;
            }
        }
        assert_eq!(committed_at, Some(10 + 250));
    }

    #[test]
    fn a_coalesced_change_waits_for_the_quiet_period() {
        let t0 = Instant::now();
        let mut r = GridReconciler::new(coalesce());
        let _ = r.observe(grid(80, 24), t0);
        assert_eq!(r.observe(grid(81, 24), t0), None);
        assert_eq!(r.due(t0 + Duration::from_millis(39)), None);
        assert_eq!(r.deadline(), Some(t0 + Duration::from_millis(40)));
        assert_eq!(r.due(t0 + Duration::from_millis(40)), Some(grid(81, 24)));
    }

    #[test]
    fn returning_to_the_committed_grid_cancels_the_pending_one() {
        let t0 = Instant::now();
        let mut r = GridReconciler::new(coalesce());
        let _ = r.observe(grid(80, 24), t0);
        assert_eq!(r.observe(grid(81, 24), t0), None);
        assert_eq!(r.observe(grid(80, 24), t0 + Duration::from_millis(5)), None);
        assert_eq!(r.due(t0 + Duration::from_secs(1)), None);
        assert_eq!(r.committed(), Some(grid(80, 24)));
    }
}
