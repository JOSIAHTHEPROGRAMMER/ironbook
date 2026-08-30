//! The matching engine, turning incoming orders into trades against a
//! single symbol's order book.

use std::cmp::min;
use std::time::Instant;

use crate::errors::{MatchingError, MatchingResult, OrderBookError};
use crate::metrics::Metrics;
use crate::orderbook::OrderBook;
use crate::orders::Order;
use crate::trade::Trade;
use crate::types::{ClientOrderId, OrderId, Price, Quantity, Side, Symbol, TradeId};

/// The result of submitting an order to the matching engine.
///
/// An order can be filled completely, partially, or not at all, and a
/// limit order that is not fully filled may or may not end up resting
/// on the book, a market order never rests. This type reports all of
/// that back to the caller in one place instead of forcing it to infer
/// what happened from the trade list alone.
#[derive(Debug, Clone)]
pub struct ExecutionReport {
    order_id: OrderId,
    trades: Vec<Trade>,
    remaining_quantity: Quantity,
    resting: bool,
}

impl ExecutionReport {
    /// Returns the engine assigned id of the order that was submitted.
    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        self.order_id
    }

    /// Returns the trades produced while matching this order.
    #[must_use]
    pub fn trades(&self) -> &[Trade] {
        &self.trades
    }

    /// Returns the quantity left unfilled after matching.
    #[must_use]
    pub const fn remaining_quantity(&self) -> Quantity {
        self.remaining_quantity
    }

    /// Returns true if the unfilled remainder was placed on the book.
    #[must_use]
    pub const fn resting(&self) -> bool {
        self.resting
    }
}

/// Matches incoming orders against a single symbol's order book,
/// producing trades according to price time priority.
#[derive(Debug)]
pub struct MatchingEngine {
    symbol: Symbol,
    book: OrderBook,
    trade_history: Vec<Trade>,
    next_order_sequence: u64,
    next_trade_sequence: u64,
    metrics: Metrics,
}

impl MatchingEngine {
    /// Creates an engine for a single symbol, with an empty book.
    #[must_use]
    pub fn new(symbol: Symbol) -> Self {
        Self {
            symbol,
            book: OrderBook::new(),
            trade_history: Vec::new(),
            next_order_sequence: 1,
            next_trade_sequence: 1,
            metrics: Metrics::new(),
        }
    }

    /// Returns the symbol this engine matches orders for.
    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    /// Returns the current state of the order book.
    #[must_use]
    pub const fn book(&self) -> &OrderBook {
        &self.book
    }

    /// Returns every trade this engine has produced.
    #[must_use]
    pub fn trade_history(&self) -> &[Trade] {
        &self.trade_history
    }

    /// Returns the activity counters and latency histograms recorded
    /// so far.
    #[must_use]
    pub const fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Submits a limit order, matching it against the book immediately
    /// and resting whatever remains unfilled.
    ///
    /// # Errors
    ///
    /// Returns `MatchingError::InvalidOrder` if `price` or `quantity`
    /// fail order validation, or `MatchingError::OrderBook` if
    /// `client_order_id` already has a resting order.
    pub fn submit_limit_order(
        &mut self,
        client_order_id: ClientOrderId,
        side: Side,
        price: Price,
        quantity: Quantity,
    ) -> MatchingResult<ExecutionReport> {
        let submit_started_at = Instant::now();

        if let Err(error) = self.ensure_client_order_id_available(client_order_id) {
            self.metrics.record_order_rejected();
            return Err(error);
        }

        let order_id = self.next_order_id();
        let mut order = match Order::new_limit(
            order_id,
            client_order_id,
            self.symbol.clone(),
            side,
            price,
            quantity,
        ) {
            Ok(order) => order,
            Err(error) => {
                self.metrics.record_order_rejected();
                return Err(error.into());
            }
        };

        let match_started_at = Instant::now();
        let trades = self.match_order(&mut order);
        self.metrics
            .record_match_latency(match_started_at.elapsed());

        let remaining_quantity = order.remaining_quantity();
        let resting = !order.is_fully_filled();

        if resting {
            self.book.insert(order)?;
        }

        self.metrics.record_order_submitted();
        self.metrics.record_trades(trades.len());
        self.metrics
            .record_submit_latency(submit_started_at.elapsed());

        Ok(ExecutionReport {
            order_id,
            trades,
            remaining_quantity,
            resting,
        })
    }

