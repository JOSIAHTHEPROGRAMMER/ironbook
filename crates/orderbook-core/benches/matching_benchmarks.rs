//! Criterion benchmarks for the matching engine across a range of
//! order counts and workload shapes.
//!
//! Each benchmark group isolates one property of the matching engine
//! rather than mixing concerns together: general throughput under
//! realistic order flow, the worst case cost of a single call, market
//! order heavy flow, cancellation under a deep price level, and
//! partial fill heavy flow. Wherever a workload needs the book in a
//! particular state before the interesting operation runs, that setup
//! happens inside criterion's untimed setup closure, only the
//! operation actually being measured runs inside the timed closure.

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use orderbook_core::matching::MatchingEngine;
use orderbook_core::types::{ClientOrderId, OrderId, Price, Quantity, Side, Symbol};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};

/// Order counts exercised by every benchmark group.
const ORDER_COUNTS: [usize; 4] = [100, 1_000, 10_000, 100_000];

/// Fixed seed for every workload generator, so benchmark runs are
/// reproducible across machines and over time rather than drifting
/// with whatever randomness happened to land on a given run.
const SEED: u64 = 42;

/// One action to replay against a fresh engine during a benchmark.
///
/// Cancel actions carry a pre-computed order id rather than looking
/// one up during replay. A fresh engine assigns ids from its own
/// counter strictly in call order, which is fully deterministic given
/// a known, fixed action sequence, so the id a submission will
/// receive is already known at generation time.
enum Action {
    /// Submits a limit order.
    SubmitLimit {
        /// Caller supplied id, must be unique within the workload.
        client_order_id: u64,
        /// Side of the book the order trades on.
        side: Side,
        /// Limit price.
        price: Price,
        /// Quantity to trade.
        quantity: Quantity,
    },
    /// Submits a market order.
    SubmitMarket {
        /// Caller supplied id, must be unique within the workload.
        client_order_id: u64,
        /// Side of the book the order trades on.
        side: Side,
        /// Quantity to trade.
        quantity: Quantity,
    },
    /// Cancels a previously submitted order by its predicted id.
    Cancel {
        /// The engine assigned id the target submission is expected
        /// to have received.
        order_id: u64,
    },
}

/// The symbol every benchmark trades, arbitrary since no benchmark
/// compares behavior across symbols.
fn symbol() -> Symbol {
    Symbol::new("BENCH")
}

/// Creates a seeded random number generator.
///
/// A fixed seed is used rather than a fresh one every run, workload
/// shape should be identical between benchmark runs so that measured
/// differences reflect changes to the engine, not to the input.
fn seeded_rng() -> StdRng {
    StdRng::seed_from_u64(SEED)
}

/// Replays a sequence of actions against an engine, discarding results.
///
/// Benchmarked workloads intentionally include actions that can be
/// rejected, land unmatched, or fail to cancel, that is realistic
/// order flow, not a benchmark defect, so results are discarded rather
/// than unwrapped.
fn run_actions(engine: &mut MatchingEngine, actions: &[Action]) {
    for action in actions {
        match action {
            Action::SubmitLimit {
                client_order_id,
                side,
                price,
                quantity,
            } => {
                let _ = engine.submit_limit_order(
                    ClientOrderId::from_raw(*client_order_id),
                    *side,
                    *price,
                    *quantity,
                );
            }
            Action::SubmitMarket {
                client_order_id,
                side,
                quantity,
            } => {
                let _ = engine.submit_market_order(
                    ClientOrderId::from_raw(*client_order_id),
                    *side,
                    *quantity,
                );
            }
            Action::Cancel { order_id } => {
                let _ = engine.cancel_order(OrderId::from_sequence(*order_id));
            }
        }
    }
}

/// Builds a mixed limit and market order workload with prices spread
/// around a central value.
///
/// Ninety percent limit, ten percent market, roughly even buy and
/// sell, produces order flow where most submissions rest or partially
/// fill rather than either always crossing or never crossing, closer
/// to what a real, unremarkable trading session looks like than any
/// single deterministic pattern would be.
fn random_workload(order_count: usize) -> Vec<Action> {
    let mut rng = seeded_rng();
    let mut actions = Vec::with_capacity(order_count);

    for client_order_id in 1..=u64_from(order_count) {
        let side = if rng.random_bool(0.5) {
            Side::Buy
        } else {
            Side::Sell
        };
        let quantity = Quantity::from_units(rng.random_range(1..=100));

        if rng.random_bool(0.9) {
            let offset = rng.random_range(-500..=500);
            actions.push(Action::SubmitLimit {
                client_order_id,
                side,
                price: Price::from_ticks(10_000 + offset),
                quantity,
            });
        } else {
            actions.push(Action::SubmitMarket {
                client_order_id,
                side,
                quantity,
            });
        }
    }

    actions
}

