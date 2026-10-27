//! Time sources.
//!
//! All engine timing goes through [`Clock`] so that pacing and stepping logic can be tested
//! deterministically without real sleeps.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// A monotonic time source that can also block the current thread.
pub trait Clock {
    /// Time elapsed since this clock's epoch. Never decreases.
    fn now(&self) -> Duration;

    /// Blocks for approximately `duration`. Real implementations may oversleep because of
    /// OS timer granularity and scheduling; callers must measure with [`Clock::now`].
    fn sleep(&self, duration: Duration);
}

/// Real clock backed by [`Instant`] and [`std::thread::sleep`].
#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    epoch: Instant,
}

impl MonotonicClock {
    /// Creates a clock whose epoch is the moment of the call.
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Deterministic clock for tests: time only moves via [`ManualClock::advance`] or `sleep`,
/// and `sleep` is exact.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Cell<Duration>,
}

impl ManualClock {
    /// Creates a clock at time zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves time forward, e.g. to simulate work done by the application.
    pub fn advance(&self, duration: Duration) {
        self.now.set(self.now.get().saturating_add(duration));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        self.now.get()
    }

    fn sleep(&self, duration: Duration) {
        self.advance(duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_sleep_advances_time_exactly() {
        let clock = ManualClock::new();
        clock.advance(Duration::from_millis(3));
        clock.sleep(Duration::from_millis(7));
        assert_eq!(clock.now(), Duration::from_millis(10));
    }

    #[test]
    fn monotonic_clock_never_goes_backwards() {
        let clock = MonotonicClock::new();
        let a = clock.now();
        let b = clock.now();
        assert!(b >= a);
    }
}
