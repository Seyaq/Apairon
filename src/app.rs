//! Application lifecycle and the engine core.
//!
//! The [`Engine`] is a frame stepper: it never waits on its own. A *driver* calls
//! [`Engine::init`], then [`Engine::step`] repeatedly, waits for the returned [`Step::wait`]
//! between frames, and finally calls [`Engine::shutdown`]. [`Engine::run`] is the headless
//! driver; [`crate::shell::run_with_shell`] drives the engine alongside OS events.
//!
//! Lifecycle contract:
//! 1. [`Application::init`] runs once. If it fails, [`Application::shutdown`] is NOT called:
//!    `init` must release anything it partially acquired before returning an error.
//! 2. Frames run until the application requests exit, a callback fails, the driver stops, or
//!    the configured frame limit is reached.
//! 3. If `init` succeeded, [`Application::shutdown`] runs exactly once afterwards, including
//!    when a callback returned an error (drivers call [`Engine::shutdown`] on every path).
//!    A panic inside a callback unwinds without calling `shutdown`; the engine does not
//!    catch panics.
//!
//! Per frame: measure the real delta since the previous frame start, run the bounded number
//! of fixed steps, run the variable-rate frame callback, then report how long the driver
//! should wait until the next scheduled frame start (see [`FramePacer`]). The first frame has
//! a zero delta.
//!
//! Timing accounting: a frame is settled when the next frame starts (or at shutdown).
//! `frame` is start-to-start, `work` is the callbacks' duration and `wait` is the remainder:
//! the pacing wait including its oversleep and any driver overhead. The frames of a run
//! therefore add up to the elapsed run time.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::clock::Clock;
use crate::diagnostics::{FrameStats, FrameTiming};
use crate::pacing::FramePacer;
use crate::timestep::FixedStepper;

/// Default pacing target, in frames per second. A target, not a guarantee.
pub const DEFAULT_TARGET_FPS: u32 = 240;
/// Default simulation rate, in fixed steps per second.
pub const DEFAULT_FIXED_STEPS_PER_SECOND: u64 = 60;
/// Default bound on fixed steps per frame (catch-up policy).
pub const DEFAULT_MAX_FIXED_STEPS_PER_FRAME: u32 = 8;

/// Whether the loop should keep running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Keep running.
    Continue,
    /// Stop after the current frame.
    Exit,
}

/// Per-frame information passed to [`Application::frame`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameContext {
    /// Zero-based frame number.
    pub frame_index: u64,
    /// Measured time since the previous frame started (zero on the first frame).
    pub delta: Duration,
    /// Fraction of a fixed step left over, in `[0, 1)`, for render interpolation.
    pub interpolation_alpha: f64,
}

/// User code driven by the engine.
pub trait Application {
    /// Application error type.
    type Error: Error + 'static;

    /// Acquires resources. See the module docs for failure semantics.
    fn init(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Advances the simulation by exactly `step`.
    fn fixed_update(&mut self, _step: Duration) -> Result<Control, Self::Error> {
        Ok(Control::Continue)
    }

    /// Runs once per frame at the variable frame rate.
    fn frame(&mut self, ctx: &FrameContext) -> Result<Control, Self::Error>;

    /// Releases resources. Infallible by design; log failures inside.
    fn shutdown(&mut self) {}
}

/// Engine configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// Pacing target in frames per second; `None` disables pacing.
    pub target_fps: Option<u32>,
    /// Fixed simulation step.
    pub fixed_step: Duration,
    /// Maximum fixed steps per frame before time is dropped.
    pub max_fixed_steps_per_frame: u32,
    /// Stop after this many frames; `None` runs until the application exits.
    pub max_frames: Option<u64>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            target_fps: Some(DEFAULT_TARGET_FPS),
            fixed_step: Duration::from_nanos(1_000_000_000 / DEFAULT_FIXED_STEPS_PER_SECOND),
            max_fixed_steps_per_frame: DEFAULT_MAX_FIXED_STEPS_PER_FRAME,
            max_frames: None,
        }
    }
}

