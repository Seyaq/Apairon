//! Platform shell integration.
//!
//! A *shell* is whatever owns the OS surface the engine runs in (typically a window). The
//! engine only needs three things from it, expressed by [`EventPump`]: process pending OS
//! events, say whether a surface is ready, and say whether the user asked to close.
//!
//! [`run_with_shell`] drives an [`Engine`] alongside an [`EventPump`]:
//! - frames only run while the shell reports [`ShellState::Ready`];
//! - [`Application::init`] runs on the first `Ready`, so a surface exists when it is called;
//! - events are pumped without blocking between frames, and the thread sleeps on the
//!   [`Clock`] to honor the pacing wait (no busy-wait);
//! - [`Application::shutdown`] runs on every exit path once `init` has succeeded.
//!
//! This module has no platform dependency, so the driver logic is tested with a scripted
//! pump. The winit-backed implementation lives in the optional `window` module.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use crate::app::{Application, Engine, EngineError, ExitReason, RunReport};
use crate::clock::Clock;

/// How long to block waiting for events while no surface is ready, so that waiting for the
/// window does not spin.
pub const WAIT_FOR_SURFACE: Duration = Duration::from_millis(10);

/// State of the shell after pumping events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellState {
    /// No surface is available right now (not created yet, or suspended): do not run frames.
    Waiting,
    /// A surface is available: frames may run.
    Ready,
    /// The user asked to close the shell, or the OS ended the event loop.
    Closed,
}

/// Source of OS events.
pub trait EventPump {
    /// Shell error type (window or event-loop creation failures, for example).
    type Error: Error + 'static;

    /// Processes pending OS events, blocking for at most `timeout`, and reports the state.
    fn pump(&mut self, timeout: Duration) -> Result<ShellState, Self::Error>;
}

/// Error returned by [`run_with_shell`].
#[derive(Debug)]
pub enum ShellError<P, E> {
    /// The shell failed.
    Pump(P),
    /// The engine or the application failed.
    Engine(EngineError<E>),
}

impl<P: Error, E: Error> fmt::Display for ShellError<P, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pump(e) => write!(f, "platform shell failed: {e}"),
            Self::Engine(e) => write!(f, "{e}"),
        }
    }
}

impl<P: Error + 'static, E: Error + 'static> Error for ShellError<P, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Pump(e) => Some(e),
            Self::Engine(e) => Some(e),
        }
    }
}

/// Runs `engine` and `app` alongside `pump` until a stop condition is reached.
///
/// On error, partial statistics are discarded, as with [`Engine::run`].
pub fn run_with_shell<A, C, P>(
    mut engine: Engine,
    app: &mut A,
    clock: &C,
    pump: &mut P,
) -> Result<RunReport, ShellError<P::Error, A::Error>>
where
    A: Application,
    C: Clock,
    P: EventPump,
{
    let outcome = drive(&mut engine, app, clock, pump);
    engine.shutdown(app, clock);
    let exit_reason = outcome?;
    Ok(engine.report(exit_reason))
}

