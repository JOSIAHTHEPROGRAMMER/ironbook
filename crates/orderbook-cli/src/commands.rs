//! Defines every interactive command and how it maps to matching
//! engine calls.

use std::collections::VecDeque;

use clap::{Args, Subcommand};
use orderbook_core::errors::OrderBookError;
use orderbook_core::matching::ExecutionReport;
use orderbook_core::orderbook::OrderBook;
use orderbook_core::orders::Order;
use orderbook_core::trade::Trade;
use orderbook_core::types::{ClientOrderId, OrderId, Price, Quantity, Side, Symbol};

use crate::formatting::{format_price, parse_price};
use crate::session::Session;

/// A limit order's arguments, symbol, price, quantity, and the caller
/// supplied id used for tracking and duplicate detection.
#[derive(Args, Debug)]
pub struct LimitOrderArgs {
    /// Symbol to trade
    #[arg(long)]
    symbol: String,
    /// Limit price, for example 100.50
    #[arg(long, value_parser = parse_price)]
    price: Price,
    /// Quantity to trade
    #[arg(long)]
    quantity: u64,
    /// Caller supplied id for tracking and duplicate detection
    #[arg(long)]
    client_order_id: u64,
}

/// A market order's arguments, symbol, quantity, and client order id.
#[derive(Args, Debug)]
pub struct MarketOrderArgs {
    /// Symbol to trade
    #[arg(long)]
    symbol: String,
    /// Quantity to trade
    #[arg(long)]
    quantity: u64,
    /// Caller supplied id for tracking and duplicate detection
    #[arg(long)]
    client_order_id: u64,
}

/// Identifies a resting order to cancel.
#[derive(Args, Debug)]
pub struct CancelArgs {
    /// Symbol the order was placed on
    #[arg(long)]
    symbol: String,
    /// Engine assigned order id, shown when the order was submitted
    #[arg(long)]
    order_id: u64,
}

/// Identifies a resting order and its replacement price and quantity.
#[derive(Args, Debug)]
pub struct ModifyArgs {
    /// Symbol the order was placed on
    #[arg(long)]
    symbol: String,
    /// Engine assigned order id, shown when the order was submitted
    #[arg(long)]
    order_id: u64,
    /// New limit price
    #[arg(long, value_parser = parse_price)]
    price: Price,
    /// New quantity
    #[arg(long)]
    quantity: u64,
}

/// A symbol to look up, used by the read only display commands.
#[derive(Args, Debug)]
pub struct SymbolArgs {
    /// Symbol to look up
    #[arg(long)]
    symbol: String,
}

/// Every command the interactive session understands.
///
/// Each variant becomes a subcommand parsed from one line of input, for
/// example `buy --symbol AAPL --price 100.50 --quantity 10
/// --client-order-id 1`.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Places a limit buy order
    Buy(LimitOrderArgs),
    /// Places a limit sell order
    Sell(LimitOrderArgs),
    /// Places a market buy order
    MarketBuy(MarketOrderArgs),
    /// Places a market sell order
    MarketSell(MarketOrderArgs),
    /// Cancels a resting order
    Cancel(CancelArgs),
    /// Modifies a resting order's price and quantity
    Modify(ModifyArgs),
    /// Lists resting orders for a symbol
    Orders(SymbolArgs),
    /// Shows the order book for a symbol
    Book(SymbolArgs),
    /// Shows trade history for a symbol
    Trades(SymbolArgs),
    /// Shows available commands
    Help,
    /// Clears the screen
    Clear,
    /// Resets all session state
    Reset,
    /// Exits the interactive session
    Exit,
}

/// What the REPL should do after a command runs.
///
/// Clear and Exit are handled by the terminal loop itself, not by
/// printing lines, giving them their own variants keeps that dispatch
/// out of the string output and lets the REPL act on them directly.
pub enum Outcome {
    /// Lines of output to print, one per line.
    Lines(Vec<String>),
    /// Clear the terminal screen.
    Clear,
    /// End the session.
    Exit,
}

/// Runs a command against the session and returns what the REPL should do.
pub fn execute(command: Command, session: &mut Session) -> Outcome {
    match command {
        Command::Buy(args) => Outcome::Lines(submit_limit(session, &args, Side::Buy)),
        Command::Sell(args) => Outcome::Lines(submit_limit(session, &args, Side::Sell)),
        Command::MarketBuy(args) => Outcome::Lines(submit_market(session, &args, Side::Buy)),
        Command::MarketSell(args) => Outcome::Lines(submit_market(session, &args, Side::Sell)),
        Command::Cancel(args) => Outcome::Lines(cancel(session, &args)),
        Command::Modify(args) => Outcome::Lines(modify(session, &args)),
        Command::Orders(args) => Outcome::Lines(list_orders(session, &args)),
        Command::Book(args) => Outcome::Lines(show_book(session, &args)),
        Command::Trades(args) => Outcome::Lines(list_trades(session, &args)),
        Command::Help => Outcome::Lines(help_lines()),
        Command::Clear => Outcome::Clear,
        Command::Reset => Outcome::Lines(reset(session)),
        Command::Exit => Outcome::Exit,
    }
}

