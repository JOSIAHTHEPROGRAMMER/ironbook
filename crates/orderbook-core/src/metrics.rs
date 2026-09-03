//! Activity counters and latency tracking for the matching engine.
//!
//! [`Metrics`] is owned by [`crate::matching::MatchingEngine`],
//! mirroring how [`crate::orderbook::OrderBook`] is a distinct type
//! the engine owns rather than a set of fields folded directly into
//! it. Nothing here depends on order book or matching types,
//! `Metrics` only records what it is told happened, the engine
//! decides when to call into it.

use std::time::Duration;

/// Upper bound, in nanoseconds, of each latency histogram bucket.
///
/// Roughly log spaced in a one, two and a half, five pattern, the same
/// spacing used by common production histogram implementations. Chosen
/// to cover realistic in process latencies, submicrosecond matching
/// through low millisecond worst case, without needing to retain every
/// individual sample. Anything slower than the final bound falls into
/// an implicit overflow bucket.
const BUCKET_BOUNDS_NANOS: &[u64] = &[
    100,
    250,
    500,
    1_000,
    2_500,
    5_000,
    10_000,
    25_000,
    50_000,
    100_000,
    250_000,
    500_000,
    1_000_000,
    2_500_000,
    5_000_000,
    10_000_000,
    25_000_000,
    50_000_000,
    100_000_000,
];

/// A bounded memory latency histogram.
///
/// Stores per bucket counts rather than individual samples, so memory
/// use is fixed regardless of how many samples are recorded, a
/// requirement for a matching engine that may run continuously across
/// a full trading session. The tradeoff is that percentiles are bucket
/// boundary approximations, not exact values, this is documented on
/// [`LatencyHistogram::percentile`] rather than left implicit.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    bucket_counts: Vec<u64>,
    total_count: u64,
    sum_nanos: u128,
    max_nanos: u64,
}

impl LatencyHistogram {
    /// Creates an empty histogram.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bucket_counts: vec![0; BUCKET_BOUNDS_NANOS.len() + 1],
            total_count: 0,
            sum_nanos: 0,
            max_nanos: 0,
        }
    }

    /// Records one latency sample.
    ///
    /// Samples longer than `Duration::from_nanos(u64::MAX)` are clamped
    /// rather than overflowing the bucket lookup, this is not reachable
    /// with real in process latencies but keeps the method total rather
    /// than panicking on pathological input.
    pub fn record(&mut self, elapsed: Duration) {
        let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        let bucket = BUCKET_BOUNDS_NANOS
            .iter()
            .position(|&bound| nanos <= bound)
            .unwrap_or(BUCKET_BOUNDS_NANOS.len());

        self.bucket_counts[bucket] += 1;
        self.total_count += 1;
        self.sum_nanos += u128::from(nanos);
        self.max_nanos = self.max_nanos.max(nanos);
    }

    /// Returns the number of samples recorded.
    #[must_use]
    pub const fn count(&self) -> u64 {
        self.total_count
    }

    /// Returns the exact mean latency, or `None` if nothing was recorded.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        if self.total_count == 0 {
            return None;
        }
        let mean_nanos = self.sum_nanos / u128::from(self.total_count);
        Some(Duration::from_nanos(
            u64::try_from(mean_nanos).unwrap_or(u64::MAX),
        ))
    }

    /// Returns the slowest recorded latency, or `None` if nothing was recorded.
    #[must_use]
    pub fn peak(&self) -> Option<Duration> {
        if self.total_count == 0 {
            None
        } else {
            Some(Duration::from_nanos(self.max_nanos))
        }
    }

    /// Returns an approximate percentile latency.
    ///
    /// `percentile` is clamped to the `0.0..=1.0` range, `0.5` is the
    /// median. The result is the upper boundary of the bucket the
    /// requested percentile falls into, not the exact sample value,
    /// this is the accuracy tradeoff made in exchange for fixed memory
    /// use regardless of sample count. Returns `None` if nothing has
    /// been recorded.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    // Sample counts are far below f64's exact integer range in any
    // realistic benchmark, and the ceiling result is always a small
    // positive bucket index, so the float round trip loses nothing that
    // matters here.
    pub fn percentile(&self, percentile: f64) -> Option<Duration> {
        if self.total_count == 0 {
            return None;
        }

        let clamped = percentile.clamp(0.0, 1.0);
        let target = ((clamped * self.total_count as f64).ceil() as u64).max(1);

        let mut cumulative = 0u64;
        for (index, &bucket_count) in self.bucket_counts.iter().enumerate() {
            cumulative += bucket_count;
            if cumulative >= target {
                let bound_nanos = BUCKET_BOUNDS_NANOS
                    .get(index)
                    .copied()
                    .unwrap_or(self.max_nanos);
                // The bucket boundary is an upper bound on samples that
                // landed inside it, but the true maximum across every
                // sample is already known exactly, no percentile can
                // legitimately exceed it, so the estimate is clamped
                // rather than allowed to round above the real peak.
                return Some(Duration::from_nanos(bound_nanos.min(self.max_nanos)));
            }
        }

        Some(Duration::from_nanos(self.max_nanos))
    }

    /// Returns the approximate median latency, equivalent to
    /// [`LatencyHistogram::percentile`]`(0.5)`.
    #[must_use]
    pub fn median(&self) -> Option<Duration> {
        self.percentile(0.5)
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

/// Activity counters and latency histograms for one matching engine.
///
/// Reports only what it is told, [`crate::matching::MatchingEngine`]
/// calls the `record_*` methods at the appropriate points, `Metrics`
/// has no knowledge of orders, trades, or the book itself.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    orders_submitted: u64,
    orders_rejected: u64,
    orders_cancelled: u64,
    trades_executed: u64,
    submit_latency: LatencyHistogram,
    match_latency: LatencyHistogram,
}