/// Invalid [`EngineConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// The target FPS is zero or too high to express as a nanosecond budget.
    InvalidTargetFps(u32),
    /// The fixed step is zero.
    ZeroFixedStep,
    /// The maximum number of fixed steps per frame is zero.
    ZeroMaxFixedSteps,
    /// The frame limit is zero.
    ZeroMaxFrames,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTargetFps(fps) => write!(f, "invalid target fps: {fps}"),
            Self::ZeroFixedStep => write!(f, "fixed step must be greater than zero"),
            Self::ZeroMaxFixedSteps => {
                write!(f, "max fixed steps per frame must be greater than zero")
            }
            Self::ZeroMaxFrames => write!(f, "max frames must be greater than zero"),
        }
    }
}

impl Error for ConfigError {}

/// Error returned by the engine, wrapping the application's own error.
#[derive(Debug)]
pub enum EngineError<E> {
    /// [`Application::init`] failed.
    Init(E),
    /// [`Application::fixed_update`] failed.
    FixedUpdate(E),
    /// [`Application::frame`] failed.
    Frame(E),
    /// [`Engine::step`] or [`Engine::init`] was called outside the running phase
    /// (before `init`, or after `shutdown`).
    NotRunning,
}

impl<E: Error> fmt::Display for EngineError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(e) => write!(f, "application init failed: {e}"),
            Self::FixedUpdate(e) => write!(f, "fixed update failed: {e}"),
            Self::Frame(e) => write!(f, "frame callback failed: {e}"),
            Self::NotRunning => write!(f, "engine is not running"),
        }
    }
}

impl<E: Error + 'static> Error for EngineError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Init(e) | Self::FixedUpdate(e) | Self::Frame(e) => Some(e),
            Self::NotRunning => None,
        }
    }
}

/// Why the loop stopped normally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    /// The application returned [`Control::Exit`].
    RequestedByApplication,
    /// [`EngineConfig::max_frames`] was reached.
    FrameLimitReached,
    /// The platform shell (the window) was closed by the user.
    ShellClosed,
}

/// Summary of a completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// Frame, work and wait statistics.
    pub stats: FrameStats,
    /// Total fixed steps executed.
    pub fixed_steps: u64,
    /// Total simulation time dropped by the catch-up bound.
    pub dropped_time: Duration,
    /// Why the loop stopped.
    pub exit_reason: ExitReason,
}

/// Outcome of one [`Engine::step`], telling the driver what to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// How long the driver should wait before stepping again. Zero when no wait is needed.
    ///
    /// Also set on the final step of a frame-limited run, so that the last frame is paced
    /// like the others. A driver stopping anyway may skip it.
    pub wait: Duration,
    /// `Some` when the driver must stop after this step.
    pub stop: Option<ExitReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Created,
    Running,
    Stopped,
}

/// A finished frame whose total duration is not known until the next frame starts.
#[derive(Debug, Clone, Copy)]
struct PendingFrame {
    start: Duration,
    work: Duration,
}

/// The engine core: a frame stepper with an explicit lifecycle.
#[derive(Debug)]
pub struct Engine {
    max_frames: Option<u64>,
    pacer: FramePacer,
    stepper: FixedStepper,
    phase: Phase,
    frame_index: u64,
    previous_frame: Option<PendingFrame>,
    stats: FrameStats,
    fixed_steps: u64,
    dropped_time: Duration,
}

impl Engine {
    /// Validates `config` and builds an engine.
    pub fn new(config: EngineConfig) -> Result<Self, ConfigError> {
        let pacer = match config.target_fps {
            None => FramePacer::unlimited(),
            Some(fps) => {
                FramePacer::from_target_fps(fps).ok_or(ConfigError::InvalidTargetFps(fps))?
            }
        };
        if config.fixed_step.is_zero() {
            return Err(ConfigError::ZeroFixedStep);
        }
        if config.max_fixed_steps_per_frame == 0 {
            return Err(ConfigError::ZeroMaxFixedSteps);
        }
        if config.max_frames == Some(0) {
            return Err(ConfigError::ZeroMaxFrames);
        }
        let stepper = FixedStepper::new(config.fixed_step, config.max_fixed_steps_per_frame)
            .ok_or(ConfigError::ZeroFixedStep)?;
        Ok(Self {
            max_frames: config.max_frames,
            pacer,
            stepper,
            phase: Phase::Created,
            frame_index: 0,
            previous_frame: None,
            stats: FrameStats::new(),
            fixed_steps: 0,
            dropped_time: Duration::ZERO,
        })
    }