fn submit_limit(session: &mut Session, args: &LimitOrderArgs, side: Side) -> Vec<String> {
    let symbol = Symbol::new(args.symbol.clone());
    let engine = session.engine_mut(&symbol);
    let result = engine.submit_limit_order(
        ClientOrderId::from_raw(args.client_order_id),
        side,
        args.price,
        Quantity::from_units(args.quantity),
    );
    match result {
        Ok(report) => format_execution_report(&report),
        Err(error) => vec![format!("error: {error}")],
    }
}

fn submit_market(session: &mut Session, args: &MarketOrderArgs, side: Side) -> Vec<String> {
    let symbol = Symbol::new(args.symbol.clone());
    let engine = session.engine_mut(&symbol);
    let result = engine.submit_market_order(
        ClientOrderId::from_raw(args.client_order_id),
        side,
        Quantity::from_units(args.quantity),
    );
    match result {
        Ok(report) => format_execution_report(&report),
        Err(error) => vec![format!("error: {error}")],
    }
}

fn cancel(session: &mut Session, args: &CancelArgs) -> Vec<String> {
    let symbol = Symbol::new(args.symbol.clone());
    let order_id = OrderId::from_sequence(args.order_id);

    match session.engine_mut_if_exists(&symbol) {
        Some(engine) => match engine.cancel_order(order_id) {
            Ok(order) => vec![format!("order {} cancelled", order.id().sequence())],
            Err(error) => vec![format!("error: {error}")],
        },
        None => vec![format!("error: {}", OrderBookError::UnknownOrder(order_id))],
    }
}

fn modify(session: &mut Session, args: &ModifyArgs) -> Vec<String> {
    let symbol = Symbol::new(args.symbol.clone());
    let order_id = OrderId::from_sequence(args.order_id);

    match session.engine_mut_if_exists(&symbol) {
        Some(engine) => {
            let result =
                engine.modify_order(order_id, args.price, Quantity::from_units(args.quantity));
            match result {
                Ok(report) => format_execution_report(&report),
                Err(error) => vec![format!("error: {error}")],
            }
        }
        None => vec![format!("error: {}", OrderBookError::UnknownOrder(order_id))],
    }
}

fn list_orders(session: &Session, args: &SymbolArgs) -> Vec<String> {
    let symbol = Symbol::new(args.symbol.clone());
    let Some(engine) = session.engine(&symbol) else {
        return vec![format!("no orders for {}", symbol.as_str())];
    };

    let mut orders: Vec<&Order> = engine.book().orders().collect();
    if orders.is_empty() {
        return vec![format!("no resting orders for {}", symbol.as_str())];
    }
    orders.sort_by_key(|order| (order.price(), order.id()));

    orders
        .iter()
        .map(|order| {
            format!(
                "order {} : {} {} @ {} remaining {}",
                order.id().sequence(),
                format_side(order.side()),
                order.original_quantity().units(),
                order
                    .price()
                    .map_or_else(|| "market".to_string(), format_price),
                order.remaining_quantity().units(),
            )
        })
        .collect()
}

fn show_book(session: &Session, args: &SymbolArgs) -> Vec<String> {
    let symbol = Symbol::new(args.symbol.clone());
    let Some(engine) = session.engine(&symbol) else {
        return vec![format!("no orders for {}", symbol.as_str())];
    };

    let book = engine.book();
    let mut lines = vec![format!("book for {}", symbol.as_str()), "asks:".to_string()];
    for (price, order_ids) in book.price_levels(Side::Sell) {
        lines.push(format_price_level(price, order_ids, book));
    }
    lines.push("bids:".to_string());
    for (price, order_ids) in book.price_levels(Side::Buy).rev() {
        lines.push(format_price_level(price, order_ids, book));
    }
    lines
}

fn list_trades(session: &Session, args: &SymbolArgs) -> Vec<String> {
    let symbol = Symbol::new(args.symbol.clone());
    let Some(engine) = session.engine(&symbol) else {
        return vec![format!("no trades for {}", symbol.as_str())];
    };

    let trades = engine.trade_history();
    if trades.is_empty() {
        return vec![format!("no trades for {}", symbol.as_str())];
    }

    trades.iter().map(format_trade).collect()
}

fn reset(session: &mut Session) -> Vec<String> {
    let symbol_count = session.symbol_count();
    session.reset();
    vec![format!("session reset, cleared {symbol_count} symbols")]
}

