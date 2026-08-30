//! Generates and runs a synthetic order workload against a live
//! session engine, for a quick throughput and latency demonstration
//! without leaving the interactive session.
//!
//! This is distinct from the Criterion benchmarks in `orderbook-core`.
//! Those run in a controlled, statistically sampled harness purely for
//! measurement. This runs once, against whatever symbol the caller
//! names, and its activity becomes part of that symbol's real session
//! history, submitted orders trade against the real book and count
//! toward its real metrics. For isolated, repeatable numbers suitable
//! for comparing across changes, use `cargo bench`, not this command.

use std::time::{Duration, Instant};

use clap::ValueEnum;
use orderbook_core::matching::MatchingEngine;
use orderbook_core::types::{ClientOrderId, Price, Quantity, Side};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};

/// Which workload shape a benchmark run should generate.
///
/// Mirrors the workload categories exercised by the Criterion
/// benchmarks in `orderbook-core`, so a number seen here and a number
/// seen in `cargo bench` output describe comparable activity, even
/// though this command runs once and those run under statistical
/// sampling.
#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum Workload {
    /// Mixed limit and market orders at randomized prices and sides.
    Random,
    /// A deep, single unit per level book, followed by one order that
    /// sweeps every level.
    WorstCase,
    /// Resting liquidity followed by a stream of market orders.
    HeavyMarket,
    /// A single deep price level, followed by cancelling every order
    /// resting in it.
    HeavyCancel,
    /// Many small resting makers, followed by a few large takers that
    /// each partially fill across several of them.
    LargePartialFill,
}

/// The outcome of one benchmark run.
#[derive(Debug, Clone, Copy)]
pub struct BenchmarkResult {
    orders_submitted: u64,
    elapsed: Duration,
}

impl BenchmarkResult {
    /// Returns the number of orders submitted during the run.
    #[must_use]
    pub const fn orders_submitted(&self) -> u64 {
        self.orders_submitted
    }

    /// Returns how long submission took.
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Returns submitted orders per second, or `None` if elapsed time
    /// was too small to divide by meaningfully.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    // Order counts here are caller supplied benchmark sizes, nowhere
    // near f64's exact integer range, the precision this could lose
    // does not exist in practice for this value.
    pub fn throughput_per_second(&self) -> Option<f64> {
        let seconds = self.elapsed.as_secs_f64();
        if seconds <= 0.0 {
            None
        } else {
            Some(self.orders_submitted as f64 / seconds)
        }
    }
}

/// Runs a benchmark workload against a live engine and reports timing.
///
/// Generated client order ids start one above the highest client
/// order id currently resting on the engine's book, computed fresh
/// from the live book rather than assumed, so a run can never collide
/// with an order the caller already had resting, regardless of what
/// numbering scheme they used for it.
pub fn run(engine: &mut MatchingEngine, workload: Workload, order_count: u64) -> BenchmarkResult {
    let start_client_id = engine
        .book()
        .orders()
        .map(|order| order.client_order_id().value())
        .max()
        .map_or(1, |highest| highest + 1);

    let mut rng = StdRng::seed_from_u64(start_client_id ^ order_count);

    let (orders_submitted, elapsed) = match workload {
        Workload::Random => run_random(engine, &mut rng, start_client_id, order_count),
        Workload::WorstCase => run_worst_case(engine, start_client_id, order_count),
        Workload::HeavyMarket => run_heavy_market(engine, &mut rng, start_client_id, order_count),
        Workload::HeavyCancel => run_heavy_cancel(engine, &mut rng, start_client_id, order_count),
        Workload::LargePartialFill => run_large_partial_fill(engine, start_client_id, order_count),
    };

    BenchmarkResult {
        orders_submitted,
        elapsed,
    }
}

/// Returns a price guaranteed clear of whatever is currently resting
/// on the opposite side of the book, for workloads that need to build
/// resting liquidity on `side` without risking an immediate cross
/// against orders the caller, or an earlier benchmark run, already
/// left on the book.
///
/// Looks at the live book rather than assuming a fixed price is safe,
/// the same principle already used for client order id assignment,
/// a fixed price that happened to be clear on a fresh engine is not
/// guaranteed to still be clear once real session history exists.
/// Falls back to `baseline` only when the opposite side is empty, the
/// only case where there is nothing to stay clear of.
fn safe_resting_price(engine: &MatchingEngine, side: Side, baseline: i64) -> Price {
    const MARGIN_TICKS: i64 = 10_000;

    let opposite_best = engine.book().best_price(side.opposite());
    let ticks = match (side, opposite_best) {
        (Side::Buy, Some(best_ask)) => best_ask.ticks() - MARGIN_TICKS,
        (Side::Sell, Some(best_bid)) => best_bid.ticks() + MARGIN_TICKS,
        (_, None) => baseline,
    };

    // A limit price must be strictly positive, a large enough margin
    // compounding across many prior runs could in principle drive this
    // non-positive, floor it rather than submit an order guaranteed to
    // be rejected.
    Price::from_ticks(ticks.max(1))
}

