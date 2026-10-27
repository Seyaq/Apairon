//! Fixed-timestep simulation stepping.
//!
//! Simulation time is kept distinct from frame time: each frame's measured delta is added to
//! an accumulator and consumed in whole fixed steps. To avoid the spiral of death, at most
//! `max_steps_per_frame` steps run per frame; time beyond that is dropped (and reported)
//! rather than carried over.

use std::time::Duration;

use crate::util::duration_from_nanos;

/// Result of feeding one frame delta to a [`FixedStepper`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepPlan {
    /// Number of fixed steps to simulate this frame.
    pub steps: u32,
    /// Simulation time discarded because of the catch-up bound.
    pub dropped: Duration,
}

/// Accumulator-based fixed stepper.
#[derive(Debug, Clone)]
pub struct FixedStepper {
    step_nanos: u128,
    max_steps_per_frame: u32,
    accumulator_nanos: u128,
}

impl FixedStepper {
    /// Returns `None` if `step` is zero or `max_steps_per_frame` is zero.
    pub fn new(step: Duration, max_steps_per_frame: u32) -> Option<Self> {
        if step.is_zero() || max_steps_per_frame == 0 {
            return None;
        }
        Some(Self {
            step_nanos: step.as_nanos(),
            max_steps_per_frame,
            accumulator_nanos: 0,
        })
    }

    /// The fixed step duration.
    pub fn step(&self) -> Duration {
        duration_from_nanos(self.step_nanos)
    }

    /// Adds a measured frame delta and returns how many fixed steps to run.
    pub fn advance(&mut self, delta: Duration) -> StepPlan {
        self.accumulator_nanos = self.accumulator_nanos.saturating_add(delta.as_nanos());

        let available = self.accumulator_nanos / self.step_nanos;
        let steps = available.min(u128::from(self.max_steps_per_frame));
        let dropped_steps = available - steps;

        // steps + dropped_steps == available, and available * step <= accumulator.
        self.accumulator_nanos -= available * self.step_nanos;

        StepPlan {
            steps: u32::try_from(steps).unwrap_or(self.max_steps_per_frame),
            dropped: duration_from_nanos(dropped_steps * self.step_nanos),
        }
    }

    /// Fraction of a step left in the accumulator, in `[0, 1)`, for render interpolation.
    pub fn alpha(&self) -> f64 {
        self.accumulator_nanos as f64 / self.step_nanos as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stepper(step_ms: u64, max_steps: u32) -> FixedStepper {
        FixedStepper::new(Duration::from_millis(step_ms), max_steps).expect("valid stepper")
    }

    #[test]
    fn consumes_whole_steps_and_keeps_remainder() {
        let mut s = stepper(10, 5);
        let plan = s.advance(Duration::from_millis(25));
        assert_eq!(plan.steps, 2);
        assert_eq!(plan.dropped, Duration::ZERO);
        assert!((s.alpha() - 0.5).abs() < f64::EPSILON);

        let plan = s.advance(Duration::from_millis(5));
        assert_eq!(plan.steps, 1);
        assert!(s.alpha().abs() < f64::EPSILON);
    }

    #[test]
    fn catch_up_is_bounded_and_excess_is_dropped() {
        let mut s = stepper(10, 5);
        let plan = s.advance(Duration::from_secs(1));
        assert_eq!(plan.steps, 5);
        assert_eq!(plan.dropped, Duration::from_millis(950));
        assert!(s.alpha().abs() < f64::EPSILON);
    }

    #[test]
    fn zero_delta_yields_no_steps() {
        let mut s = stepper(10, 5);
        assert_eq!(s.advance(Duration::ZERO).steps, 0);
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        assert!(FixedStepper::new(Duration::ZERO, 5).is_none());
        assert!(FixedStepper::new(Duration::from_millis(10), 0).is_none());
    }
}