fn format_execution_report(report: &ExecutionReport) -> Vec<String> {
    let mut lines = vec![format!("order {} submitted", report.order_id().sequence())];

    for trade in report.trades() {
        lines.push(format_trade(trade));
    }

    if report.resting() {
        lines.push(format!(
            "remaining {} resting on book",
            report.remaining_quantity().units()
        ));
    } else if report.remaining_quantity().units() > 0 {
        lines.push(format!(
            "remaining {} unfilled, not resting",
            report.remaining_quantity().units()
        ));
    } else {
        lines.push("fully filled".to_string());
    }

    lines
}

fn format_trade(trade: &Trade) -> String {
    format!(
        "trade {} : {} {} @ {} maker {} taker {}",
        trade.id().sequence(),
        trade.quantity().units(),
        format_side(trade.taker_side()),
        format_price(trade.price()),
        trade.maker_order_id().sequence(),
        trade.taker_order_id().sequence(),
    )
}

fn format_price_level(price: Price, order_ids: &VecDeque<OrderId>, book: &OrderBook) -> String {
    let total: u64 = order_ids
        .iter()
        .filter_map(|id| book.get(*id))
        .map(|order| order.remaining_quantity().units())
        .sum();
    format!("  {} x {}", format_price(price), total)
}

fn format_side(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn help_lines() -> Vec<String> {
    vec![
        "available commands:".to_string(),
        "  buy --symbol SYM --price PRICE --quantity QTY --client-order-id ID".to_string(),
        "  sell --symbol SYM --price PRICE --quantity QTY --client-order-id ID".to_string(),
        "  market-buy --symbol SYM --quantity QTY --client-order-id ID".to_string(),
        "  market-sell --symbol SYM --quantity QTY --client-order-id ID".to_string(),
        "  cancel --symbol SYM --order-id ID".to_string(),
        "  modify --symbol SYM --order-id ID --price PRICE --quantity QTY".to_string(),
        "  orders --symbol SYM".to_string(),
        "  book --symbol SYM".to_string(),
        "  trades --symbol SYM".to_string(),
        "  clear".to_string(),
        "  reset".to_string(),
        "  help".to_string(),
        "  exit".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buy_args(symbol: &str, price: &str, quantity: u64, client_order_id: u64) -> LimitOrderArgs {
        LimitOrderArgs {
            symbol: symbol.to_string(),
            price: parse_price(price).unwrap(),
            quantity,
            client_order_id,
        }
    }

    #[test]
    fn submit_limit_reports_a_resting_order() {
        let mut session = Session::new();
        let lines = submit_limit(&mut session, &buy_args("AAPL", "100.00", 10, 1), Side::Buy);
        assert!(lines[0].starts_with("order 1 submitted"));
        assert_eq!(lines.last().unwrap(), "remaining 10 resting on book");
    }

    #[test]
    fn cancel_unknown_symbol_does_not_create_an_engine() {
        let mut session = Session::new();
        let lines = cancel(
            &mut session,
            &CancelArgs {
                symbol: "AAPL".to_string(),
                order_id: 1,
            },
        );
        assert!(lines[0].starts_with("error:"));
        assert_eq!(session.symbol_count(), 0);
    }

    #[test]
    fn list_orders_reports_none_for_an_untraded_symbol() {
        let session = Session::new();
        let lines = list_orders(
            &session,
            &SymbolArgs {
                symbol: "AAPL".to_string(),
            },
        );
        assert_eq!(lines, vec!["no orders for AAPL"]);
    }

    #[test]
    fn show_book_lists_asks_ascending_and_bids_descending() {
        let mut session = Session::new();
        submit_limit(&mut session, &buy_args("AAPL", "100.00", 5, 1), Side::Buy);
        submit_limit(&mut session, &buy_args("AAPL", "99.00", 5, 2), Side::Buy);
        submit_limit(&mut session, &buy_args("AAPL", "101.00", 5, 3), Side::Sell);

        let lines = show_book(
            &session,
            &SymbolArgs {
                symbol: "AAPL".to_string(),
            },
        );

        assert_eq!(
            lines,
            vec![
                "book for AAPL".to_string(),
                "asks:".to_string(),
                "  101.00 x 5".to_string(),
                "bids:".to_string(),
                "  100.00 x 5".to_string(),
                "  99.00 x 5".to_string(),
            ]
        );
    }

    #[test]
    fn reset_reports_the_number_of_symbols_cleared() {
        let mut session = Session::new();
        submit_limit(&mut session, &buy_args("AAPL", "100.00", 5, 1), Side::Buy);
        submit_limit(&mut session, &buy_args("MSFT", "200.00", 5, 2), Side::Buy);

        let lines = reset(&mut session);
        assert_eq!(lines, vec!["session reset, cleared 2 symbols"]);
        assert_eq!(session.symbol_count(), 0);
    }

    #[test]
    fn help_lines_lists_every_command() {
        let lines = help_lines();
        assert!(lines.iter().any(|line| line.contains("buy")));
        assert!(lines.iter().any(|line| line.contains("exit")));
    }
}
