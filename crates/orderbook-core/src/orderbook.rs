//! The order book, storing resting limit orders indexed by price and time.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::errors::{OrderBookError, OrderBookResult};
use crate::orders::{Order, OrderType};
use crate::types::{ClientOrderId, OrderId, Price, Side};

/// Index into a price level's node arena.
///
/// Private to this module, an implementation detail of how
/// `PriceLevelQueue` locates nodes in O(1), not a domain concept
/// anything outside this file needs to know about.
type NodeIndex = usize;

/// One entry in a price level's arrival order queue.
///
/// Forms a doubly linked list threaded through a `Vec`, `prev` and
/// `next` are indices into that `Vec` rather than pointers, this is
/// the standard safe Rust substitute for an intrusive linked list,
/// giving O(1) removal from the middle of the list without `unsafe`.
#[derive(Debug, Clone, Copy)]
struct Node {
    order_id: OrderId,
    prev: Option<NodeIndex>,
    next: Option<NodeIndex>,
}

/// A FIFO queue of order ids resting at one price level, supporting
/// O(1) removal from anywhere in the queue given the node index
/// `OrderBook` already tracks for each order.
///
/// Removing an order used to mean scanning the queue to find its
/// position first, an O(k) cost in the number of orders at that price
/// level, this was the project's known, previously flagged
/// optimization target. An arena backed doubly linked list turns that
/// scan into a direct index lookup: given the node index, splicing it
/// out only touches its immediate neighbors, regardless of how many
/// other orders are queued at the same price. Freed slots are tracked
/// on a free list and reused by later insertions, so memory use stays
/// bounded by the price level's peak concurrent size rather than
/// growing with total churn over a long session.
#[derive(Debug, Default)]
pub struct PriceLevelQueue {
    nodes: Vec<Option<Node>>,
    free: Vec<NodeIndex>,
    head: Option<NodeIndex>,
    tail: Option<NodeIndex>,
    len: usize,
}

impl PriceLevelQueue {
    /// Appends an order id to the back of the queue and returns the
    /// node index `OrderBook` should retain to remove it later in
    /// O(1).
    fn push_back(&mut self, order_id: OrderId) -> NodeIndex {
        let index = self.free.pop().unwrap_or(self.nodes.len());
        let node = Node {
            order_id,
            prev: self.tail,
            next: None,
        };

        if index == self.nodes.len() {
            self.nodes.push(Some(node));
        } else {
            self.nodes[index] = Some(node);
        }

        match self.tail {
            Some(previous_tail) => {
                self.nodes[previous_tail]
                    .as_mut()
                    .expect("a stored tail index always references a live node")
                    .next = Some(index);
            }
            None => self.head = Some(index),
        }

        self.tail = Some(index);
        self.len += 1;
        index
    }

    /// Removes the node at `index` from the queue in O(1), splicing
    /// its neighbors together directly rather than scanning for it.
    ///
    /// # Panics
    ///
    /// Panics if `index` does not reference a currently live node.
    /// This can only happen if a caller passes an index this queue
    /// did not issue, or one it already removed, both are internal
    /// bugs in how `OrderBook` tracks node indices, not something
    /// `OrderBook::cancel`'s caller can trigger with ordinary input.
    fn remove(&mut self, index: NodeIndex) {
        let node = self.nodes[index]
            .take()
            .expect("index must reference a currently live node");

        match node.prev {
            Some(previous) => {
                self.nodes[previous]
                    .as_mut()
                    .expect("a live node's linked neighbor is always itself live")
                    .next = node.next;
            }
            None => self.head = node.next,
        }

        match node.next {
            Some(next) => {
                self.nodes[next]
                    .as_mut()
                    .expect("a live node's linked neighbor is always itself live")
                    .prev = node.prev;
            }
            None => self.tail = node.prev,
        }

        self.free.push(index);
        self.len -= 1;
    }

    /// Returns the order id at the front of the queue, without
    /// removing it.
    ///
    /// # Panics
    ///
    /// Panics if the queue's tracked head index does not reference a
    /// currently live node, an internal bug in this type's own
    /// bookkeeping, not something a caller can trigger.
    #[must_use]
    pub fn front(&self) -> Option<OrderId> {
        let index = self.head?;
        Some(
            self.nodes[index]
                .expect("a stored head index always references a live node")
                .order_id,
        )
    }

    /// Returns true if no orders remain in the queue.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the number of orders currently in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns an iterator over order ids in arrival order, front to
    /// back.
    #[must_use]
    pub fn iter(&self) -> PriceLevelIter<'_> {
        PriceLevelIter {
            queue: self,
            current: self.head,
        }
    }
}

/// Iterator over a price level's order ids in arrival order.
///
/// Returned by [`PriceLevelQueue::iter`].
#[derive(Debug)]
pub struct PriceLevelIter<'a> {
    queue: &'a PriceLevelQueue,
    current: Option<NodeIndex>,
}

impl Iterator for PriceLevelIter<'_> {
    type Item = OrderId;

    fn next(&mut self) -> Option<OrderId> {
        let index = self.current?;
        let node = self.queue.nodes[index].expect("iterator only ever visits currently live nodes");
        self.current = node.next;
        Some(node.order_id)
    }
}