/// Runs the random workload: mostly limit orders with some market
/// orders mixed in, roughly even buy and sell, prices spread around a
/// central value.
fn run_random(
    engine: &mut MatchingEngine,
    rng: &mut StdRng,
    start_client_id: u64,
    order_count: u64,
) -> (u64, Duration) {
    let started_at = Instant::now();

    for offset in 0..order_count {
        let client_order_id = ClientOrderId::from_raw(start_client_id + offset);
        let side = if rng.random_bool(0.5) {
            Side::Buy
        } else {
            Side::Sell
        };
        let quantity = Quantity::from_units(rng.random_range(1..=100));

        if rng.random_bool(0.9) {
            let price_offset: i64 = rng.random_range(-500..=500);
            let _ = engine.submit_limit_order(
                client_order_id,
                side,
                Price::from_ticks(10_000 + price_offset),
                quantity,
            );
        } else {
            let _ = engine.submit_market_order(client_order_id, side, quantity);
        }
    }

    (order_count, started_at.elapsed())
}

/// Runs the worst case workload: builds a resting sell order at each
/// of `order_count` distinct price levels holding one unit, then
/// submits a single market buy order sized to sweep every level.
fn run_worst_case(
    engine: &mut MatchingEngine,
    start_client_id: u64,
    order_count: u64,
) -> (u64, Duration) {
    let base_price = safe_resting_price(engine, Side::Sell, 10_000).ticks();
    let started_at = Instant::now();

    for offset in 0..order_count {
        let client_order_id = ClientOrderId::from_raw(start_client_id + offset);
        let level = i64::try_from(offset).unwrap_or(i64::MAX);
        let _ = engine.submit_limit_order(
            client_order_id,
            Side::Sell,
            Price::from_ticks(base_price + level),
            Quantity::from_units(1),
        );
    }

    let sweep_id = ClientOrderId::from_raw(start_client_id + order_count);
    let _ = engine.submit_market_order(sweep_id, Side::Buy, Quantity::from_units(order_count));

    (order_count + 1, started_at.elapsed())
}

/// Runs the heavy market order workload: seeds resting sell liquidity
/// across ten price levels, then submits a stream of small market buy
/// orders against it.
fn run_heavy_market(
    engine: &mut MatchingEngine,
    rng: &mut StdRng,
    start_client_id: u64,
    order_count: u64,
) -> (u64, Duration) {
    let base_price = safe_resting_price(engine, Side::Sell, 10_000).ticks();
    let resting_orders = order_count / 2;
    let taker_orders = order_count - resting_orders;
    let started_at = Instant::now();

    for offset in 0..resting_orders {
        let client_order_id = ClientOrderId::from_raw(start_client_id + offset);
        let level = i64::try_from(offset % 10).unwrap_or(0);
        let _ = engine.submit_limit_order(
            client_order_id,
            Side::Sell,
            Price::from_ticks(base_price + level),
            Quantity::from_units(1_000),
        );
    }

    for offset in 0..taker_orders {
        let client_order_id = ClientOrderId::from_raw(start_client_id + resting_orders + offset);
        let quantity = Quantity::from_units(rng.random_range(1..=10));
        let _ = engine.submit_market_order(client_order_id, Side::Buy, quantity);
    }

    (order_count, started_at.elapsed())
}

/// Runs the heavy cancel workload: rests `order_count` orders all at
/// the same price, the worst case shape for the order book's linear
/// scan within a level, then cancels every one of them in shuffled
/// order.
fn run_heavy_cancel(
    engine: &mut MatchingEngine,
    rng: &mut StdRng,
    start_client_id: u64,
    order_count: u64,
) -> (u64, Duration) {
    let price = safe_resting_price(engine, Side::Buy, 10_000);
    let started_at = Instant::now();
    let mut order_ids = Vec::new();

    for offset in 0..order_count {
        let client_order_id = ClientOrderId::from_raw(start_client_id + offset);
        let result =
            engine.submit_limit_order(client_order_id, Side::Buy, price, Quantity::from_units(1));
        // Only orders that actually rested belong in the cancel list,
        // an order that filled immediately is gone from the book and
        // cancelling it would just fail, this is checked explicitly
        // rather than assumed safe by the chosen price alone.
        if let Ok(report) = result
            && report.resting()
        {
            order_ids.push(report.order_id());
        }
    }

    order_ids.shuffle(rng);
    let cancelled = order_ids.len();
    for order_id in order_ids {
        let _ = engine.cancel_order(order_id);
    }

    let submitted = u64::try_from(cancelled).unwrap_or(order_count);
    (order_count + submitted, started_at.elapsed())
}