impl Metrics {
    /// Creates an empty set of metrics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that an order was accepted and submitted for matching.
    pub fn record_order_submitted(&mut self) {
        self.orders_submitted += 1;
    }

    /// Records that an order was rejected before matching, for example
    /// by failing validation or by using an already active client order id.
    pub fn record_order_rejected(&mut self) {
        self.orders_rejected += 1;
    }

    /// Records that a resting order was cancelled.
    pub fn record_order_cancelled(&mut self) {
        self.orders_cancelled += 1;
    }

    /// Records that `count` trades were produced.
    ///
    /// Takes a count rather than being called once per trade so a
    /// caller matching several trades in one submission can report them
    /// in a single call.
    pub fn record_trades(&mut self, count: usize) {
        self.trades_executed += count as u64;
    }

    /// Records the end to end latency of one
    /// [`crate::matching::MatchingEngine::submit_limit_order`] or
    /// [`crate::matching::MatchingEngine::submit_market_order`] call.
    pub fn record_submit_latency(&mut self, elapsed: Duration) {
        self.submit_latency.record(elapsed);
    }

    /// Records the latency of the matching loop only, excluding
    /// validation and duplicate id checks.
    pub fn record_match_latency(&mut self, elapsed: Duration) {
        self.match_latency.record(elapsed);
    }

    /// Returns the number of orders accepted and submitted for matching.
    #[must_use]
    pub const fn orders_submitted(&self) -> u64 {
        self.orders_submitted
    }

    /// Returns the number of orders rejected before matching.
    #[must_use]
    pub const fn orders_rejected(&self) -> u64 {
        self.orders_rejected
    }

    /// Returns the number of resting orders cancelled.
    #[must_use]
    pub const fn orders_cancelled(&self) -> u64 {
        self.orders_cancelled
    }

    /// Returns the number of trades produced.
    #[must_use]
    pub const fn trades_executed(&self) -> u64 {
        self.trades_executed
    }

    /// Returns the end to end submit call latency histogram.
    #[must_use]
    pub const fn submit_latency(&self) -> &LatencyHistogram {
        &self.submit_latency
    }

