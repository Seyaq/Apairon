//! Frame pacing.
//!
//! A target frame rate is a pacing target, not a guarantee. The pacer keeps a schedule of
//! frame start times (`start, start + T, start + 2T, ...`) instead of waiting "T minus work"
//! after every frame. A sleep that wakes up late is therefore absorbed by shortening the next
//! wait, so the long-run average frame time converges to `T` instead of drifting to
//! `T + mean oversleep`.
//!
//! Trade-offs, by design:
//! - Individual frames after a late wake-up are shorter than `T`: the target is enforced on
//!   average, not per frame.
//! - When the work itself overruns its slot, the schedule is re-anchored to the current time
//!   and no wait happens. The pacer never tries to catch up with a burst of short frames.
//! - The pacer only computes durations. Waiting is done with a sleep (no busy-wait), so the
//!   achieved frame time still jitters by the OS timer granularity and scheduler latency.

use std::time::Duration;

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Computes how long a frame should wait after finishing its work.
///
/// Stateful (it owns the schedule), hence not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramePacer {
    target_frame_time: Option<Duration>,
    /// Scheduled start of the frame being processed; `None` before the first frame.
    scheduled_frame_start: Option<Duration>,
}

impl FramePacer {
    /// A pacer that never waits.
    pub const fn unlimited() -> Self {
        Self {
            target_frame_time: None,
            scheduled_frame_start: None,
        }
    }

    /// A pacer targeting an explicit frame duration.
    pub const fn with_target_frame_time(target_frame_time: Duration) -> Self {
        Self {
            target_frame_time: Some(target_frame_time),
            scheduled_frame_start: None,
        }
    }

    /// A pacer targeting `fps` frames per second (integer nanosecond budget).
    ///
    /// Returns `None` if `fps` is zero or so high that the budget rounds to zero nanoseconds.
    pub fn from_target_fps(fps: u32) -> Option<Self> {
        if fps == 0 {
            return None;
        }
        let nanos = NANOS_PER_SECOND / u64::from(fps);
        if nanos == 0 {
            return None;
        }
        Some(Self::with_target_frame_time(Duration::from_nanos(nanos)))
    }

    /// The target frame duration, if any.
    pub const fn target_frame_time(&self) -> Option<Duration> {
        self.target_frame_time
    }

    /// Returns how long to wait after a frame's work finished at `now`.
    ///
    /// `frame_start` is only used to anchor the schedule on the first call. Call exactly once
    /// per frame, after the work and before sleeping.
    pub fn plan_wait(&mut self, frame_start: Duration, now: Duration) -> Duration {
        let Some(target) = self.target_frame_time else {
            return Duration::ZERO;
        };

        let current = self.scheduled_frame_start.unwrap_or(frame_start);
        let next = current.saturating_add(target);

        if now >= next {
            // The work overran its slot: re-anchor instead of catching up.
            self.scheduled_frame_start = Some(now);
            Duration::ZERO
        } else {
            self.scheduled_frame_start = Some(next);
            next - now
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEN_MS: Duration = Duration::from_millis(10);

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn target_240_fps_budget_is_integer_nanoseconds() {
        let pacer = FramePacer::from_target_fps(240).expect("240 fps is valid");
        assert_eq!(
            pacer.target_frame_time(),
            Some(Duration::from_nanos(4_166_666))
        );
    }

    #[test]
    fn first_call_anchors_the_schedule_to_the_frame_start() {
        let mut pacer = FramePacer::with_target_frame_time(TEN_MS);
        assert_eq!(pacer.plan_wait(ms(100), ms(103)), ms(7));
    }

    #[test]
    fn a_late_wake_up_shortens_the_next_wait() {
        let mut pacer = FramePacer::with_target_frame_time(TEN_MS);
        assert_eq!(pacer.plan_wait(ms(0), ms(3)), ms(7));

        // The sleep woke 250 us late: the next frame starts at 10.25 ms.
        let frame_start = Duration::from_micros(10_250);
        let work_end = Duration::from_micros(13_250);
        assert_eq!(
            pacer.plan_wait(frame_start, work_end),
            Duration::from_micros(6_750)
        );
    }

    #[test]
    fn an_overrun_re_anchors_the_schedule_without_a_catch_up_burst() {
        let mut pacer = FramePacer::with_target_frame_time(TEN_MS);
        assert_eq!(pacer.plan_wait(ms(0), ms(3)), ms(7));
        assert_eq!(pacer.plan_wait(ms(10), ms(13)), ms(7));

        // A 35 ms hitch: no wait, and the schedule restarts from the current time.
        assert_eq!(pacer.plan_wait(ms(20), ms(55)), Duration::ZERO);
        assert_eq!(pacer.plan_wait(ms(55), ms(58)), ms(7));
    }

    #[test]
    fn finishing_exactly_on_the_deadline_does_not_wait() {
        let mut pacer = FramePacer::with_target_frame_time(TEN_MS);
        assert_eq!(pacer.plan_wait(ms(0), ms(3)), ms(7));
        assert_eq!(pacer.plan_wait(ms(10), ms(20)), Duration::ZERO);
        assert_eq!(pacer.plan_wait(ms(20), ms(23)), ms(7));
    }

    #[test]
    fn unlimited_never_waits() {
        let mut pacer = FramePacer::unlimited();
        assert_eq!(pacer.plan_wait(Duration::ZERO, Duration::ZERO), Duration::ZERO);
        assert_eq!(pacer.plan_wait(ms(5), ms(50)), Duration::ZERO);
    }

    #[test]
    fn invalid_fps_is_rejected() {
        assert_eq!(FramePacer::from_target_fps(0), None);
        assert_eq!(FramePacer::from_target_fps(2_000_000_000), None);
    }
}
