//! Frame diagnostics.
//!
//! Total frame duration, useful work duration and waiting duration are tracked separately.
//! GPU time is not measured here: there is no renderer yet.

use std::time::Duration;

use crate::util::duration_from_nanos;

/// Timing of one frame. By construction `frame == work + wait`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameTiming {
    /// Total frame duration (work plus wait).
    pub frame: Duration,
    /// Time spent in simulation and frame callbacks.
    pub work: Duration,
    /// Rest of the frame: the pacing wait including its oversleep, plus any driver overhead
    /// (for example event pumping) before the next frame starts.
    pub wait: Duration,
}

/// Aggregated frame timings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameStats {
    frames: u64,
    last: FrameTiming,
    max_frame: Duration,
    total_frame: Duration,
    total_work: Duration,
    total_wait: Duration,
}

impl FrameStats {
    /// Creates empty statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one finished frame.
    pub fn record(&mut self, timing: FrameTiming) {
        self.frames = self.frames.saturating_add(1);
        self.last = timing;
        self.max_frame = self.max_frame.max(timing.frame);
        self.total_frame = self.total_frame.saturating_add(timing.frame);
        self.total_work = self.total_work.saturating_add(timing.work);
        self.total_wait = self.total_wait.saturating_add(timing.wait);
    }

    /// Number of recorded frames.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Timing of the most recent frame (zeros if none).
    pub fn last(&self) -> FrameTiming {
        self.last
    }

    /// Longest recorded frame.
    pub fn max_frame(&self) -> Duration {
        self.max_frame
    }

    /// Mean total frame duration, or `None` if no frame was recorded.
    pub fn average_frame(&self) -> Option<Duration> {
        self.average(self.total_frame)
    }

    /// Mean work duration, or `None` if no frame was recorded.
    pub fn average_work(&self) -> Option<Duration> {
        self.average(self.total_work)
    }

    /// Mean wait duration, or `None` if no frame was recorded.
    pub fn average_wait(&self) -> Option<Duration> {
        self.average(self.total_wait)
    }

    fn average(&self, total: Duration) -> Option<Duration> {
        if self.frames == 0 {
            return None;
        }
        Some(duration_from_nanos(
            total.as_nanos() / u128::from(self.frames),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(work_ms: u64, wait_ms: u64) -> FrameTiming {
        let work = Duration::from_millis(work_ms);
        let wait = Duration::from_millis(wait_ms);
        FrameTiming {
            frame: work + wait,
            work,
            wait,
        }
    }

    #[test]
    fn empty_stats_have_no_averages() {
        let stats = FrameStats::new();
        assert_eq!(stats.frames(), 0);
        assert_eq!(stats.average_frame(), None);
        assert_eq!(stats.average_work(), None);
        assert_eq!(stats.average_wait(), None);
    }

    #[test]
    fn averages_and_max_are_tracked_separately() {
        let mut stats = FrameStats::new();
        stats.record(timing(2, 8));
        stats.record(timing(4, 6));
        stats.record(timing(12, 0));

        assert_eq!(stats.frames(), 3);
        assert_eq!(stats.last(), timing(12, 0));
        assert_eq!(stats.max_frame(), Duration::from_millis(12));
        assert_eq!(
            stats.average_frame(),
            Some(Duration::from_nanos(10_666_666))
        );
        assert_eq!(stats.average_work(), Some(Duration::from_millis(6)));
        assert_eq!(stats.average_wait(), Some(Duration::from_nanos(4_666_666)));
    }
}
