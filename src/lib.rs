//! Apairon: the foundation of a modular game engine.
//!
//! Scope of this foundation (std only by default; the optional `window` feature adds winit):
//!
//! - [`clock`]: injectable monotonic time source (real and manual).
//! - [`pacing`]: frame pacing toward a target frame time (a target, not a guarantee).
//! - [`timestep`]: fixed-timestep accumulator with a bounded catch-up policy.
//! - [`diagnostics`]: frame, work and wait durations tracked separately.
//! - [`app`]: application lifecycle and the engine core (a frame stepper).
//! - [`shell`]: platform-independent driver running the engine alongside OS events.
//! - `window` (optional feature, winit): a native window implementing the shell.
//!
//! Rendering, input forwarding, assets and an ECS are deliberately absent: there is no
//! concrete requirement for them yet.

pub mod app;
pub mod clock;
pub mod diagnostics;
pub mod pacing;
pub mod shell;
pub mod timestep;
mod util;
#[cfg(feature = "window")]
pub mod window;

#[cfg(all(
    feature = "window",
    not(any(target_os = "linux", target_os = "windows", target_os = "macos"))
))]
compile_error!(
    "the `window` feature supports Linux, Windows and macOS only (it relies on winit's pump_app_events)"
);

pub use app::{
    Application, Control, Engine, EngineConfig, EngineError, ExitReason, FrameContext, RunReport,
    Step,
};
pub use clock::{Clock, ManualClock, MonotonicClock};
pub use diagnostics::{FrameStats, FrameTiming};
pub use pacing::FramePacer;
pub use shell::{EventPump, ShellError, ShellState, run_with_shell};
pub use timestep::{FixedStepper, StepPlan};
