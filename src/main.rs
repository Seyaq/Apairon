use std::convert::Infallible;
use std::process::ExitCode;
use std::time::Duration;

use apairon::{
    Application, Clock, Control, Engine, EngineConfig, FrameContext, MonotonicClock,
};

/// Number of frames the demo runs before exiting.
const DEMO_FRAMES: u64 = 480;

/// Headless demo application: counts fixed steps and does no real work.
struct DemoApp {
    fixed_ticks: u64,
}

impl Application for DemoApp {
    type Error = Infallible;

    fn fixed_update(&mut self, _step: Duration) -> Result<Control, Self::Error> {
        self.fixed_ticks += 1;
        Ok(Control::Continue)
    }

    fn frame(&mut self, _ctx: &FrameContext) -> Result<Control, Self::Error> {
        Ok(Control::Continue)
    }
}

fn main() -> ExitCode {
    let config = EngineConfig {
        max_frames: Some(DEMO_FRAMES),
        ..EngineConfig::default()
    };
    let engine = match Engine::new(config) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("invalid engine configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    let clock = MonotonicClock::new();
    let mut app = DemoApp { fixed_ticks: 0 };
    let started = clock.now();

    match engine.run(&mut app, &clock) {
        Ok(report) => {
            let wall = clock.now().saturating_sub(started);
            println!("exit reason     : {:?}", report.exit_reason);
            println!("frames          : {}", report.stats.frames());
            println!("wall time       : {wall:?}");
            println!("avg frame       : {:?}", report.stats.average_frame());
            println!("avg work        : {:?}", report.stats.average_work());
            println!("avg wait        : {:?}", report.stats.average_wait());
            println!("max frame       : {:?}", report.stats.max_frame());
            println!("fixed steps     : {} (app counted {})", report.fixed_steps, app.fixed_ticks);
            println!("dropped sim time: {:?}", report.dropped_time);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("engine error: {error}");
            ExitCode::FAILURE
        }
    }
}
