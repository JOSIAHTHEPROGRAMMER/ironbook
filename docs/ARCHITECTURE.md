# Architecture

This document explains the design decisions behind ironbook: what each piece does, why it's built this way rather than an obvious alternative, and how the pieces depend on each other. It assumes familiarity with the [README](../README.md)'s overview but not with the code itself.

## Workspace layout

The project is a Cargo workspace with two crates, split from the start rather than merged later:

- **`orderbook-core`** — the order book, matching engine, metrics, and domain types. No I/O, no CLI dependency, no knowledge that a terminal exists.
- **`orderbook-cli`** — a thin client over `orderbook-core`: command parsing, decimal price display, multi symbol session management.

The split exists because `orderbook-core` is the part worth reusing. A REST API, a WebSocket market data feed, or a persistence layer would all sit on top of the same engine without needing to touch matching logic, and none of them would want the CLI's command parsing or decimal formatting along for the ride. Keeping the boundary in place from Phase 1 also kept the matching logic testable in isolation the whole way through, `orderbook-core`'s test suite never needed a terminal, a session, or a parsed command to exercise the engine.

## Domain types (`types.rs`)

Every numeric value with a specific meaning gets its own type: `Price`, `Quantity`, `OrderId`, `ClientOrderId`, `TradeId`, `Symbol`. None of these are raw `i64` or `u64` passed around by convention. The compiler rejects code that passes a quantity where a price is expected, at zero runtime cost since these are all effectively `#[repr(transparent)]`.

**Prices are integer ticks, never floats.** A tick is the smallest price increment a symbol trades in; a price of `10050` at a tick size of `0.01` represents `100.50`. Real exchange matching engines do this for the same reason: floating point cannot represent every two decimal value exactly, and rounding error in a matching engine is unacceptable, not a cosmetic issue but a correctness one, since two engines computing the same trade could disagree on the exact price. The cost of this choice is that converting to and from a human readable decimal needs a tick size, handled entirely at the CLI's presentation boundary (`formatting.rs`), `orderbook-core` never sees a decimal string.

**`OrderId` doubles as the time priority key.** Values come from a single monotonically increasing counter owned by the matching engine. This means comparing two `OrderId`s with `<` directly answers "which order arrived first", no separate sequence counter is needed anywhere else in the system for FIFO ordering within a price level.

**`ClientOrderId` is a separate type from `OrderId`, deliberately.** `OrderId` is assigned by the engine and used internally for time priority. `ClientOrderId` is supplied by whoever submits the order and is what duplicate submission checks compare against. Conflating the two would mean either exposing internal sequencing to callers or losing the caller's own tracking id, neither is acceptable on a real exchange interface.

**`TradeId` has its own counter, separate from `OrderId`.** A trade is not an order; tying their numbering together would couple two independent sequences for no benefit and make either one harder to reason about alone.

## Orders (`orders.rs`)

`Order` is built only through `new_limit` or `new_market`, both validating inputs and returning a `Result`. There is no other path to construct an `Order`, which means a live `Order` can never be in an invalid state, no limit order with a non positive price, no order with zero quantity. Illegal states are unrepresentable rather than checked defensively at every use site.

Validation logic for limit orders lives in `Order::validate_limit_inputs`, a `pub(crate)` function shared between `new_limit` and the matching engine's `modify_order`. This matters specifically for `modify_order`'s correctness, see below.

`Order::fill` reduces remaining quantity and returns `bool`, not a `Result`. An attempt to overfill an order can only happen if the matching engine itself has a bug, computing a fill quantity larger than what remains, it is never something a caller's input can trigger. Modeling that as an internal invariant with a boolean return, rather than a public error variant callers are expected to handle, reflects that distinction: this is a signal to the engine's own code, not part of the public contract.

## The order book (`orderbook.rs`)

`OrderBook` holds one `HashMap<OrderId, Order>` as the single source of truth for order data, and two `BTreeMap<Price, PriceLevelQueue>` (bids and asks) that store only the ids resting at each price, in arrival order. Price levels never duplicate order data, they index into the order map.

