//! Reproducible frame-pacing benchmark (std only).
//!
//! Usage: `cargo run --release --example pacing_bench -- [frames] [target_fps] [warmup] [csv_path]`
//! Defaults: 10000 frames, 240 fps, 240 warm-up frames discarded, no CSV.
//!
//! Frame times are start-to-start durations taken from the engine's own measured
//! `FrameContext::delta`. The first delta (zero by definition) and the warm-up frames are
//! discarded. Percentiles use the nearest-rank method. Deviations are `value - budget`.
//!
//! Protocol for comparable runs: release build, same frame count, 5 or more runs per
//! variant, same machine state (power source, CPU governor, background load). Record
//! `rustc -V`, the kernel/OS version and the CPU model next to the results; this program
//! cannot measure them. Compare variants by the median of each run's statistic.

use std::convert::Infallible;
use std::fmt::Write;
use std::process::ExitCode;

use apairon::{
    Application, Clock, Control, Engine, EngineConfig, FrameContext, FramePacer, MonotonicClock,
};

const DEFAULT_FRAMES: usize = 10_000;
const DEFAULT_TARGET_FPS: u32 = 240;
const DEFAULT_WARMUP_FRAMES: usize = 240;

/// Records every measured frame delta in a pre-allocated buffer.
struct Recorder {
    deltas_ns: Vec<u64>,
}

impl Application for Recorder {
    type Error = Infallible;

    fn frame(&mut self, ctx: &FrameContext) -> Result<Control, Self::Error> {
        let nanos = u64::try_from(ctx.delta.as_nanos()).unwrap_or(u64::MAX);
        self.deltas_ns.push(nanos);
        Ok(Control::Continue)
    }
}

/// Nearest-rank percentile of an ascending-sorted slice. `p` is in `[0, 100]`.
fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() || !(0.0..=100.0).contains(&p) {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}

/// `value - budget` in microseconds, signed.
fn signed_micros(value_ns: u64, budget_ns: u64) -> f64 {
    (i128::from(value_ns) - i128::from(budget_ns)) as f64 / 1_000.0
}

fn parse_arg<T: std::str::FromStr>(
    args: &[String],
    index: usize,
    default: T,
    name: &str,
) -> Result<T, String> {
    match args.get(index) {
        None => Ok(default),
        Some(text) => text
            .parse::<T>()
            .map_err(|_| format!("invalid {name}: {text}")),
    }
}

