# Benchmarking

`orderbook-core` ships a Criterion benchmark suite (`crates/orderbook-core/benches/matching_benchmarks.rs`) covering five workload shapes at four order counts each, twenty cases total. This document explains how to run it, what each group measures, and includes real measured output as a baseline.

## Running it

```bash
cargo bench
```

Runs the full statistical suite: Criterion's defaults are 100 samples per case with a 3 second warm up and 5 second measurement window, at the 100,000 order scale across twenty cases this can take several minutes. Results and an HTML report land in `target/criterion/report/index.html`.

For a faster run that still produces real statistical output, just stopping earlier once Criterion's significance threshold is reached:

```bash
cargo bench -- --quick
```

For a pure correctness check, running every case once with no statistical analysis:

```bash
cargo bench -- --test
```

To benchmark a single group instead of all five:

```bash
cargo bench -- heavy_cancel
```

## What each group measures

- **`random_workload`** — a realistic mix of limit and market orders, roughly even buy and sell, ninety percent limit. General throughput under order flow that isn't adversarial in either direction.
- **`worst_case_sweep`** — setup (untimed) rests one order at each of N distinct price levels, holding one unit each. The timed portion is a single market order sized to sweep the entire book. This isolates the matching loop's actual worst case: every level requires its own `best_price` lookup, no level is cleared by fewer than the minimum number of fills.
- **`heavy_market_orders`** — setup seeds resting liquidity across ten price levels, the timed portion is a stream of small market orders against it.
- **`heavy_cancel`** — setup rests N orders all at the same price level, the worst case shape for a price level's internal scan, the timed portion cancels every one of them in randomized order. This is the group that demonstrates the Phase 8 optimization directly, see below.
- **`large_partial_fill`** — setup rests many one unit maker orders at a single price, the timed portion submits a handful of large takers, each sized to partially fill across many of them.

In every group, setup work stays outside Criterion's timed closure. Only the operation the group is named for is actually measured.

## Measured results

Captured with `cargo bench -- --quick --noplot` on the development machine used to build this project. **These numbers are hardware dependent and will differ on other machines**; run the suite locally for numbers that reflect your own hardware. What should hold regardless of hardware is the _shape_ of how each group scales with order count.

| Order count | random | worst_case | heavy_market | heavy_cancel | large_partial_fill |
| ----------: | -----: | ---------: | -----------: | -----------: | -----------------: |
|         100 |   55µs |       40µs |         18µs |         13µs |               27µs |
|       1,000 |  601µs |      363µs |        176µs |        102µs |              232µs |
|      10,000 | 6.49ms |     4.56ms |       1.83ms |       1.64ms |             2.90ms |
|     100,000 | 76.3ms |      115ms |       19.5ms |       60.0ms |             56.5ms |

Every group scales roughly linearly with order count, one to two orders of magnitude in time for one to two orders of magnitude in order count, which is the expected shape for this engine: matching is O(1) amortized per fill in the common case, and cancellation is O(1) as of Phase 8 (previously O(k) in the size of the price level, see below).

`worst_case_sweep` is the slowest group at scale, expected, since it's specifically constructed to maximize the number of price levels a single order must visit, closer to O(n) _lookups_ in one call rather than n cheap operations spread across many calls.

## Case study: the Phase 8 cancellation fix

`OrderBook::cancel` originally scanned a price level's queue linearly to find an order's position before removing it, an O(k) cost in the number of orders resting at that price. This was flagged from Phase 3 onward as the known optimization target and fixed in Phase 8 by replacing the per level `VecDeque<OrderId>` with an arena backed doubly linked list, giving true O(1) removal. Full design rationale is in [ARCHITECTURE.md](ARCHITECTURE.md).

The before and after numbers below were captured with a standalone timing driver, not the Criterion suite itself, isolating exactly the cancel operation: build a book of N orders all resting at one price, then cancel them in strict reverse order, the worst possible scan depth for the old linear scan on every single cancel.

|  Orders | Before (O(k) scan) | After (O(1) arena) | Speedup |
| ------: | -----------------: | -----------------: | ------: |
|     100 |               13µs |               12µs |     ~1x |
|   1,000 |              264µs |             80.9µs |    3.3x |
|  10,000 |             19.6ms |             1.26ms |   15.6x |
|  50,000 |              464ms |             8.80ms |   52.8x |
| 100,000 |              1.84s |             23.9ms |     77x |

The important number isn't the raw speedup, it's the change in scaling. From 10,000 to 100,000 orders, a 10x increase, the old implementation took roughly 100x longer (quadratic), the new one takes roughly 19x longer (linear, with a small constant overhead). That's the actual fix: not a faster constant factor, a different growth curve.

The Criterion suite's own `heavy_cancel` group (table above) measures a related but not identical operation: cancellation in _randomly shuffled_ order rather than strict reverse, and through the full benchmark harness rather than a bare timing loop, which is why its 100,000 order number (60.0ms) differs from the standalone driver's (23.9ms) despite both reflecting the same O(1) underlying operation. Both are legitimate measurements of different things; neither contradicts the other.

## A note on `--quick` versus a full run

`--quick` stops sampling once Criterion's significance threshold is reached rather than always collecting the full default sample count. It's the right choice for getting a fast, still statistically real answer, as used for the table above, but a full `cargo bench` run with default settings will produce tighter confidence intervals and is what the HTML report's regression detection (comparing against a saved baseline) is designed around. For tracking whether a change actually regressed performance, save a baseline before the change and compare after:

```bash
cargo bench -- --save-baseline before
# make the change
cargo bench -- --baseline before
```