/// Builds the setup phase of the worst case workload: one resting
/// sell order at each of `order_count` distinct, adjacent price
/// levels, each holding exactly one unit.
///
/// A single unit per level forces a sweeping order to visit every
/// level individually rather than clearing several levels at once,
/// this is deliberately the most expensive shape for the matching
/// loop to walk, one `best_price` lookup and one maker fill per level,
/// with no level satisfied by more than a single fill.
fn worst_case_setup(order_count: usize) -> Vec<Action> {
    (1..=u64_from(order_count))
        .map(|client_order_id| Action::SubmitLimit {
            client_order_id,
            side: Side::Sell,
            price: Price::from_ticks(10_000 + i64_from(client_order_id) - 1),
            quantity: Quantity::from_units(1),
        })
        .collect()
}

/// Builds the setup phase of the heavy market order workload: resting
/// sell liquidity spread across ten price levels.
///
/// Ten levels with deep quantity at each is enough liquidity that the
/// timed market orders mostly find a fill without exhausting the book
/// partway through, keeping the timed phase representative of ongoing
/// market order flow rather than degrading into an empty book toward
/// the end of the run.
fn heavy_market_setup(order_count: usize) -> Vec<Action> {
    let resting_orders = order_count / 2;
    (1..=u64_from(resting_orders))
        .map(|client_order_id| {
            let level = i64_from(client_order_id) % 10;
            Action::SubmitLimit {
                client_order_id,
                side: Side::Sell,
                price: Price::from_ticks(10_000 + level),
                quantity: Quantity::from_units(1_000),
            }
        })
        .collect()
}

/// Builds the timed phase of the heavy market order workload: a
/// stream of small market buy orders against the liquidity `heavy_market_setup` seeded.
fn heavy_market_timed(order_count: usize) -> Vec<Action> {
    let mut rng = seeded_rng();
    let resting_orders = u64_from(order_count / 2);
    let taker_orders = u64_from(order_count) - resting_orders;

    (1..=taker_orders)
        .map(|offset| Action::SubmitMarket {
            client_order_id: resting_orders + offset,
            side: Side::Buy,
            quantity: Quantity::from_units(rng.random_range(1..=10)),
        })
        .collect()
}

/// Builds the setup phase of the heavy cancel workload: `order_count`
/// resting buy orders, all at the same price.
///
/// Concentrating every order on one price level is deliberately the
/// worst case for `OrderBook::cancel`'s linear scan within a level,
/// already flagged as the known optimization target, every cancel in
/// this workload pays the full cost of scanning a level with up to
/// `order_count` entries in it.
fn heavy_cancel_setup(order_count: usize) -> Vec<Action> {
    (1..=u64_from(order_count))
        .map(|client_order_id| Action::SubmitLimit {
            client_order_id,
            side: Side::Buy,
            price: Price::from_ticks(10_000),
            quantity: Quantity::from_units(1),
        })
        .collect()
}

/// Builds the timed phase of the heavy cancel workload: every order
/// id from the setup phase, in shuffled order.
///
/// Shuffling rather than cancelling in submission order avoids always
/// measuring the cheapest case for a queue based scan, the front of
/// the queue, cancelling in random order exercises the full range of
/// scan depths a real session would produce.
fn heavy_cancel_timed(order_count: usize) -> Vec<Action> {
    let mut rng = seeded_rng();
    let mut order_ids: Vec<u64> = (1..=u64_from(order_count)).collect();
    order_ids.shuffle(&mut rng);
    order_ids
        .into_iter()
        .map(|order_id| Action::Cancel { order_id })
        .collect()
}

/// Builds the setup phase of the large partial fill workload:
/// `order_count` resting sell orders of one unit each, all at the
/// same price.
///
/// Every resting order holds the minimum possible quantity, so any
/// taker large enough to need more than one unit is guaranteed to
/// partially fill across several makers rather than being satisfied
/// by a single one.
fn large_partial_fill_setup(order_count: usize) -> Vec<Action> {
    (1..=u64_from(order_count))
        .map(|client_order_id| Action::SubmitLimit {
            client_order_id,
            side: Side::Sell,
            price: Price::from_ticks(10_000),
            quantity: Quantity::from_units(1),
        })
        .collect()
}