**Cancellation is O(1)**, not the O(k) linear scan the project shipped with through Phase 7. Each price level's queue, `PriceLevelQueue`, is an arena backed doubly linked list: nodes live in a `Vec`, `prev`/`next` are indices into that `Vec` rather than pointers, the standard safe Rust substitute for an intrusive linked list. `OrderBook` tracks each resting order's node index in a `HashMap<OrderId, NodeIndex>`, so cancelling an order splices its node out directly, touching only its immediate neighbors, regardless of how many other orders share that price level. Freed slots go on a free list and get reused by later insertions, keeping memory bounded by a price level's peak concurrent size rather than growing with total churn over a long session.

This replaced a `VecDeque<OrderId>` per price level, where cancellation first had to scan the queue to find the order's position. That scan was flagged as a known optimization target from Phase 3 onward, and fixing it (Phase 8) turned out to change the actual computational complexity, not just the constant factor: cancelling every order in a 100,000 order deep price level dropped from roughly 1.8 seconds to roughly 24 milliseconds, a 77x improvement, with 10x more orders now costing roughly 10x the time instead of roughly 100x. See [BENCHMARKING.md](BENCHMARKING.md) for the full numbers.

The alternative designs considered and rejected: a `Vec` with `swap_remove` gives O(1) removal but breaks strict FIFO arrival order, since the last element moves into the removed slot; lazy tombstone deletion (mark cancelled, skip during matching, compact periodically) gives O(1) amortized cancellation but pushes complexity into the matching loop, which now has to skip tombstones, and into a compaction strategy. The arena backed linked list gives true O(1) removal, preserves FIFO order exactly, and keeps the matching loop itself unchanged, at the cost of being more code to write and reason about than either alternative.

## The matching engine (`matching.rs`)

**One `MatchingEngine` per symbol, deliberately.** This is a scope decision, not a missing feature: a matching engine that spans multiple symbols has to reason about cross symbol effects that don't actually exist on a real exchange (each symbol's book is independent), and keeping the scope narrow keeps the matching loop's invariants simple to state and test. Multi symbol support is a session concern, handled by the CLI's `Session` type wrapping one engine per symbol, not something `orderbook-core` itself needs to know about.

**Trades execute at the maker's price, not the taker's.** The maker is the resting order providing liquidity; the taker is the incoming order removing it. This is a real exchange rule, not an arbitrary choice: the maker committed to a price by resting on the book, the taker's limit price only expresses a willingness to trade at that price or better, so the trade should not cost the taker more than the maker was already offering.

**Duplicate client order id is checked before any matching starts**, not only at insertion time. Checking at insertion would be too late: a duplicate order could already have matched against other resting orders before the duplicate check ever ran, and those trades cannot be undone. The check has to be the very first thing that happens.

**`modify_order` is validate, then cancel, then resubmit, in that exact order.** The new price and quantity are validated before the existing order is touched. If validation fails, the original order is untouched, still resting exactly as it was. This is why `Order::validate_limit_inputs` is shared between `new_limit` and `modify_order`, the same validation logic has to run before either construction or replacement, and a rejected modify must never destroy the order it was trying to replace. The replacement receives a new `OrderId` and goes to the back of its price level's queue, it loses time priority exactly as it would on a real exchange; only the caller's client order id carries over. `modify_order` reuses `submit_limit_order` internally rather than duplicating matching logic, and records no metrics of its own for the same reason, the `cancel_order` and `submit_limit_order` calls it delegates to already record correctly, adding metrics calls directly in `modify_order` would double count.

## Errors (`errors.rs`)

Errors are layered rather than flattened into one enum. `OrderError` covers bad input to `Order` construction. `OrderBookError` covers book level failures: duplicate id, unknown order, a market order that can't rest. `MatchingError` wraps both via `#[from]` and `#[error(transparent)]`, letting the engine return one error type to its caller without redeclaring what's already fully described by the layers underneath. All three are `#[non_exhaustive]`, since future order types (stop orders, iceberg orders) will add their own rejection cases, and code matching on these enums from outside the crate should always include a wildcard arm.

## Metrics (`metrics.rs`)