fn print_row(label: &str, value_ns: u64, budget_ns: u64) {
    println!(
        "  {label:<6} {:>10.1} us   deviation {:>+9.1} us",
        value_ns as f64 / 1_000.0,
        signed_micros(value_ns, budget_ns)
    );
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: usize = parse_arg(&args, 0, DEFAULT_FRAMES, "frames")?;
    let target_fps: u32 = parse_arg(&args, 1, DEFAULT_TARGET_FPS, "target_fps")?;
    let warmup: usize = parse_arg(&args, 2, DEFAULT_WARMUP_FRAMES, "warmup")?;
    let csv_path = args.get(3);

    let budget = FramePacer::from_target_fps(target_fps)
        .and_then(|pacer| pacer.target_frame_time())
        .ok_or_else(|| format!("invalid target fps: {target_fps}"))?;
    let budget_ns = u64::try_from(budget.as_nanos()).unwrap_or(u64::MAX);

    if frames <= warmup + 1 {
        return Err(format!(
            "frames ({frames}) must exceed warmup + 1 ({})",
            warmup + 1
        ));
    }

    let config = EngineConfig {
        target_fps: Some(target_fps),
        max_frames: Some(u64::try_from(frames).unwrap_or(u64::MAX)),
        ..EngineConfig::default()
    };
    let engine = Engine::new(config).map_err(|error| format!("invalid configuration: {error}"))?;

    let clock = MonotonicClock::new();
    let mut app = Recorder {
        deltas_ns: Vec::with_capacity(frames),
    };
    let started = clock.now();
    let report = engine
        .run(&mut app, &clock)
        .map_err(|error| format!("engine error: {error}"))?;
    let wall = clock.now().saturating_sub(started);

    let in_order: Vec<u64> = app.deltas_ns.iter().skip(1 + warmup).copied().collect();
    let mut sorted = in_order.clone();
    sorted.sort_unstable();
    let count = sorted.len();

    let sum: u128 = sorted.iter().map(|value| u128::from(*value)).sum();
    let mean_ns = u64::try_from(sum / count as u128).unwrap_or(u64::MAX);
    let budget_wide = u128::from(budget_ns);
    let slow = sorted
        .iter()
        .filter(|value| u128::from(**value) * 10 > budget_wide * 11)
        .count();
    let fast = sorted
        .iter()
        .filter(|value| u128::from(**value) * 10 < budget_wide * 9)
        .count();
    let missed = sorted
        .iter()
        .filter(|value| u128::from(**value) > budget_wide * 2)
        .count();
    let percent = |n: usize| n as f64 * 100.0 / count as f64;

    println!(
        "environment : os={} arch={} threads={:?} profile={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map(|n| n.get()).ok(),
        if cfg!(debug_assertions) {
            "debug (NOT representative)"
        } else {
            "release"
        }
    );
    println!(
        "run         : {frames} frames, {warmup} warm-up discarded, target {target_fps} fps, budget {budget_ns} ns"
    );
    println!(
        "wall        : {wall:?} -> {:.3} fps over all frames",
        frames as f64 / wall.as_secs_f64()
    );
    println!(
        "engine avg  : frame {:?} | work {:?} | wait {:?} | max {:?}",
        report.stats.average_frame(),
        report.stats.average_work(),
        report.stats.average_wait(),
        report.stats.max_frame()
    );
    println!("frame time distribution (start-to-start), n = {count}:");
    for (label, p) in [
        ("min", 0.0),
        ("p50", 50.0),
        ("p90", 90.0),
        ("p99", 99.0),
        ("p99.9", 99.9),
        ("max", 100.0),
    ] {
        if let Some(value) = percentile(&sorted, p) {
            print_row(label, value, budget_ns);
        }
    }
    print_row("mean", mean_ns, budget_ns);
    println!(
        "  achieved fps from mean frame time: {:.3}",
        1e9 / mean_ns as f64
    );
    println!("  frames > 1.1x budget : {slow} ({:.2}%)", percent(slow));
    println!("  frames < 0.9x budget : {fast} ({:.2}%)", percent(fast));
    println!("  frames > 2x budget   : {missed} ({:.2}%)", percent(missed));

    if let Some(path) = csv_path {
        let mut text = String::from("frame_index,frame_time_ns\n");
        for (offset, value) in in_order.iter().enumerate() {
            writeln!(text, "{},{value}", warmup + offset).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, text).map_err(|error| format!("cannot write {path}: {error}"))?;
        println!("raw samples written to {path}");
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_to_ten() -> Vec<u64> {
        (1..=10).collect()
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let data = one_to_ten();
        assert_eq!(percentile(&data, 0.0), Some(1));
        assert_eq!(percentile(&data, 50.0), Some(5));
        assert_eq!(percentile(&data, 90.0), Some(9));
        assert_eq!(percentile(&data, 99.9), Some(10));
        assert_eq!(percentile(&data, 100.0), Some(10));
    }

    #[test]
    fn percentile_rejects_empty_input_and_bad_rank() {
        assert_eq!(percentile(&[], 50.0), None);
        assert_eq!(percentile(&one_to_ten(), 101.0), None);
        assert_eq!(percentile(&one_to_ten(), -1.0), None);
    }

    #[test]
    fn deviation_is_signed_in_microseconds() {
        assert!((signed_micros(4_424_390, 4_166_666) - 257.724).abs() < 1e-9);
        assert!((signed_micros(3_000_000, 4_000_000) + 1_000.0).abs() < 1e-9);
    }
}