    /// Returns the matching loop only latency histogram.
    #[must_use]
    pub const fn match_latency(&self) -> &LatencyHistogram {
        &self.match_latency
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_histogram_reports_no_data() {
        let histogram = LatencyHistogram::new();
        assert_eq!(histogram.count(), 0);
        assert_eq!(histogram.mean(), None);
        assert_eq!(histogram.peak(), None);
        assert_eq!(histogram.percentile(0.5), None);
    }

    #[test]
    fn single_sample_is_its_own_mean_peak_and_percentile() {
        let mut histogram = LatencyHistogram::new();
        histogram.record(Duration::from_micros(1));

        assert_eq!(histogram.count(), 1);
        assert_eq!(histogram.mean(), Some(Duration::from_micros(1)));
        assert_eq!(histogram.peak(), Some(Duration::from_micros(1)));
        assert_eq!(histogram.median(), Some(Duration::from_micros(1)));
    }

    #[test]
    fn peak_tracks_the_slowest_sample_regardless_of_order() {
        let mut histogram = LatencyHistogram::new();
        histogram.record(Duration::from_nanos(500));
        histogram.record(Duration::from_millis(1));
        histogram.record(Duration::from_nanos(200));

        assert_eq!(histogram.peak(), Some(Duration::from_millis(1)));
    }

    #[test]
    fn percentile_moves_toward_the_high_end_as_it_increases() {
        let mut histogram = LatencyHistogram::new();
        for _ in 0..90 {
            histogram.record(Duration::from_nanos(100));
        }
        for _ in 0..10 {
            histogram.record(Duration::from_millis(50));
        }

        let p50 = histogram.percentile(0.5).unwrap();
        let p99 = histogram.percentile(0.99).unwrap();
        assert!(p50 < p99);
        assert_eq!(p99, Duration::from_millis(50));
    }

    #[test]
    fn percentile_clamps_out_of_range_input() {
        let mut histogram = LatencyHistogram::new();
        histogram.record(Duration::from_nanos(100));

        assert_eq!(histogram.percentile(-1.0), histogram.percentile(0.0));
        assert_eq!(histogram.percentile(2.0), histogram.percentile(1.0));
    }

    #[test]
    fn sample_slower_than_the_widest_bucket_falls_into_overflow() {
        let mut histogram = LatencyHistogram::new();
        histogram.record(Duration::from_secs(10));

        assert_eq!(histogram.peak(), Some(Duration::from_secs(10)));
        assert_eq!(histogram.percentile(1.0), Some(Duration::from_secs(10)));
    }

    #[test]
    fn no_percentile_estimate_ever_exceeds_the_true_peak() {
        let mut histogram = LatencyHistogram::new();
        histogram.record(Duration::from_nanos(1));
        histogram.record(Duration::from_micros(66));

        let peak = histogram.peak().unwrap();
        for hundredth in 0..=100u32 {
            let fraction = f64::from(hundredth) / 100.0;
            let estimate = histogram.percentile(fraction).unwrap();
            assert!(
                estimate <= peak,
                "percentile {fraction} produced {estimate:?}, exceeding peak {peak:?}"
            );
        }
    }

    #[test]
    fn metrics_default_is_all_zero() {
        let metrics = Metrics::new();
        assert_eq!(metrics.orders_submitted(), 0);
        assert_eq!(metrics.orders_rejected(), 0);
        assert_eq!(metrics.orders_cancelled(), 0);
        assert_eq!(metrics.trades_executed(), 0);
        assert_eq!(metrics.submit_latency().count(), 0);
        assert_eq!(metrics.match_latency().count(), 0);
    }

    #[test]
    fn counters_increment_independently() {
        let mut metrics = Metrics::new();
        metrics.record_order_submitted();
        metrics.record_order_submitted();
        metrics.record_order_rejected();
        metrics.record_order_cancelled();
        metrics.record_trades(3);

        assert_eq!(metrics.orders_submitted(), 2);
        assert_eq!(metrics.orders_rejected(), 1);
        assert_eq!(metrics.orders_cancelled(), 1);
        assert_eq!(metrics.trades_executed(), 3);
    }

    #[test]
    fn submit_and_match_latency_are_tracked_separately() {
        let mut metrics = Metrics::new();
        metrics.record_submit_latency(Duration::from_micros(10));
        metrics.record_submit_latency(Duration::from_micros(20));
        metrics.record_match_latency(Duration::from_nanos(500));

        assert_eq!(metrics.submit_latency().count(), 2);
        assert_eq!(metrics.match_latency().count(), 1);
    }
}