fn drive<A, C, P>(
    engine: &mut Engine,
    app: &mut A,
    clock: &C,
    pump: &mut P,
) -> Result<ExitReason, ShellError<P::Error, A::Error>>
where
    A: Application,
    C: Clock,
    P: EventPump,
{
    let mut state = ShellState::Waiting;
    loop {
        let timeout = if state == ShellState::Ready {
            Duration::ZERO
        } else {
            WAIT_FOR_SURFACE
        };
        state = pump.pump(timeout).map_err(ShellError::Pump)?;

        match state {
            ShellState::Closed => return Ok(ExitReason::ShellClosed),
            ShellState::Waiting => continue,
            ShellState::Ready => {}
        }

        engine.init(app).map_err(ShellError::Engine)?;
        let step = engine.step(app, clock).map_err(ShellError::Engine)?;
        if !step.wait.is_zero() {
            clock.sleep(step.wait);
        }
        if let Some(reason) = step.stop {
            return Ok(reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::app::{Control, EngineConfig, FrameContext};
    use crate::clock::ManualClock;

    #[derive(Debug)]
    struct TestError;

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "test error")
        }
    }

    impl Error for TestError {}

    struct ScriptedPump {
        script: VecDeque<Result<ShellState, TestError>>,
        timeouts: Vec<Duration>,
    }

    impl ScriptedPump {
        fn new(script: Vec<Result<ShellState, TestError>>) -> Self {
            Self {
                script: VecDeque::from(script),
                timeouts: Vec::new(),
            }
        }
    }

    impl EventPump for ScriptedPump {
        type Error = TestError;

        fn pump(&mut self, timeout: Duration) -> Result<ShellState, TestError> {
            self.timeouts.push(timeout);
            self.script.pop_front().unwrap_or(Ok(ShellState::Closed))
        }
    }

    struct RecordingApp<'a> {
        clock: &'a ManualClock,
        work: Duration,
        events: Vec<&'static str>,
        frames: u64,
        exit_at_frame: Option<u64>,
        fail_frame: bool,
    }

    impl<'a> RecordingApp<'a> {
        fn new(clock: &'a ManualClock) -> Self {
            Self {
                clock,
                work: Duration::from_millis(3),
                events: Vec::new(),
                frames: 0,
                exit_at_frame: None,
                fail_frame: false,
            }
        }
    }

    impl Application for RecordingApp<'_> {
        type Error = TestError;

        fn init(&mut self) -> Result<(), TestError> {
            self.events.push("init");
            Ok(())
        }

        fn frame(&mut self, ctx: &FrameContext) -> Result<Control, TestError> {
            self.clock.advance(self.work);
            self.frames += 1;
            if self.fail_frame {
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

    fn engine() -> Engine {
        Engine::new(EngineConfig {
            target_fps: Some(100),
            fixed_step: Duration::from_millis(10),
            max_fixed_steps_per_frame: 8,
            max_frames: None,
        })
        .expect("valid config")
    }

    #[test]
    fn frames_only_run_once_a_surface_is_ready() {
        let clock = ManualClock::new();
        let mut app = RecordingApp::new(&clock);
        let mut pump = ScriptedPump::new(vec![
            Ok(ShellState::Waiting),
            Ok(ShellState::Waiting),
            Ok(ShellState::Ready),
            Ok(ShellState::Ready),
            Ok(ShellState::Closed),
        ]);

        let report = run_with_shell(engine(), &mut app, &clock, &mut pump).expect("run succeeds");

        assert_eq!(report.exit_reason, ExitReason::ShellClosed);
        assert_eq!(app.events, vec!["init", "shutdown"]);
        assert_eq!(app.frames, 2);
        assert_eq!(report.stats.frames(), 2);
        assert_eq!(
            pump.timeouts,
            vec![
                WAIT_FOR_SURFACE,
                WAIT_FOR_SURFACE,
                WAIT_FOR_SURFACE,
                Duration::ZERO,
                Duration::ZERO
            ]
        );
        let last = report.stats.last();
        assert_eq!(last.frame, Duration::from_millis(10));
        assert_eq!(last.work, Duration::from_millis(3));
        assert_eq!(last.wait, Duration::from_millis(7));
    }

    #[test]
    fn a_suspended_surface_pauses_frames_without_spinning() {
        let clock = ManualClock::new();
        let mut app = RecordingApp::new(&clock);
        let mut pump = ScriptedPump::new(vec![
            Ok(ShellState::Ready),
            Ok(ShellState::Waiting),
            Ok(ShellState::Ready),
            Ok(ShellState::Closed),
        ]);

        let report = run_with_shell(engine(), &mut app, &clock, &mut pump).expect("run succeeds");

        assert_eq!(report.stats.frames(), 2);
        assert_eq!(
            pump.timeouts,
            vec![
                WAIT_FOR_SURFACE,
                Duration::ZERO,
                WAIT_FOR_SURFACE,
                Duration::ZERO
            ]
        );
    }

    #[test]
    fn pump_error_after_init_still_runs_shutdown() {
        let clock = ManualClock::new();
        let mut app = RecordingApp::new(&clock);
        let mut pump = ScriptedPump::new(vec![Ok(ShellState::Ready), Err(TestError)]);

        let result = run_with_shell(engine(), &mut app, &clock, &mut pump);

        assert!(matches!(result, Err(ShellError::Pump(_))));
        assert_eq!(app.events, vec!["init", "shutdown"]);
    }

    #[test]
    fn pump_error_before_init_skips_shutdown() {
        let clock = ManualClock::new();
        let mut app = RecordingApp::new(&clock);
        let mut pump = ScriptedPump::new(vec![Err(TestError)]);

        let result = run_with_shell(engine(), &mut app, &clock, &mut pump);

        assert!(matches!(result, Err(ShellError::Pump(_))));
        assert!(app.events.is_empty());
    }

    #[test]
    fn application_error_still_runs_shutdown() {
        let clock = ManualClock::new();
        let mut app = RecordingApp::new(&clock);
        app.fail_frame = true;
        let mut pump = ScriptedPump::new(vec![Ok(ShellState::Ready), Ok(ShellState::Ready)]);

        let result = run_with_shell(engine(), &mut app, &clock, &mut pump);

        assert!(matches!(
            result,
            Err(ShellError::Engine(EngineError::Frame(_)))
        ));
        assert_eq!(app.events, vec!["init", "shutdown"]);
    }

    #[test]
    fn application_exit_request_stops_the_shell_loop() {
        let clock = ManualClock::new();
        let mut app = RecordingApp::new(&clock);
        app.exit_at_frame = Some(1);
        let mut pump = ScriptedPump::new(
            (0..5)
                .map(|_| Ok::<ShellState, TestError>(ShellState::Ready))
                .collect(),
        );

        let report = run_with_shell(engine(), &mut app, &clock, &mut pump).expect("run succeeds");

        assert_eq!(report.exit_reason, ExitReason::RequestedByApplication);
        assert_eq!(report.stats.frames(), 2);
        assert_eq!(pump.timeouts, vec![WAIT_FOR_SURFACE, Duration::ZERO]);
        assert_eq!(app.events, vec!["init", "shutdown"]);
    }
}