/// Runs the large partial fill workload: rests `order_count` one unit
/// sell orders all at the same price, then submits a handful of large
/// market buy orders, each sized to sweep across many of them.
fn run_large_partial_fill(
    engine: &mut MatchingEngine,
    start_client_id: u64,
    order_count: u64,
) -> (u64, Duration) {
    let price = safe_resting_price(engine, Side::Sell, 10_000);
    let started_at = Instant::now();

    for offset in 0..order_count {
        let client_order_id = ClientOrderId::from_raw(start_client_id + offset);
        let _ =
            engine.submit_limit_order(client_order_id, Side::Sell, price, Quantity::from_units(1));
    }

    let taker_count = (order_count / 100).max(1);
    let taker_quantity = order_count / taker_count;

    for offset in 0..taker_count {
        let client_order_id = ClientOrderId::from_raw(start_client_id + order_count + offset);
        let _ = engine.submit_market_order(
            client_order_id,
            Side::Buy,
            Quantity::from_units(taker_quantity),
        );
    }

    (order_count + taker_count, started_at.elapsed())
}

#[cfg(test)]
mod tests {
    use orderbook_core::types::Symbol;

    use super::*;

    fn engine() -> MatchingEngine {
        MatchingEngine::new(Symbol::new("AAPL"))
    }

    #[test]
    fn random_workload_submits_exactly_order_count_orders() {
        let mut engine = engine();
        let result = run(&mut engine, Workload::Random, 50);
        assert_eq!(result.orders_submitted(), 50);
        assert_eq!(engine.metrics().orders_submitted(), 50);
    }

    #[test]
    fn worst_case_workload_submits_order_count_plus_one_orders() {
        let mut engine = engine();
        let result = run(&mut engine, Workload::WorstCase, 20);
        assert_eq!(result.orders_submitted(), 21);
    }

    #[test]
    fn worst_case_workload_leaves_the_book_empty_after_the_sweep() {
        let mut engine = engine();
        run(&mut engine, Workload::WorstCase, 20);
        assert!(engine.book().is_empty());
    }

    #[test]
    fn heavy_cancel_workload_leaves_no_resting_orders() {
        let mut engine = engine();
        run(&mut engine, Workload::HeavyCancel, 30);
        assert!(engine.book().is_empty());
        assert_eq!(engine.metrics().orders_cancelled(), 30);
    }

    #[test]
    fn heavy_cancel_still_cancels_everything_after_prior_runs_left_resting_liquidity() {
        // Reproduces the exact sequence that surfaced a real bug: a
        // prior benchmark run leaves resting liquidity at the fixed
        // price a later workload assumed was clear, causing that
        // workload's own orders to fill immediately instead of resting,
        // so there was nothing left for it to actually cancel.
        let mut engine = engine();
        run(&mut engine, Workload::HeavyMarket, 150);
        let cancelled_before = engine.metrics().orders_cancelled();

        run(&mut engine, Workload::HeavyCancel, 150);

        assert_eq!(engine.metrics().orders_cancelled(), cancelled_before + 150);
    }

    #[test]
    fn benchmark_never_collides_with_an_existing_client_order_id() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(9_000),
                Quantity::from_units(1),
            )
            .unwrap();

        let result = run(&mut engine, Workload::Random, 20);
        assert_eq!(result.orders_submitted(), 20);
        // A collision would mean every submission using id 1 was
        // rejected before matching, well below orders_submitted's
        // count, this failing would signal the id scheme is broken.
        assert_eq!(engine.metrics().orders_rejected(), 0);
    }

    #[test]
    fn throughput_per_second_is_none_for_zero_elapsed() {
        let result = BenchmarkResult {
            orders_submitted: 10,
            elapsed: Duration::ZERO,
        };
        assert_eq!(result.throughput_per_second(), None);
    }

    #[test]
    fn throughput_per_second_divides_orders_by_elapsed_seconds() {
        let result = BenchmarkResult {
            orders_submitted: 100,
            elapsed: Duration::from_secs(2),
        };
        assert_eq!(result.throughput_per_second(), Some(50.0));
    }
}