    /// Submits a market order, matching it immediately against whatever
    /// liquidity is available.
    ///
    /// A market order never rests on the book. If the book cannot fill
    /// it completely, the unfilled remainder is simply not matched,
    /// this is normal exchange behavior, not an error.
    ///
    /// # Errors
    ///
    /// Returns `MatchingError::InvalidOrder` if `quantity` fails order
    /// validation, or `MatchingError::OrderBook` if `client_order_id`
    /// already has a resting order.
    pub fn submit_market_order(
        &mut self,
        client_order_id: ClientOrderId,
        side: Side,
        quantity: Quantity,
    ) -> MatchingResult<ExecutionReport> {
        let submit_started_at = Instant::now();

        if let Err(error) = self.ensure_client_order_id_available(client_order_id) {
            self.metrics.record_order_rejected();
            return Err(error);
        }

        let order_id = self.next_order_id();
        let mut order = match Order::new_market(
            order_id,
            client_order_id,
            self.symbol.clone(),
            side,
            quantity,
        ) {
            Ok(order) => order,
            Err(error) => {
                self.metrics.record_order_rejected();
                return Err(error.into());
            }
        };

        let match_started_at = Instant::now();
        let trades = self.match_order(&mut order);
        self.metrics
            .record_match_latency(match_started_at.elapsed());

        let remaining_quantity = order.remaining_quantity();

        self.metrics.record_order_submitted();
        self.metrics.record_trades(trades.len());
        self.metrics
            .record_submit_latency(submit_started_at.elapsed());

        Ok(ExecutionReport {
            order_id,
            trades,
            remaining_quantity,
            resting: false,
        })
    }

    /// Cancels a resting order and returns it.
    ///
    /// # Errors
    ///
    /// Returns `MatchingError::OrderBook` wrapping
    /// `OrderBookError::UnknownOrder` if no resting order has this id.
    pub fn cancel_order(&mut self, order_id: OrderId) -> MatchingResult<Order> {
        let cancelled = self.book.cancel(order_id)?;
        self.metrics.record_order_cancelled();
        Ok(cancelled)
    }

    /// Replaces a resting limit order's price and quantity.
    ///
    /// Implemented as validate, then cancel, then resubmit. The new
    /// price and quantity are checked before the existing order is
    /// touched, so a rejected modify never destroys the order it was
    /// trying to replace. The replacement receives a new engine
    /// assigned id and goes to the back of its price level's queue, it
    /// loses time priority exactly as it would on a real exchange, only
    /// the caller's client order id carries over. If the new price
    /// crosses the book, the replacement can produce trades
    /// immediately, the same as any other submitted order.
    ///
    /// Records no metrics of its own. The `cancel_order` and
    /// `submit_limit_order` calls this delegates to already record the
    /// cancellation and the resubmission, recording them again here
    /// would count the same activity twice.
    ///
    /// # Errors
    ///
    /// Returns `MatchingError::InvalidOrder` if `new_price` or
    /// `new_quantity` fail validation, before anything on the book is
    /// touched. Returns `MatchingError::OrderBook` wrapping
    /// `OrderBookError::UnknownOrder` if no resting order has `order_id`.
    pub fn modify_order(
        &mut self,
        order_id: OrderId,
        new_price: Price,
        new_quantity: Quantity,
    ) -> MatchingResult<ExecutionReport> {
        Order::validate_limit_inputs(new_price, new_quantity)?;

        let existing = self.cancel_order(order_id)?;

        self.submit_limit_order(
            existing.client_order_id(),
            existing.side(),
            new_price,
            new_quantity,
        )
    }

    fn ensure_client_order_id_available(
        &self,
        client_order_id: ClientOrderId,
    ) -> MatchingResult<()> {
        if self.book.contains_client_order_id(client_order_id) {
            return Err(MatchingError::from(OrderBookError::DuplicateClientOrderId(
                client_order_id,
            )));
        }
        Ok(())
    }

