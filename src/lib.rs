//! Apairon: the foundation of a modular game engine.
//!
//! Scope of this foundation (std only, no external dependencies):
//!
//! - [`clock`]: injectable monotonic time source (real and manual).
//! - [`pacing`]: frame pacing toward a target frame time (a target, not a guarantee).
//! - [`timestep`]: fixed-timestep accumulator with a bounded catch-up policy.
//! - [`diagnostics`]: frame, work and wait durations tracked separately.
//! - [`app`]: application lifecycle and the engine loop tying the above together.
//!
//! Windowing, rendering, input, assets and an ECS are deliberately absent: there is no
//! concrete requirement for them yet.

pub mod app;
pub mod clock;
pub mod diagnostics;
pub mod pacing;
pub mod timestep;
mod util;

pub use app::{
    Application, Control, Engine, EngineConfig, EngineError, ExitReason, FrameContext, RunReport,
};
pub use clock::{Clock, ManualClock, MonotonicClock};
pub use diagnostics::{FrameStats, FrameTiming};
pub use pacing::FramePacer;
pub use timestep::{FixedStepper, StepPlan};
