//! Converts between human typed decimal prices and the integer tick
//! representation [`orderbook_core`] uses internally, and formats
//! other values that need a display specific presentation, like
//! latency.
//!
//! This intentionally lives in the CLI crate, not [`orderbook_core`].
//! The core library has no opinion on tick size, decimal display, or
//! how a duration should be printed, those are presentation concerns,
//! this module is where they belong.

use std::time::Duration;

use orderbook_core::types::Price;

/// The number of ticks in one full unit of price, fixed at two decimal
/// places until real per symbol configuration exists.
const TICKS_PER_UNIT: i64 = 100;

/// Parses a decimal price string like `"100.50"` into ticks.
///
/// Works on the string's digits directly rather than parsing as a
/// float and multiplying, floating point cannot reliably represent
/// every two decimal value, doing the conversion with floats would
/// undermine the entire reason [`Price`] is an integer in the first
/// place.
///
/// # Errors
///
/// Returns a message if the input is not a valid decimal number, or
/// has more than two digits after the decimal point.
pub fn parse_price(input: &str) -> Result<Price, String> {
    let (negative, unsigned) = match input.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, input),
    };

    let mut parts = unsigned.splitn(2, '.');
    let whole_part = parts.next().unwrap_or_default();
    let fractional_part = parts.next().unwrap_or("0");

    if fractional_part.len() > 2 {
        return Err(format!(
            "price {input} has more than two decimal places, the smallest unit is 0.01"
        ));
    }
    if whole_part.is_empty() || !whole_part.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("price {input} is not a valid decimal number"));
    }
    if !fractional_part.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("price {input} is not a valid decimal number"));
    }

    let whole: i64 = whole_part
        .parse()
        .map_err(|_| format!("price {input} is not a valid decimal number"))?;
    let fractional: i64 = format!("{fractional_part:0<2}")
        .parse()
        .map_err(|_| format!("price {input} is not a valid decimal number"))?;

    let ticks = whole * TICKS_PER_UNIT + fractional;
    Ok(Price::from_ticks(if negative { -ticks } else { ticks }))
}

/// Formats a price in ticks back into a two decimal display string.
#[must_use]
pub fn format_price(price: Price) -> String {
    let ticks = price.ticks();
    let sign = if ticks < 0 { "-" } else { "" };
    let magnitude = ticks.abs();
    let whole = magnitude / TICKS_PER_UNIT;
    let fractional = magnitude % TICKS_PER_UNIT;
    format!("{sign}{whole}.{fractional:02}")
}

/// Formats a duration as a short human readable string, choosing
/// nanoseconds, microseconds, or milliseconds based on magnitude.
///
/// Matching engine latencies span several orders of magnitude within
/// the same report, a submit call might be single digit microseconds
/// while a slow outlier is several milliseconds. A fixed unit would
/// either print unreadably many digits or round the fast path down to
/// nothing, so this picks whichever unit keeps the value readable.
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos < 1_000 {
        format!("{nanos}ns")
    } else if nanos < 1_000_000 {
        format!("{:.2}\u{b5}s", duration.as_secs_f64() * 1_000_000.0)
    } else {
        format!("{:.2}ms", duration.as_secs_f64() * 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_price_reads_whole_and_fractional_parts() {
        assert_eq!(parse_price("100.50").unwrap(), Price::from_ticks(10050));
    }

    #[test]
    fn parse_price_pads_a_single_fractional_digit() {
        assert_eq!(parse_price("100.5").unwrap(), Price::from_ticks(10050));
    }

    #[test]
    fn parse_price_accepts_a_whole_number_with_no_decimal_point() {
        assert_eq!(parse_price("100").unwrap(), Price::from_ticks(10000));
    }

    #[test]
    fn parse_price_rejects_more_than_two_fractional_digits() {
        assert!(parse_price("100.505").is_err());
    }

    #[test]
    fn parse_price_rejects_non_numeric_input() {
        assert!(parse_price("abc").is_err());
        assert!(parse_price("10a.50").is_err());
    }

    #[test]
    fn parse_price_handles_a_negative_value() {
        assert_eq!(parse_price("-5.25").unwrap(), Price::from_ticks(-525));
    }

    #[test]
    fn format_price_round_trips_with_parse_price() {
        let price = parse_price("1234.07").unwrap();
        assert_eq!(format_price(price), "1234.07");
    }

    #[test]
    fn format_price_pads_a_single_digit_fractional_part() {
        assert_eq!(format_price(Price::from_ticks(10005)), "100.05");
    }

    #[test]
    fn format_price_handles_negative_values() {
        assert_eq!(format_price(Price::from_ticks(-525)), "-5.25");
    }

    #[test]
    fn format_duration_uses_nanoseconds_below_one_microsecond() {
        assert_eq!(format_duration(Duration::from_nanos(500)), "500ns");
    }

    #[test]
    fn format_duration_uses_microseconds_below_one_millisecond() {
        assert_eq!(format_duration(Duration::from_nanos(1_500)), "1.50\u{b5}s");
    }

    #[test]
    fn format_duration_uses_milliseconds_at_and_above_one_millisecond() {
        assert_eq!(format_duration(Duration::from_micros(2_500)), "2.50ms");
    }

    #[test]
    fn format_duration_handles_zero() {
        assert_eq!(format_duration(Duration::ZERO), "0ns");
    }
}
