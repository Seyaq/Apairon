use std::time::Duration;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// Converts a nanosecond count to a [`Duration`], saturating the seconds part.
pub(crate) fn duration_from_nanos(nanos: u128) -> Duration {
    let secs = u64::try_from(nanos / NANOS_PER_SECOND).unwrap_or(u64::MAX);
    let subsec_nanos = u32::try_from(nanos % NANOS_PER_SECOND).unwrap_or(0);
    Duration::new(secs, subsec_nanos)
}