/// Builds the timed phase of the large partial fill workload: a
/// handful of large market buy orders, each sized to sweep across
/// many of the one unit makers `large_partial_fill_setup` seeded.
fn large_partial_fill_timed(order_count: usize) -> Vec<Action> {
    let taker_count = (order_count / 100).max(1);
    let resting_orders = u64_from(order_count);
    let taker_quantity = resting_orders / u64_from(taker_count);

    (1..=u64_from(taker_count))
        .map(|offset| Action::SubmitMarket {
            client_order_id: resting_orders + offset,
            side: Side::Buy,
            quantity: Quantity::from_units(taker_quantity),
        })
        .collect()
}

/// Converts an order count to `u64` for use as a client or order id.
///
/// Order counts in this file are always small, fixed benchmark
/// constants, well within `u64`'s range, this exists only so call
/// sites read as an intentional conversion rather than a bare `as`.
fn u64_from(value: usize) -> u64 {
    u64::try_from(value).expect("benchmark order counts fit in u64")
}

/// Converts a `u64` order or client id back to `i64` for building a
/// benchmark price offset.
fn i64_from(value: u64) -> i64 {
    i64::try_from(value).expect("benchmark order counts fit in i64")
}

/// General throughput under a realistic mix of limit and market orders.
fn bench_random(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("random_workload");
    for &order_count in &ORDER_COUNTS {
        let actions = random_workload(order_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(order_count),
            &actions,
            |bencher, actions| {
                bencher.iter_batched(
                    || MatchingEngine::new(symbol()),
                    |mut engine| {
                        run_actions(&mut engine, actions);
                        engine
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

/// The worst case cost of a single call: one market order sweeping an
/// entire book of single unit price levels.
fn bench_worst_case(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("worst_case_sweep");
    for &order_count in &ORDER_COUNTS {
        let setup_actions = worst_case_setup(order_count);
        let sweep_client_id = u64_from(order_count) + 1;
        let sweep_quantity = Quantity::from_units(u64_from(order_count));

        group.bench_with_input(
            BenchmarkId::from_parameter(order_count),
            &setup_actions,
            |bencher, setup_actions| {
                bencher.iter_batched(
                    || {
                        let mut engine = MatchingEngine::new(symbol());
                        run_actions(&mut engine, setup_actions);
                        engine
                    },
                    |mut engine| {
                        let _ = engine.submit_market_order(
                            ClientOrderId::from_raw(sweep_client_id),
                            Side::Buy,
                            sweep_quantity,
                        );
                        engine
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

/// Throughput of a stream of market orders against deep resting
/// liquidity.
fn bench_heavy_market(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("heavy_market_orders");
    for &order_count in &ORDER_COUNTS {
        let setup_actions = heavy_market_setup(order_count);
        let timed_actions = heavy_market_timed(order_count);

        group.bench_with_input(
            BenchmarkId::from_parameter(order_count),
            &(setup_actions, timed_actions),
            |bencher, (setup_actions, timed_actions)| {
                bencher.iter_batched(
                    || {
                        let mut engine = MatchingEngine::new(symbol());
                        run_actions(&mut engine, setup_actions);
                        engine
                    },
                    |mut engine| {
                        run_actions(&mut engine, timed_actions);
                        engine
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

/// Cancellation throughput against the worst case shape for the
/// order book's O(k) linear scan within a price level.
fn bench_heavy_cancel(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("heavy_cancel");
    for &order_count in &ORDER_COUNTS {
        let setup_actions = heavy_cancel_setup(order_count);
        let timed_actions = heavy_cancel_timed(order_count);

        group.bench_with_input(
            BenchmarkId::from_parameter(order_count),
            &(setup_actions, timed_actions),
            |bencher, (setup_actions, timed_actions)| {
                bencher.iter_batched(
                    || {
                        let mut engine = MatchingEngine::new(symbol());
                        run_actions(&mut engine, setup_actions);
                        engine
                    },
                    |mut engine| {
                        run_actions(&mut engine, timed_actions);
                        engine
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

/// Throughput of large takers each partially filling across many
/// small resting makers.
fn bench_large_partial_fill(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("large_partial_fill");
    for &order_count in &ORDER_COUNTS {
        let setup_actions = large_partial_fill_setup(order_count);
        let timed_actions = large_partial_fill_timed(order_count);

        group.bench_with_input(
            BenchmarkId::from_parameter(order_count),
            &(setup_actions, timed_actions),
            |bencher, (setup_actions, timed_actions)| {
                bencher.iter_batched(
                    || {
                        let mut engine = MatchingEngine::new(symbol());
                        run_actions(&mut engine, setup_actions);
                        engine
                    },
                    |mut engine| {
                        run_actions(&mut engine, timed_actions);
                        engine
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_random,
    bench_worst_case,
    bench_heavy_market,
    bench_heavy_cancel,
    bench_large_partial_fill,
);
criterion_main!(benches);