    /// `true` between a successful [`Engine::init`] and [`Engine::shutdown`].
    pub fn is_running(&self) -> bool {
        self.phase == Phase::Running
    }

    /// Runs [`Application::init`]. A no-op if the engine is already running; an error if it
    /// was already shut down. If the application's `init` fails, the engine stays in its
    /// initial phase and [`Engine::shutdown`] will not call [`Application::shutdown`].
    pub fn init<A: Application>(&mut self, app: &mut A) -> Result<(), EngineError<A::Error>> {
        match self.phase {
            Phase::Running => Ok(()),
            Phase::Stopped => Err(EngineError::NotRunning),
            Phase::Created => {
                app.init().map_err(EngineError::Init)?;
                self.phase = Phase::Running;
                Ok(())
            }
        }
    }

    /// Runs one frame without waiting and tells the driver what to do next.
    ///
    /// Returns [`EngineError::NotRunning`] outside the running phase. After an error the
    /// driver must still call [`Engine::shutdown`].
    pub fn step<A, C>(&mut self, app: &mut A, clock: &C) -> Result<Step, EngineError<A::Error>>
    where
        A: Application,
        C: Clock,
    {
        if self.phase != Phase::Running {
            return Err(EngineError::NotRunning);
        }

        let frame_start = clock.now();
        let delta = self.settle_previous_frame(frame_start);

        let plan = self.stepper.advance(delta);
        self.dropped_time = self.dropped_time.saturating_add(plan.dropped);

        let mut exit_requested = false;
        for _ in 0..plan.steps {
            self.fixed_steps = self.fixed_steps.saturating_add(1);
            let control = app
                .fixed_update(self.stepper.step())
                .map_err(EngineError::FixedUpdate)?;
            if control == Control::Exit {
                exit_requested = true;
                break;
            }
        }

        if !exit_requested {
            let ctx = FrameContext {
                frame_index: self.frame_index,
                delta,
                interpolation_alpha: self.stepper.alpha(),
            };
            exit_requested = app.frame(&ctx).map_err(EngineError::Frame)? == Control::Exit;
        }

        let work_end = clock.now();
        self.previous_frame = Some(PendingFrame {
            start: frame_start,
            work: work_end.saturating_sub(frame_start),
        });
        self.frame_index = self.frame_index.saturating_add(1);

        // No point waiting when the loop is about to stop.
        if exit_requested {
            return Ok(Step {
                wait: Duration::ZERO,
                stop: Some(ExitReason::RequestedByApplication),
            });
        }

        let wait = self.pacer.plan_wait(frame_start, work_end);
        let limit_reached = self.max_frames.is_some_and(|max| self.frame_index >= max);
        Ok(Step {
            wait,
            stop: limit_reached.then_some(ExitReason::FrameLimitReached),
        })
    }

    /// Settles the last frame at the current time and runs [`Application::shutdown`] if the
    /// engine is running. A no-op otherwise, so drivers can call it on every path.
    pub fn shutdown<A, C>(&mut self, app: &mut A, clock: &C)
    where
        A: Application,
        C: Clock,
    {
        if self.phase != Phase::Running {
            return;
        }
        self.settle_previous_frame(clock.now());
        app.shutdown();
        self.phase = Phase::Stopped;
    }

    /// Builds a report from the statistics gathered so far.
    pub fn report(&self, exit_reason: ExitReason) -> RunReport {
        RunReport {
            stats: self.stats.clone(),
            fixed_steps: self.fixed_steps,
            dropped_time: self.dropped_time,
            exit_reason,
        }
    }