    fn match_order(&mut self, taker: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        let opposite_side = taker.side().opposite();

        while !taker.is_fully_filled() {
            let Some(best_price) = self.book.best_price(opposite_side) else {
                break;
            };

            if let Some(limit_price) = taker.price()
                && !prices_cross(taker.side(), limit_price, best_price)
            {
                break;
            }

            let maker_id = self.first_resting_order_id(opposite_side, best_price);
            trades.push(self.match_against_maker(taker, best_price, maker_id));
        }

        trades
    }

    fn first_resting_order_id(&self, side: Side, price: Price) -> OrderId {
        *self
            .book
            .price_level(side, price)
            .and_then(std::collections::VecDeque::front)
            .expect("best_price only returns a price that has a resting order")
    }

    fn match_against_maker(&mut self, taker: &mut Order, price: Price, maker_id: OrderId) -> Trade {
        let maker_remaining = self
            .book
            .get(maker_id)
            .expect("maker id was just read from this price level")
            .remaining_quantity();
        let fill_quantity = min(taker.remaining_quantity(), maker_remaining);

        taker.fill(fill_quantity);

        let maker = self
            .book
            .get_mut(maker_id)
            .expect("maker id was just read from this price level");
        maker.fill(fill_quantity);
        let maker_fully_filled = maker.is_fully_filled();

        if maker_fully_filled {
            self.book
                .cancel(maker_id)
                .expect("maker id was just read from this price level");
        }

        let trade = Trade::new(
            self.next_trade_id(),
            self.symbol.clone(),
            price,
            fill_quantity,
            maker_id,
            taker.id(),
            taker.side(),
        );
        self.trade_history.push(trade.clone());
        trade
    }

    fn next_order_id(&mut self) -> OrderId {
        let id = OrderId::from_sequence(self.next_order_sequence);
        self.next_order_sequence += 1;
        id
    }

    fn next_trade_id(&mut self) -> TradeId {
        let id = TradeId::from_sequence(self.next_trade_sequence);
        self.next_trade_sequence += 1;
        id
    }
}