impl<'a> IntoIterator for &'a PriceLevelQueue {
    type Item = OrderId;
    type IntoIter = PriceLevelIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Resting limit orders for a single symbol, organized by price and
/// time priority.
///
/// Order data lives once in `orders`, price levels only store the
/// `OrderId`s resting at that price, in arrival order, via
/// `PriceLevelQueue`. `node_index` tracks where each resting order's
/// id lives within its price level's queue, so `cancel` can splice it
/// out in O(1) instead of scanning the level to find it first. This
/// keeps cancellation and lookups from needing to duplicate or
/// synchronize order data across two places.
#[derive(Debug, Default)]
pub struct OrderBook {
    orders: HashMap<OrderId, Order>,
    bids: BTreeMap<Price, PriceLevelQueue>,
    asks: BTreeMap<Price, PriceLevelQueue>,
    client_order_ids: HashSet<ClientOrderId>,
    node_index: HashMap<OrderId, NodeIndex>,
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
        let node_index = self
            .book_for_mut(side)
            .entry(price)
            .or_default()
            .push_back(id);
        self.node_index.insert(id, node_index);
        self.orders.insert(id, order);

        Ok(())
    }

    /// Removes a resting order from the book and returns it.
    ///
    /// Removal from its price level is O(1): `node_index` already
    /// tracks exactly where the order's id lives in that level's
    /// queue, so this splices it out directly rather than scanning the
    /// level to find it first.
    ///
    /// # Errors
    ///
    /// Returns `OrderBookError::UnknownOrder` if no resting order has
    /// this id.
    ///
    /// # Panics
    ///
    /// Panics if an order stored in `orders` is missing its price, its
    /// node index, or its price level cannot be found. All three would
    /// mean `insert` and `cancel` have fallen out of sync with each
    /// other, an internal bug, not ordinary input the caller could
    /// trigger.
    pub fn cancel(&mut self, order_id: OrderId) -> OrderBookResult<Order> {
        let order = self
            .orders
            .remove(&order_id)
            .ok_or(OrderBookError::UnknownOrder(order_id))?;

        let price = order
            .price()
            .expect("orders stored on the book always have a price");
        let node_index = self
            .node_index
            .remove(&order_id)
            .expect("a stored order's node index must exist");
        let level = self
            .book_for_mut(order.side())
            .get_mut(&price)
            .expect("a stored order's price level must exist");
        level.remove(node_index);

        if level.is_empty() {
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
    pub fn price_level(&self, side: Side, price: Price) -> Option<&PriceLevelQueue> {
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
    ) -> impl DoubleEndedIterator<Item = (Price, &PriceLevelQueue)> {
        self.book_for(side)
            .iter()
            .map(|(&price, level)| (price, level))
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

    fn book_for(&self, side: Side) -> &BTreeMap<Price, PriceLevelQueue> {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    fn book_for_mut(&mut self, side: Side) -> &mut BTreeMap<Price, PriceLevelQueue> {
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

    #[test]
    fn cancelling_from_the_middle_of_a_deep_price_level_preserves_order_of_the_rest() {
        let mut book = OrderBook::new();
        for sequence in 1..=5u64 {
            book.insert(limit_order(sequence, sequence, Side::Buy, 100))
                .unwrap();
        }

        book.cancel(OrderId::from_sequence(3)).unwrap();

        let remaining: Vec<OrderId> = book
            .price_level(Side::Buy, Price::from_ticks(100))
            .unwrap()
            .iter()
            .collect();
        assert_eq!(
            remaining,
            vec![
                OrderId::from_sequence(1),
                OrderId::from_sequence(2),
                OrderId::from_sequence(4),
                OrderId::from_sequence(5),
            ]
        );
    }

    #[test]
    fn cancelling_every_order_in_reverse_leaves_an_empty_level() {
        let mut book = OrderBook::new();
        for sequence in 1..=20u64 {
            book.insert(limit_order(sequence, sequence, Side::Buy, 100))
                .unwrap();
        }

        for sequence in (1..=20u64).rev() {
            book.cancel(OrderId::from_sequence(sequence)).unwrap();
        }

        assert!(book.is_empty());
        assert_eq!(book.best_price(Side::Buy), None);
    }

    #[test]
    fn a_node_slot_freed_by_cancel_is_reused_by_a_later_insert() {
        let mut book = OrderBook::new();
        book.insert(limit_order(1, 1, Side::Buy, 100)).unwrap();
        book.insert(limit_order(2, 2, Side::Buy, 100)).unwrap();
        book.cancel(OrderId::from_sequence(1)).unwrap();
        book.insert(limit_order(3, 3, Side::Buy, 100)).unwrap();

        let remaining: Vec<OrderId> = book
            .price_level(Side::Buy, Price::from_ticks(100))
            .unwrap()
            .iter()
            .collect();
        assert_eq!(
            remaining,
            vec![OrderId::from_sequence(2), OrderId::from_sequence(3)]
        );
    }
}
