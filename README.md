# ironbook

[![CI](https://github.com/JOSIAHTHEPROGRAMMER/ironbook/actions/workflows/ci.yml/badge.svg)](https://github.com/JOSIAHTHEPROGRAMMER/ironbook/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/JOSIAHTHEPROGRAMMER/ironbook/branch/main/graph/badge.svg)](https://codecov.io/gh/JOSIAHTHEPROGRAMMER/ironbook)
[![Rust](https://img.shields.io/badge/rust-1.97.1-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![docs](https://img.shields.io/badge/docs-rustdoc-blue.svg)](https://github.com/JOSIAHTHEPROGRAMMER/ironbook)

A limit order book and matching engine, built in Rust, modeled on the core of a modern electronic exchange.

ironbook matches buy and sell orders by price time priority, the same rule real exchanges use: better prices trade first, and orders at the same price trade in the order they arrived. It ships as an interactive command line application today, structured so a market data feed, a REST API, or persistence can be layered on top later without reworking the matching logic.

## Why integer ticks

Prices and quantities are represented as scaled integers rather than floating point or a decimal type. This is the same approach used by real exchange matching engines: it's deterministic, allocation free, and avoids floating point rounding entirely. The tradeoff is that display formatting needs a tick size to convert back to a human readable price, which the CLI's presentation layer handles, `orderbook-core` itself never touches a decimal.

## Architecture

The project is a Cargo workspace with two crates:

- **orderbook-core** — the order book, matching engine, metrics collection, and domain types. No I/O, no CLI dependency. This is the crate that would get reused if a REST API or WebSocket feed were added later. Includes a Criterion benchmark suite covering five workload shapes (random flow, worst case book sweeps, heavy market orders, heavy cancellation, large partial fills) at order counts from 100 to 100,000.
- **orderbook-cli** — the command line interface. A thin client over `orderbook-core`, handling command parsing, decimal price display, and session management across multiple symbols.

Splitting these out from the start keeps the matching logic testable in isolation and keeps the door open for other frontends without touching the engine.

Full design rationale, including why prices use integer ticks, why order ids double as the time priority key, and why each `MatchingEngine` is scoped to a single symbol, lives in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Quickstart

```bash
git clone https://github.com/JOSIAHTHEPROGRAMMER/ironbook.git
cd ironbook
cargo build
cargo test
cargo run --bin ironbook
```

Inside the interactive session:

```
> buy --symbol AAPL --price 100.50 --quantity 10 --client-order-id 1
> sell --symbol AAPL --price 100.50 --quantity 4 --client-order-id 2
> book --symbol AAPL
> stats --symbol AAPL
> metrics --symbol AAPL
> benchmark --symbol AAPL --orders 10000 --workload heavy-cancel
> help
```

`help` lists every available command. See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for the full local development workflow, and [docs/BENCHMARKING.md](docs/BENCHMARKING.md) for how to run and interpret the Criterion benchmarks.

## Roadmap

| Phase | Scope                                                   | Status      |
| ----- | ------------------------------------------------------- | ----------- |
| 1     | Repository setup, CI, license, contributing guidelines  | Complete    |
| 2     | Domain models: types, errors, orders, trades            | Complete    |
| 3     | Order book: price and time priority storage             | Complete    |
| 4     | Matching engine: price time priority matching           | Complete    |
| 5     | Order management: cancel and modify                     | Complete    |
| 6     | Interactive CLI                                         | Complete    |
| 7     | Metrics collection, Criterion benchmarks, CLI reporting | Complete    |
| 8     | Optimization: O(1) order cancellation                   | Complete    |
| 9     | Documentation and interview preparation guide           | In progress |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