`Metrics` is a separate type `MatchingEngine` owns, mirroring how `OrderBook` is a distinct type the engine delegates to rather than a set of fields folded directly in. It tracks plain counters (orders submitted, rejected, cancelled, trades executed) and two `LatencyHistogram`s: one for the full `submit_limit_order`/`submit_market_order` call, one for the matching loop in isolation, tracked separately so submission overhead and matching cost can be told apart.

**Percentiles are bucket boundary approximations, not exact values**, a deliberate memory tradeoff. `LatencyHistogram` stores per bucket counts across a fixed set of geometrically spaced boundaries, not individual samples, so memory use stays constant regardless of how many samples are recorded, a requirement for an engine that might run for a full trading session. The cost is that `percentile()` returns the bucket's upper boundary as its estimate rather than an exact value. Every estimate is clamped to the true observed maximum (`peak()`, tracked exactly), since a percentile can never legitimately exceed the actual highest latency observed, and an earlier version of this code did not enforce that, producing a `p99` reading above `peak`, caught by running the CLI end to end rather than by unit tests alone.

The CLI surfaces this as two separate commands rather than one: `stats` shows the plain counters, `metrics` shows counters plus the full latency breakdown. This mirrors a real distinction in production observability: business metrics (counts, volume) versus full instrumentation data (latency percentiles) used for performance monitoring are conventionally exposed separately.

## The CLI (`orderbook-cli`)

**`Session` wraps one `MatchingEngine` per symbol**, created lazily on first use. Read only lookups (`orders`, `book`, `trades`, `stats`, `metrics`) use `engine`/`engine_mut_if_exists`, which never create an engine as a side effect, looking at a symbol that has never traded should report an empty result, not silently instantiate state for it.

**Prices are parsed and formatted with manual integer arithmetic, never floats.** `formatting.rs` converts between decimal strings like `"100.50"` and integer ticks by working on the string's digits directly. Parsing as a float and multiplying would reintroduce exactly the precision problem integer ticks exist to avoid, undermining the reason for that choice at the one place a human actually types a price.

**Commands use flag based syntax** (`buy --symbol AAPL --price 100.50 --quantity 10 --client-order-id 1`), not positional arguments, parsed with `clap`'s derive API. This trades brevity for clarity and for `clap`'s built in validation, missing or malformed flags produce a real error message rather than a positional argument silently landing in the wrong field.

**The `benchmark` command runs against the caller's real session engine**, not an isolated scratch instance, a deliberate choice: its activity becomes part of that symbol's real session history, visible afterward in `stats` and `metrics`. This means workload generators cannot assume a clean book. Generated client order ids are computed from the highest id currently resting on the book, not assumed safe, and resting prices for workloads that need controlled state (`worst-case`, `heavy-market`, `heavy-cancel`, `large-partial-fill`) are derived from the live book's current best price on the opposite side, rather than a fixed constant, for the same reason: a fixed price that was safe on a fresh engine is not guaranteed to stay clear once real session history exists. An earlier version assumed a fixed safe price and silently broke, a `heavy-cancel` benchmark run after a `heavy-market` run found its own resting orders immediately filled by liquidity the prior run left behind, leaving nothing to cancel.

## Benchmarks (`benches/matching_benchmarks.rs`)

Five workload shapes, each isolating one property of the engine rather than mixing concerns, at order counts of 100, 1,000, 10,000, and 100,000:

- **random** — realistic mixed limit and market order flow, a general throughput baseline.
- **worst_case** — builds a resting order at each of N distinct price levels holding one unit, then times a single market order sweeping the entire book. Setup stays outside Criterion's timed closure, isolating the matching loop's actual worst case single call cost, not the cost of building the book.
- **heavy_market** — seeds resting liquidity, then times a stream of market orders against it.
- **heavy_cancel** — rests N orders all at the same price level, the worst case shape for the order book's per level scan, then times cancelling all of them in shuffled order. This is the group that produced the before and after numbers for the Phase 8 optimization.
- **large_partial_fill** — rests many small maker orders at one price, then times a handful of large takers each partially filling across several of them.

Randomness uses `rand`'s `StdRng`, seeded deterministically rather than from a fresh source each run, so benchmark input is identical run to run and measured differences reflect changes to the engine, not to the input.