/// Returns true if a taker's limit price is aggressive enough to trade
/// against the best opposite price.
///
/// A buy crosses if it is willing to pay at least the best ask, a sell
/// crosses if it is willing to accept at most the best bid.
fn prices_cross(taker_side: Side, taker_limit_price: Price, best_opposite_price: Price) -> bool {
    match taker_side {
        Side::Buy => taker_limit_price >= best_opposite_price,
        Side::Sell => taker_limit_price <= best_opposite_price,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> MatchingEngine {
        MatchingEngine::new(Symbol::new("AAPL"))
    }

    #[test]
    fn resting_limit_order_produces_no_trades() {
        let mut engine = engine();
        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        assert!(report.trades().is_empty());
        assert!(report.resting());
        assert_eq!(report.remaining_quantity(), Quantity::from_units(10));
    }

    #[test]
    fn crossing_limit_order_with_equal_quantity_fully_fills_both() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        assert_eq!(report.trades().len(), 1);
        assert!(!report.resting());
        assert_eq!(report.remaining_quantity(), Quantity::from_units(0));
        assert!(engine.book().is_empty());
    }

    #[test]
    fn trade_executes_at_the_maker_price_not_the_taker_price() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(110),
                Quantity::from_units(10),
            )
            .unwrap();

        assert_eq!(report.trades()[0].price(), Price::from_ticks(100));
    }

    #[test]
    fn larger_taker_partially_fills_and_rests_the_remainder() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(4),
            )
            .unwrap();

        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        assert_eq!(report.trades().len(), 1);
        assert!(report.resting());
        assert_eq!(report.remaining_quantity(), Quantity::from_units(6));
        assert_eq!(
            engine.book().best_price(Side::Buy),
            Some(Price::from_ticks(100))
        );
    }

    #[test]
    fn larger_maker_partially_fills_taker_and_keeps_resting() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(4),
            )
            .unwrap();

        assert!(!report.resting());
        assert_eq!(report.remaining_quantity(), Quantity::from_units(0));

        let maker = engine.book().get(OrderId::from_sequence(1)).unwrap();
        assert_eq!(maker.remaining_quantity(), Quantity::from_units(6));
    }

    #[test]
    fn fifo_within_a_price_level_fills_earlier_orders_first() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(3),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(6),
            )
            .unwrap();

        assert_eq!(report.trades().len(), 2);
        assert_eq!(
            report.trades()[0].maker_order_id(),
            OrderId::from_sequence(1)
        );
        assert_eq!(report.trades()[0].quantity(), Quantity::from_units(5));
        assert_eq!(
            report.trades()[1].maker_order_id(),
            OrderId::from_sequence(2)
        );
        assert_eq!(report.trades()[1].quantity(), Quantity::from_units(1));
    }

    #[test]
    fn better_price_fills_before_an_earlier_order_at_a_worse_price() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(105),
                Quantity::from_units(5),
            )
            .unwrap();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(3),
                Side::Buy,
                Price::from_ticks(105),
                Quantity::from_units(5),
            )
            .unwrap();

        assert_eq!(
            report.trades()[0].maker_order_id(),
            OrderId::from_sequence(2)
        );
        assert_eq!(report.trades()[0].price(), Price::from_ticks(100));
    }

    #[test]
    fn non_crossing_limit_order_rests_without_matching() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let report = engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(90),
                Quantity::from_units(5),
            )
            .unwrap();

        assert!(report.trades().is_empty());
        assert!(report.resting());
    }

    #[test]
    fn market_order_matches_at_the_resting_price() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let report = engine
            .submit_market_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Quantity::from_units(5),
            )
            .unwrap();

        assert_eq!(report.trades().len(), 1);
        assert_eq!(report.trades()[0].price(), Price::from_ticks(100));
        assert!(!report.resting());
    }

    #[test]
    fn market_order_without_liquidity_matches_nothing_and_does_not_rest() {
        let mut engine = engine();
        let report = engine
            .submit_market_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Quantity::from_units(5),
            )
            .unwrap();

        assert!(report.trades().is_empty());
        assert!(!report.resting());
        assert_eq!(report.remaining_quantity(), Quantity::from_units(5));
    }

    #[test]
    fn duplicate_client_order_id_is_rejected_before_matching() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let result = engine.submit_limit_order(
            ClientOrderId::from_raw(1),
            Side::Sell,
            Price::from_ticks(100),
            Quantity::from_units(5),
        );

        assert_eq!(
            result.unwrap_err(),
            MatchingError::from(OrderBookError::DuplicateClientOrderId(
                ClientOrderId::from_raw(1)
            ))
        );
        assert!(engine.trade_history().is_empty());
    }

    #[test]
    fn invalid_order_input_is_rejected_before_matching() {
        let mut engine = engine();
        let result = engine.submit_limit_order(
            ClientOrderId::from_raw(1),
            Side::Buy,
            Price::from_ticks(0),
            Quantity::from_units(5),
        );

        assert!(matches!(result, Err(MatchingError::InvalidOrder(_))));
    }

    #[test]
    fn cancel_order_removes_a_resting_order() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let cancelled = engine.cancel_order(OrderId::from_sequence(1)).unwrap();
        assert_eq!(cancelled.id(), OrderId::from_sequence(1));
        assert!(engine.book().is_empty());
    }

    #[test]
    fn cancel_order_unknown_id_returns_error() {
        let mut engine = engine();
        let result = engine.cancel_order(OrderId::from_sequence(99));
        assert!(matches!(result, Err(MatchingError::OrderBook(_))));
    }

    #[test]
    fn modify_order_replaces_price_and_quantity() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let report = engine
            .modify_order(
                OrderId::from_sequence(1),
                Price::from_ticks(105),
                Quantity::from_units(8),
            )
            .unwrap();

        assert_eq!(
            engine.book().best_price(Side::Buy),
            Some(Price::from_ticks(105))
        );
        assert_eq!(report.remaining_quantity(), Quantity::from_units(8));
        assert!(report.resting());
    }

    #[test]
    fn modify_order_gets_a_new_id_and_loses_time_priority() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let report = engine
            .modify_order(
                OrderId::from_sequence(1),
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        assert_ne!(report.order_id(), OrderId::from_sequence(1));

        let level: Vec<OrderId> = engine
            .book()
            .price_level(Side::Buy, Price::from_ticks(100))
            .unwrap()
            .iter()
            .copied()
            .collect();
        assert_eq!(level, vec![OrderId::from_sequence(2), report.order_id()]);
    }

    #[test]
    fn modify_order_can_immediately_cross_and_produce_trades() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(90),
                Quantity::from_units(5),
            )
            .unwrap();

        let report = engine
            .modify_order(
                OrderId::from_sequence(2),
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        assert_eq!(report.trades().len(), 1);
        assert!(!report.resting());
    }

    #[test]
    fn modify_order_unknown_id_returns_error() {
        let mut engine = engine();
        let result = engine.modify_order(
            OrderId::from_sequence(99),
            Price::from_ticks(100),
            Quantity::from_units(5),
        );
        assert!(matches!(result, Err(MatchingError::OrderBook(_))));
    }

    #[test]
    fn modify_order_rejects_invalid_replacement_without_destroying_the_original() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let result = engine.modify_order(
            OrderId::from_sequence(1),
            Price::from_ticks(100),
            Quantity::from_units(0),
        );

        assert!(matches!(result, Err(MatchingError::InvalidOrder(_))));

        let original = engine.book().get(OrderId::from_sequence(1)).unwrap();
        assert_eq!(original.remaining_quantity(), Quantity::from_units(5));
        assert_eq!(original.price(), Some(Price::from_ticks(100)));
    }

    #[test]
    fn prices_cross_buy_requires_limit_at_or_above_best_ask() {
        assert!(prices_cross(
            Side::Buy,
            Price::from_ticks(100),
            Price::from_ticks(100)
        ));
        assert!(prices_cross(
            Side::Buy,
            Price::from_ticks(101),
            Price::from_ticks(100)
        ));
        assert!(!prices_cross(
            Side::Buy,
            Price::from_ticks(99),
            Price::from_ticks(100)
        ));
    }

    #[test]
    fn prices_cross_sell_requires_limit_at_or_below_best_bid() {
        assert!(prices_cross(
            Side::Sell,
            Price::from_ticks(100),
            Price::from_ticks(100)
        ));
        assert!(prices_cross(
            Side::Sell,
            Price::from_ticks(99),
            Price::from_ticks(100)
        ));
        assert!(!prices_cross(
            Side::Sell,
            Price::from_ticks(101),
            Price::from_ticks(100)
        ));
    }

    #[test]
    fn successful_submit_records_order_submitted_and_trade_counts() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Sell,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(2),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        assert_eq!(engine.metrics().orders_submitted(), 2);
        assert_eq!(engine.metrics().trades_executed(), 1);
        assert_eq!(engine.metrics().orders_rejected(), 0);
    }

    #[test]
    fn rejected_submit_records_a_rejection_not_a_submission() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let _ = engine.submit_limit_order(
            ClientOrderId::from_raw(1),
            Side::Sell,
            Price::from_ticks(100),
            Quantity::from_units(5),
        );

        assert_eq!(engine.metrics().orders_submitted(), 1);
        assert_eq!(engine.metrics().orders_rejected(), 1);
    }

    #[test]
    fn invalid_order_input_records_a_rejection() {
        let mut engine = engine();
        let _ = engine.submit_limit_order(
            ClientOrderId::from_raw(1),
            Side::Buy,
            Price::from_ticks(0),
            Quantity::from_units(5),
        );

        assert_eq!(engine.metrics().orders_rejected(), 1);
        assert_eq!(engine.metrics().orders_submitted(), 0);
    }

    #[test]
    fn cancel_records_a_cancellation_only_on_success() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        let _ = engine.cancel_order(OrderId::from_sequence(99));
        assert_eq!(engine.metrics().orders_cancelled(), 0);

        engine.cancel_order(OrderId::from_sequence(1)).unwrap();
        assert_eq!(engine.metrics().orders_cancelled(), 1);
    }

    #[test]
    fn modify_records_exactly_one_cancel_and_one_submit() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        engine
            .modify_order(
                OrderId::from_sequence(1),
                Price::from_ticks(105),
                Quantity::from_units(8),
            )
            .unwrap();

        assert_eq!(engine.metrics().orders_submitted(), 2);
        assert_eq!(engine.metrics().orders_cancelled(), 1);
    }

    #[test]
    fn submit_and_match_latency_histograms_receive_samples() {
        let mut engine = engine();
        engine
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(5),
            )
            .unwrap();

        assert_eq!(engine.metrics().submit_latency().count(), 1);
        assert_eq!(engine.metrics().match_latency().count(), 1);
    }
}