    /// Headless driver: runs the application to completion, sleeping on `clock` between
    /// frames. On error, the partial statistics are discarded.
    pub fn run<A, C>(mut self, app: &mut A, clock: &C) -> Result<RunReport, EngineError<A::Error>>
    where
        A: Application,
        C: Clock,
    {
        self.init(app)?;
        let outcome = self.drive(app, clock);
        self.shutdown(app, clock);
        let exit_reason = outcome?;
        Ok(self.report(exit_reason))
    }

    fn drive<A, C>(&mut self, app: &mut A, clock: &C) -> Result<ExitReason, EngineError<A::Error>>
    where
        A: Application,
        C: Clock,
    {
        loop {
            let step = self.step(app, clock)?;
            if !step.wait.is_zero() {
                clock.sleep(step.wait);
            }
            if let Some(reason) = step.stop {
                return Ok(reason);
            }
        }
    }

    /// Records the previous frame, if any, now that its end is known. Returns its duration,
    /// which is the delta of the frame starting at `end`.
    fn settle_previous_frame(&mut self, end: Duration) -> Duration {
        let Some(previous) = self.previous_frame.take() else {
            return Duration::ZERO;
        };
        let frame = end.saturating_sub(previous.start);
        self.stats.record(FrameTiming {
            frame,
            work: previous.work.min(frame),
            wait: frame.saturating_sub(previous.work),
        });
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;

    #[derive(Debug)]
    struct TestError;

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "test error")
        }
    }

    impl Error for TestError {}

    struct TestApp<'a> {
        clock: &'a ManualClock,
        work_per_frame: Duration,
        exit_at_frame: Option<u64>,
        fail_init: bool,
        fail_frame_at: Option<u64>,
        events: Vec<&'static str>,
        deltas: Vec<Duration>,
        fixed_calls: u64,
    }

    impl<'a> TestApp<'a> {
        fn new(clock: &'a ManualClock, work_per_frame: Duration) -> Self {
            Self {
                clock,
                work_per_frame,
                exit_at_frame: None,
                fail_init: false,
                fail_frame_at: None,
                events: Vec::new(),
                deltas: Vec::new(),
                fixed_calls: 0,
            }
        }
    }

    impl Application for TestApp<'_> {
        type Error = TestError;

        fn init(&mut self) -> Result<(), TestError> {
            self.events.push("init");
            if self.fail_init {
                Err(TestError)
            } else {
                Ok(())
            }
        }

        fn fixed_update(&mut self, _step: Duration) -> Result<Control, TestError> {
            self.fixed_calls += 1;
            Ok(Control::Continue)
        }

        fn frame(&mut self, ctx: &FrameContext) -> Result<Control, TestError> {
            self.clock.advance(self.work_per_frame);
            self.deltas.push(ctx.delta);
            if self.fail_frame_at == Some(ctx.frame_index) {
                return Err(TestError);
            }
            if self.exit_at_frame == Some(ctx.frame_index) {
                return Ok(Control::Exit);
            }
            Ok(Control::Continue)
        }

        fn shutdown(&mut self) {
            self.events.push("shutdown");
        }
    }

    fn config(
        target_fps: Option<u32>,
        step_ms: u64,
        max_steps: u32,
        max_frames: u64,
    ) -> EngineConfig {
        EngineConfig {
            target_fps,
            fixed_step: Duration::from_millis(step_ms),
            max_fixed_steps_per_frame: max_steps,
            max_frames: Some(max_frames),
        }
    }

    #[test]
    fn paced_frames_wait_for_the_remaining_budget() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(3));
        let engine = Engine::new(config(Some(100), 10, 8, 5)).expect("valid config");

        let report = engine.run(&mut app, &clock).expect("run succeeds");

        assert_eq!(report.exit_reason, ExitReason::FrameLimitReached);
        assert_eq!(report.stats.frames(), 5);
        assert_eq!(report.stats.last().work, Duration::from_millis(3));
        assert_eq!(report.stats.last().wait, Duration::from_millis(7));
        assert_eq!(report.stats.last().frame, Duration::from_millis(10));
        let ten = Duration::from_millis(10);
        assert_eq!(app.deltas, vec![Duration::ZERO, ten, ten, ten, ten]);
        assert_eq!(report.fixed_steps, 4);
        assert_eq!(app.fixed_calls, 4);
    }

    #[test]
    fn overrunning_frames_do_not_wait_and_delta_is_measured() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(20));
        let engine = Engine::new(config(Some(100), 10, 8, 3)).expect("valid config");

        let report = engine.run(&mut app, &clock).expect("run succeeds");

        let twenty = Duration::from_millis(20);
        assert_eq!(app.deltas, vec![Duration::ZERO, twenty, twenty]);
        assert_eq!(report.stats.last().wait, Duration::ZERO);
        assert_eq!(report.fixed_steps, 4);
        assert_eq!(report.dropped_time, Duration::ZERO);
    }

    #[test]
    fn catch_up_is_bounded_and_dropped_time_is_reported() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(100));
        let engine = Engine::new(config(None, 10, 3, 3)).expect("valid config");

        let report = engine.run(&mut app, &clock).expect("run succeeds");

        assert_eq!(report.fixed_steps, 6);
        assert_eq!(report.dropped_time, Duration::from_millis(140));
    }

    #[test]
    fn exit_request_runs_shutdown_and_stops_without_waiting() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(3));
        app.exit_at_frame = Some(2);
        let engine = Engine::new(config(Some(100), 10, 8, 100)).expect("valid config");

        let report = engine.run(&mut app, &clock).expect("run succeeds");

        assert_eq!(report.exit_reason, ExitReason::RequestedByApplication);
        assert_eq!(report.stats.frames(), 3);
        assert_eq!(report.stats.last().wait, Duration::ZERO);
        assert_eq!(app.events, vec!["init", "shutdown"]);
    }

    #[test]
    fn frame_error_still_runs_shutdown() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(1));
        app.fail_frame_at = Some(1);
        let engine = Engine::new(config(None, 10, 8, 10)).expect("valid config");

        let result = engine.run(&mut app, &clock);

        assert!(matches!(result, Err(EngineError::Frame(_))));
        assert_eq!(app.events, vec!["init", "shutdown"]);
    }

    #[test]
    fn init_error_skips_shutdown() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(1));
        app.fail_init = true;
        let engine = Engine::new(config(None, 10, 8, 10)).expect("valid config");

        let result = engine.run(&mut app, &clock);

        assert!(matches!(result, Err(EngineError::Init(_))));
        assert_eq!(app.events, vec!["init"]);
    }

    #[test]
    fn invalid_configs_are_rejected() {
        let valid = config(Some(240), 10, 8, 1);

        let cases = [
            (
                EngineConfig {
                    target_fps: Some(0),
                    ..valid.clone()
                },
                ConfigError::InvalidTargetFps(0),
            ),
            (
                EngineConfig {
                    fixed_step: Duration::ZERO,
                    ..valid.clone()
                },
                ConfigError::ZeroFixedStep,
            ),
            (
                EngineConfig {
                    max_fixed_steps_per_frame: 0,
                    ..valid.clone()
                },
                ConfigError::ZeroMaxFixedSteps,
            ),
            (
                EngineConfig {
                    max_frames: Some(0),
                    ..valid
                },
                ConfigError::ZeroMaxFrames,
            ),
        ];

        for (bad, expected) in cases {
            assert_eq!(Engine::new(bad).err(), Some(expected));
        }
    }

    #[test]
    fn default_config_is_valid() {
        assert!(Engine::new(EngineConfig::default()).is_ok());
    }

    /// Clock whose sleeps always wake up late, like a real OS timer.
    struct LateSleepClock {
        inner: ManualClock,
        overshoot: Duration,
    }

    impl LateSleepClock {
        fn new(overshoot: Duration) -> Self {
            Self {
                inner: ManualClock::new(),
                overshoot,
            }
        }
    }

    impl Clock for LateSleepClock {
        fn now(&self) -> Duration {
            self.inner.now()
        }

        fn sleep(&self, duration: Duration) {
            self.inner.advance(duration.saturating_add(self.overshoot));
        }
    }

    #[test]
    fn oversleep_is_compensated_so_frames_stay_on_budget() {
        let clock = LateSleepClock::new(Duration::from_micros(250));
        let mut app = TestApp::new(&clock.inner, Duration::from_millis(3));
        let engine = Engine::new(config(Some(100), 10, 8, 100)).expect("valid config");

        let report = engine.run(&mut app, &clock).expect("run succeeds");

        // Only the first frame pays for the oversleep; later frames are exactly on budget.
        let ten = Duration::from_millis(10);
        assert_eq!(app.deltas[0], Duration::ZERO);
        assert_eq!(app.deltas[1], Duration::from_micros(10_250));
        assert!(app.deltas[2..].iter().all(|delta| *delta == ten));
        assert_eq!(clock.now(), Duration::from_micros(1_000_250));
        assert_eq!(report.stats.max_frame(), Duration::from_micros(10_250));
        assert_eq!(
            report.stats.average_frame(),
            Some(Duration::from_nanos(10_002_500))
        );
        let last = report.stats.last();
        assert_eq!(last.frame, ten);
        assert_eq!(last.frame, last.work + last.wait);
    }

    #[test]
    fn fixed_step_count_matches_total_measured_time_without_drops() {
        let clock = LateSleepClock::new(Duration::from_nanos(257_724));
        let mut app = TestApp::new(&clock.inner, Duration::from_micros(2));
        let step = Duration::from_nanos(16_666_666);
        let config = EngineConfig {
            target_fps: Some(240),
            fixed_step: step,
            max_fixed_steps_per_frame: 8,
            max_frames: Some(480),
        };

        let report = Engine::new(config)
            .expect("valid config")
            .run(&mut app, &clock)
            .expect("run succeeds");

        let total_delta_nanos: u128 = app.deltas.iter().map(Duration::as_nanos).sum();
        assert_eq!(
            u128::from(report.fixed_steps),
            total_delta_nanos / step.as_nanos()
        );
        assert_eq!(app.fixed_calls, report.fixed_steps);
        assert_eq!(report.dropped_time, Duration::ZERO);

        // Compensated pacing stays within 1 us of the 4_166_666 ns budget on average.
        let average = report.stats.average_frame().expect("frames were recorded");
        assert!(average >= Duration::from_nanos(4_166_666));
        assert!(average <= Duration::from_nanos(4_167_666));
    }

    #[test]
    fn step_before_init_is_rejected_and_shutdown_is_a_no_op() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(1));
        let mut engine = Engine::new(config(None, 10, 8, 10)).expect("valid config");

        assert!(matches!(
            engine.step(&mut app, &clock),
            Err(EngineError::NotRunning)
        ));
        engine.shutdown(&mut app, &clock);

        assert!(app.events.is_empty());
        assert!(!engine.is_running());
    }

    #[test]
    fn init_is_idempotent_and_shutdown_settles_the_last_frame() {
        let clock = ManualClock::new();
        let mut app = TestApp::new(&clock, Duration::from_millis(3));
        let mut engine = Engine::new(config(None, 10, 8, 10)).expect("valid config");

        engine.init(&mut app).expect("init succeeds");
        engine.init(&mut app).expect("second init is a no-op");
        assert!(engine.is_running());
        engine.step(&mut app, &clock).expect("first step");
        engine.step(&mut app, &clock).expect("second step");
        engine.shutdown(&mut app, &clock);
        engine.shutdown(&mut app, &clock);

        assert_eq!(app.events, vec!["init", "shutdown"]);
        let report = engine.report(ExitReason::FrameLimitReached);
        assert_eq!(report.stats.frames(), 2);
        assert!(matches!(
            engine.init(&mut app),
            Err(EngineError::NotRunning)
        ));
    }
}
