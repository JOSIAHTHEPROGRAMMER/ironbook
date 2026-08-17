//! The order book, storing resting limit orders indexed by price and time.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::errors::{OrderBookError, OrderBookResult};
use crate::orders::{Order, OrderType};
use crate::types::{ClientOrderId, OrderId, Price, Side};

/// Resting limit orders for a single symbol, organized by price and
/// time priority.
///
/// Order data lives once in `orders`, price levels only store the
/// `OrderId`s resting at that price, in arrival order. This keeps
/// cancellation and lookups from needing to duplicate or synchronize
/// order data across two places.
#[derive(Debug, Default)]
pub struct OrderBook {
    orders: HashMap<OrderId, Order>,
    bids: BTreeMap<Price, VecDeque<OrderId>>,
    asks: BTreeMap<Price, VecDeque<OrderId>>,
    client_order_ids: HashSet<ClientOrderId>,
}

impl OrderBook {
    /// Creates an empty order book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a limit order to rest on the book.
    ///
    /// # Errors
    ///
    /// Returns `OrderBookError::MarketOrderCannotRest` if `order` is a
    /// market order, or `OrderBookError::DuplicateClientOrderId` if its
    /// client order id already has a resting order.
    ///
    /// # Panics
    ///
    /// Panics if `order` has no price after the market order check
    /// above has passed. This can only happen if `Order` is changed to
    /// allow a limit order without a price, which would itself be a bug
    /// in `orders.rs`, not something this function can receive as
    /// ordinary input.
    pub fn insert(&mut self, order: Order) -> OrderBookResult<()> {
        if order.order_type() == OrderType::Market {
            return Err(OrderBookError::MarketOrderCannotRest);
        }
        if self.client_order_ids.contains(&order.client_order_id()) {
            return Err(OrderBookError::DuplicateClientOrderId(
                order.client_order_id(),
            ));
        }

        // Market orders were already rejected above, every order reaching
        // this point is a limit order, which always carries a price.
        let price = order.price().expect("limit orders always have a price");
        let id = order.id();
        let side = order.side();

        self.client_order_ids.insert(order.client_order_id());
        self.orders.insert(id, order);
        self.book_for_mut(side)
            .entry(price)
            .or_default()
            .push_back(id);

        Ok(())
    }

    /// Removes a resting order from the book and returns it.
    ///
    /// # Errors
    ///
    /// Returns `OrderBookError::UnknownOrder` if no resting order has
    /// this id.
    ///
    /// # Panics
    ///
    /// Panics if an order stored in `orders` is missing its price, or
    /// if its price level or its own id within that level cannot be
    /// found. All three would mean `insert` and `cancel` have fallen
    /// out of sync with each other, an internal bug, not ordinary
    /// input the caller could trigger.
    pub fn cancel(&mut self, order_id: OrderId) -> OrderBookResult<Order> {
        let order = self
            .orders
            .remove(&order_id)
            .ok_or(OrderBookError::UnknownOrder(order_id))?;

        let price = order
            .price()
            .expect("orders stored on the book always have a price");
        let queue = self
            .book_for_mut(order.side())
            .get_mut(&price)
            .expect("a stored order's price level must exist");
        let position = queue
            .iter()
            .position(|id| *id == order_id)
            .expect("a stored order's id must be present in its price level");
        queue.remove(position);

        if queue.is_empty() {
            self.book_for_mut(order.side()).remove(&price);
        }
        self.client_order_ids.remove(&order.client_order_id());

        Ok(order)
    }

    /// Returns a resting order by id, without removing it.
    #[must_use]
    pub fn get(&self, order_id: OrderId) -> Option<&Order> {
        self.orders.get(&order_id)
    }

    /// Returns a mutable reference to a resting order by id.
    ///
    /// Used by the matching engine to apply a fill directly to a maker
    /// order still resting on the book, without removing and
    /// reinserting it.
    pub fn get_mut(&mut self, order_id: OrderId) -> Option<&mut Order> {
        self.orders.get_mut(&order_id)
    }

