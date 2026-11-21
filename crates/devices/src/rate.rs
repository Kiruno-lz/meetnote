//! Copy throughput over a sliding window, so the shown speed follows the
//! device without jumping on every progress report. Time is passed in, which
//! keeps the meter deterministic under test.

use std::collections::VecDeque;
use std::time::Duration;

/// How far back samples count.
const WINDOW: Duration = Duration::from_secs(2);
/// A shorter span says too little to show a speed.
const MIN_SPAN: Duration = Duration::from_millis(250);

#[derive(Debug, Default)]
pub struct TransferRate {
    /// (time since the meter started, total bytes copied by then).
    samples: VecDeque<(Duration, u64)>,
}

impl TransferRate {
    /// Records that `total` bytes have been copied at `at`.
    pub fn record(&mut self, at: Duration, total: u64) {
        if let Some(&(last_at, last_total)) = self.samples.back()
            && (at < last_at || total < last_total)
        {
            // Time or the byte count went backwards: a new run.
            self.samples.clear();
        }
        self.samples.push_back((at, total));
        // Older samples go, but two always stay to measure a stalled copy.
        while self.samples.len() > 2
            && self
                .samples
                .front()
                .is_some_and(|&(first, _)| at.saturating_sub(first) > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    /// Bytes per second over the window, once it spans long enough.
    pub fn bytes_per_second(&self) -> Option<f64> {
        let (&(first_at, first), &(last_at, last)) = (self.samples.front()?, self.samples.back()?);
        let span = last_at.checked_sub(first_at)?;
        (span >= MIN_SPAN).then(|| (last - first) as f64 / span.as_secs_f64())
    }
}

/// `12.3 MB/s`, in decimal units as Finder shows transfer speeds.
pub fn format_rate(bytes_per_second: f64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut value = bytes_per_second.max(0.0);
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn measures_over_the_recent_window_only() {
        let mut rate = TransferRate::default();
        rate.record(ms(0), 0);
        assert_eq!(rate.bytes_per_second(), None, "one sample");
        rate.record(ms(100), 1_000_000);
        assert_eq!(rate.bytes_per_second(), None, "too short");
        rate.record(ms(500), 5_000_000);
        assert_eq!(rate.bytes_per_second(), Some(10_000_000.0));
        // A slow first second ages out of the two-second window.
        rate.record(ms(3_000), 10_000_000);
        rate.record(ms(4_000), 30_000_000);
        assert_eq!(rate.bytes_per_second(), Some(20_000_000.0));
    }

    #[test]
    fn a_new_run_starts_over() {
        let mut rate = TransferRate::default();
        rate.record(ms(0), 0);
        rate.record(ms(1_000), 8_000_000);
        rate.record(ms(1_100), 0);
        assert_eq!(rate.bytes_per_second(), None);
    }

    #[test]
    fn formats_decimal_units() {
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(12_345.0), "12.3 KB/s");
        assert_eq!(format_rate(38_400_000.0), "38.4 MB/s");
        assert_eq!(format_rate(152_000_000.0), "152 MB/s");
        assert_eq!(format_rate(1_200_000_000.0), "1.2 GB/s");
    }
}
