//! Opens a window and runs the engine in it until the window is closed.
//!
//! `cargo run --release --features window --example window`

use std::convert::Infallible;
use std::process::ExitCode;

use apairon::window::{WindowConfig, run_windowed};
use apairon::{Application, Control, Engine, EngineConfig, FrameContext, MonotonicClock};

/// Counts frames and does no real work.
struct Demo;

impl Application for Demo {
    type Error = Infallible;

    fn frame(&mut self, _ctx: &FrameContext) -> Result<Control, Self::Error> {
        Ok(Control::Continue)
    }
}

fn main() -> ExitCode {
    let engine = match Engine::new(EngineConfig::default()) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("invalid engine configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    let clock = MonotonicClock::new();
    match run_windowed(engine, &mut Demo, &clock, WindowConfig::default()) {
        Ok(report) => {
            println!("exit reason : {:?}", report.exit_reason);
            println!("frames      : {}", report.stats.frames());
            println!("avg frame   : {:?}", report.stats.average_frame());
            println!("avg work    : {:?}", report.stats.average_work());
            println!("avg wait    : {:?}", report.stats.average_wait());
            println!("max frame   : {:?}", report.stats.max_frame());
            println!("fixed steps : {}", report.fixed_steps);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