    /// Returns true if a resting order already uses this client order id.
    ///
    /// Exposed so the matching engine can reject a duplicate before
    /// matching starts, checking only at insertion time would be too
    /// late, by then trades may have already executed against other
    /// orders and cannot be undone.
    #[must_use]
    pub fn contains_client_order_id(&self, client_order_id: ClientOrderId) -> bool {
        self.client_order_ids.contains(&client_order_id)
    }

    /// Returns the best price on the given side, the highest bid or the
    /// lowest ask.
    #[must_use]
    pub fn best_price(&self, side: Side) -> Option<Price> {
        match side {
            Side::Buy => self.bids.last_key_value().map(|(price, _)| *price),
            Side::Sell => self.asks.first_key_value().map(|(price, _)| *price),
        }
    }

    /// Returns the order ids resting at a specific price level, in
    /// arrival order.
    #[must_use]
    pub fn price_level(&self, side: Side, price: Price) -> Option<&VecDeque<OrderId>> {
        self.book_for(side).get(&price)
    }

    /// Returns every resting order, in no particular order.
    ///
    /// For listing all resting orders regardless of price or side.
    /// Callers that care about price time priority should use
    /// `price_level` instead, this makes no ordering guarantee.
    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.values()
    }

    /// Returns every price level on a side, in ascending price order,
    /// each with the order ids resting there in arrival order.
    ///
    /// Ascending is how the underlying map iterates naturally. For the
    /// bid side the best price is the last entry, not the first,
    /// callers that want best price first should reverse the iterator.
    #[must_use]
    pub fn price_levels(
        &self,
        side: Side,
    ) -> impl DoubleEndedIterator<Item = (Price, &VecDeque<OrderId>)> {
        self.book_for(side)
            .iter()
            .map(|(&price, orders)| (price, orders))
    }

    /// Returns true if no orders are resting on the book.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Returns the total number of resting orders across both sides.
    #[must_use]
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }

    fn book_for(&self, side: Side) -> &BTreeMap<Price, VecDeque<OrderId>> {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    fn book_for_mut(&mut self, side: Side) -> &mut BTreeMap<Price, VecDeque<OrderId>> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Quantity;

    fn limit_order(sequence: u64, client_id: u64, side: Side, price: i64) -> Order {
        Order::new_limit(
            OrderId::from_sequence(sequence),
            ClientOrderId::from_raw(client_id),
            crate::types::Symbol::new("AAPL"),
            side,
            Price::from_ticks(price),
            Quantity::from_units(10),
        )
        .unwrap()
    }

    fn market_order(sequence: u64, client_id: u64, side: Side) -> Order {
        Order::new_market(
            OrderId::from_sequence(sequence),
            ClientOrderId::from_raw(client_id),
            crate::types::Symbol::new("AAPL"),
            side,
            Quantity::from_units(10),
        )
        .unwrap()
    }

    #[test]
    fn insert_rejects_market_order() {
        let mut book = OrderBook::new();
        let result = book.insert(market_order(1, 1, Side::Buy));
        assert_eq!(result.unwrap_err(), OrderBookError::MarketOrderCannotRest);
    }

    #[test]
    fn insert_rejects_duplicate_client_order_id() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        let result = book.insert(limit_order(2, 1, Side::Buy, 101));
        assert_eq!(
            result.unwrap_err(),
            OrderBookError::DuplicateClientOrderId(ClientOrderId::from_raw(1))
        );
    }

    #[test]
    fn insert_and_get_round_trip() {
        let mut book = OrderBook::new();
        let id = limit_order(1, 1, Side::Buy, 100).id();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        assert!(book.get(id).is_some());
    }

    #[test]
    fn cancel_removes_order_and_returns_it() {
        let mut book = OrderBook::new();
        let id = OrderId::from_sequence(1);
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();

        let cancelled = book.cancel(id).unwrap();
        assert_eq!(cancelled.id(), id);
        assert!(book.get(id).is_none());
        assert!(book.is_empty());
    }

    #[test]
    fn cancel_unknown_order_returns_error() {
        let mut book = OrderBook::new();
        let result = book.cancel(OrderId::from_sequence(99));
        assert_eq!(
            result.unwrap_err(),
            OrderBookError::UnknownOrder(OrderId::from_sequence(99))
        );
    }

    #[test]
    fn cancel_removes_empty_price_level() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.cancel(OrderId::from_sequence(1)).unwrap();
        assert_eq!(book.best_price(Side::Buy), None);
    }

    #[test]
    fn best_price_buy_returns_highest_bid() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Buy, 150)).unwrap();
        book.insert(limit_order(3, 3, Side::Buy, 120)).unwrap();
        assert_eq!(book.best_price(Side::Buy), Some(Price::from_ticks(150)));
    }

    #[test]
    fn best_price_sell_returns_lowest_ask() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Sell, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Sell, 90)).unwrap();
        book.insert(limit_order(3, 3, Side::Sell, 110)).unwrap();
        assert_eq!(book.best_price(Side::Sell), Some(Price::from_ticks(90)));
    }

    #[test]
    fn price_level_preserves_fifo_arrival_order() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Buy, 100)).unwrap();
        book.insert(limit_order(3, 3, Side::Buy, 100)).unwrap();

        let level: Vec<OrderId> = book
            .price_level(Side::Buy, Price::from_ticks(100))
            .unwrap()
            .iter()
            .copied()
            .collect();

        assert_eq!(
            level,
            vec![
                OrderId::from_sequence(1),
                OrderId::from_sequence(2),
                OrderId::from_sequence(3),
            ]
        );
    }

    #[test]
    fn order_count_tracks_both_sides() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Sell, 101)).unwrap();
        assert_eq!(book.order_count(), 2);
    }

    #[test]
    fn orders_iterates_every_resting_order_regardless_of_side() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Sell, 101)).unwrap();

        let mut ids: Vec<OrderId> = book.orders().map(Order::id).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![OrderId::from_sequence(1), OrderId::from_sequence(2)]
        );
    }

    #[test]
    fn price_levels_iterates_in_ascending_price_order() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Buy, 90)).unwrap();
        book.insert(limit_order(3, 3, Side::Buy, 110)).unwrap();

        let prices: Vec<Price> = book
            .price_levels(Side::Buy)
            .map(|(price, _)| price)
            .collect();
        assert_eq!(
            prices,
            vec![
                Price::from_ticks(90),
                Price::from_ticks(100),
                Price::from_ticks(110)
            ]
        );
    }

    #[test]
    fn price_levels_can_be_reversed_for_best_price_first() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Buy, 110)).unwrap();

        let best_first: Vec<Price> = book
            .price_levels(Side::Buy)
            .rev()
            .map(|(price, _)| price)
            .collect();
        assert_eq!(
            best_first,
            vec![Price::from_ticks(110), Price::from_ticks(100)]
        );
    }

    #[test]
    fn get_mut_allows_applying_a_fill_in_place() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();

        let order = book.get_mut(OrderId::from_sequence(1)).unwrap();
        order.fill(Quantity::from_units(4));

        let order = book.get(OrderId::from_sequence(1)).unwrap();
        assert_eq!(order.remaining_quantity(), Quantity::from_units(6));
    }

    #[test]
    fn contains_client_order_id_reflects_resting_orders() {
        let mut book = OrderBook::new();
        let client_id = ClientOrderId::from_raw(1);
        assert!(!book.contains_client_order_id(client_id));

        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        assert!(book.contains_client_order_id(client_id));

        book.cancel(OrderId::from_sequence(1)).unwrap();
        assert!(!book.contains_client_order_id(client_id));
    }
}
