# ironbook

[![CI](https://github.com/JOSIAHTHEPROGRAMMER/ironbook/actions/workflows/ci.yml/badge.svg)](https://github.com/JOSIAHTHEPROGRAMMER/ironbook/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/JOSIAHTHEPROGRAMMER/ironbook/branch/main/graph/badge.svg)](https://codecov.io/gh/JOSIAHTHEPROGRAMMER/ironbook)
[![Rust](https://img.shields.io/badge/rust-1.97.1-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![docs](https://img.shields.io/badge/docs-rustdoc-blue.svg)](https://github.com/JOSIAHTHEPROGRAMMER/ironbook)

A limit order book and matching engine, built in Rust, modeled on the core of a modern electronic exchange.

ironbook matches buy and sell orders by price time priority, the same rule real exchanges use: better prices trade first, and orders at the same price trade in the order they arrived. It's a command line application today, structured so a market data feed, a REST API, or persistence can be layered on top later without reworking the matching logic.

## Why integer ticks

Prices and quantities are represented as scaled integers rather than floating point or a decimal type. This is the same approach used by real exchange matching engines: it's deterministic, allocation free, and avoids floating point rounding entirely. The tradeoff is that display formatting needs a tick size to convert back to a human readable price, which the config layer handles.

## Architecture

The project is a Cargo workspace with two crates:

- **orderbook-core** - the order book, matching engine, and domain types. No I/O, no CLI dependency. This is the crate that would get reused if a REST API or WebSocket feed were added later.
- **orderbook-cli** - the command line interface. A thin client over orderbook-core.

Splitting these out from the start keeps the matching logic testable in isolation and keeps the door open for other frontends without touching the engine.

## Quickstart

```bash
git clone https://github.com/JOSIAHTHEPROGRAMMER/ironbook.git
cd ironbook
cargo build
cargo test
cargo run --bin ironbook
```

## Roadmap

| Phase | Scope                              | Status      |
| ----- | ---------------------------------- | ----------- |
| 1     | Repository setup, CI, CLI skeleton | In progress |
| 2     | Domain models                      | Not started |
| 3     | Order book                         | Not started |
| 4     | Matching engine                    | Not started |
| 5     | Order management                   | Not started |
| 6     | CLI                                | Not started |
| 7     | Metrics and benchmarks             | Not started |
| 8     | Optimization                       | Not started |
| 9     | Documentation                      | Not started |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
