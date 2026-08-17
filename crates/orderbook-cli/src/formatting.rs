//! Converts between human typed decimal prices and the integer tick
//! representation `orderbook-core` uses internally.
//!
//! This intentionally lives in the CLI crate, not `orderbook-core`.
//! The core library has no opinion on tick size or decimal display,
//! that is a presentation concern, this module is where it belongs.

use orderbook_core::types::Price;

/// The number of ticks in one full unit of price, fixed at two decimal
/// places until real per symbol configuration exists.
const TICKS_PER_UNIT: i64 = 100;

/// Parses a decimal price string like `"100.50"` into ticks.
///
/// Works on the string's digits directly rather than parsing as a
/// float and multiplying, floating point cannot reliably represent
/// every two decimal value, doing the conversion with floats would
/// undermine the entire reason `Price` is an integer in the first
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
}
