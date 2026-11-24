//! Payload text to values of the driver's data types (`_address.._datatype`,
//! RsMqttTransformationType.hxx): the numbers and booleans. Text and blob
//! payloads are taken as they are by the driver.
//!
//! The rules:
//! - white space around the text is ignored, and one pair of quotes around
//!   it ("\"21.5\"") as well; text from a JSON string (dyn items) only loses
//!   leading white space;
//! - integers also accept integral floating point text ("12.0", "1e3") in
//!   range, and true / false;
//! - floats accept true / false (1 / 0), inf and nan;
//! - booleans are true/on/yes, false/off/no (any case) or a number (not 0).

/// `_address.._datatype` values (TransUserType + n); keep them in step with
/// driver/RsMqttTransformationType.hxx.
pub mod types {
    pub const UNDEFINED: i32 = 1000; // treated as STRING
    pub const STRING: i32 = 1001;
    pub const INT32: i32 = 1002;
    pub const UINT32: i32 = 1003;
    pub const INT64: i32 = 1004;
    pub const UINT64: i32 = 1005;
    pub const FLOAT: i32 = 1006;
    pub const BOOL: i32 = 1007;
    pub const BLOB: i32 = 1008;
}
use types::*;

/// UNDEFINED (and 0) are STRING.
pub fn normalize(t: i32) -> i32 {
    if t == UNDEFINED || t == 0 { STRING } else { t }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Int(i64),
    UInt(u64),
    Float(f64),
    Bool(bool),
}

/// The first 64 bytes of a payload for messages, in quotes.
pub fn shown(text: &[u8]) -> String {
    let head = String::from_utf8_lossy(&text[..text.len().min(64)]);
    if text.len() > 64 { format!("'{head}...'") } else { format!("'{head}'") }
}

fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r')
}

fn trim(s: &[u8]) -> &[u8] {
    let a = s.iter().position(|c| !is_c_space(*c)).unwrap_or(s.len());
    let b = s.iter().rposition(|c| !is_c_space(*c)).map_or(a, |p| p + 1);
    let s = &s[a..b.max(a)];
    // tolerate JSON strings like "\"21.5\""
    if s.len() >= 2 && s[0] == b'"' && s[s.len() - 1] == b'"' { &s[1..s.len() - 1] } else { s }
}

/// A float as std::from_chars reads it: no leading '+'.
fn plain_float(s: &str) -> Option<f64> {
    if s.starts_with('+') { None } else { s.parse().ok() }
}

fn word_bool(s: &str) -> Option<bool> {
    let l = s.to_ascii_lowercase();
    match l.as_str() {
        "true" | "on" | "yes" => Some(true),
        "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    word_bool(s).or_else(|| plain_float(s).map(|d| d != 0.0))
}

fn parse_float(s: &str) -> Option<f64> {
    if s.eq_ignore_ascii_case("true") {
        return Some(1.0);
    }
    if s.eq_ignore_ascii_case("false") {
        return Some(0.0);
    }
    plain_float(s.strip_prefix('+').unwrap_or(s))
}

/// An integer in [lo, hi_exclusive) as f64 bounds; integral floats count.
fn parse_int<T: std::str::FromStr>(s: &str, lo: f64, hi_exclusive: f64, from: impl Fn(f64) -> T) -> Option<T> {
    let t = s.strip_prefix('+').unwrap_or(s);
    if !t.starts_with('+') {
        if let Ok(v) = t.parse::<T>() {
            return Some(v);
        }
    }
    let d = parse_float(s)?;
    if !d.is_finite() || d.floor() != d || d < lo || d >= hi_exclusive {
        return None;
    }
    Some(from(d))
}

/// A number or boolean of data type `t` (INT32 .. BOOL) from payload text;
/// `json_string`: the text came from a JSON string.
pub fn parse_number(t: i32, text: &[u8], json_string: bool) -> Result<Number, String> {
    let t = normalize(t);
    let s = if json_string {
        let a = text.iter().position(|c| !is_c_space(*c)).unwrap_or(text.len());
        &text[a..]
    } else {
        trim(text)
    };
    let fail = || format!("cannot convert {} to the configured data type {t}", shown(text));
    let s = std::str::from_utf8(s).map_err(|_| fail())?;
    const I64_END: f64 = 9223372036854775808.0; // 2^63
    const U64_END: f64 = 18446744073709551616.0; // 2^64
    let v = match t {
        INT32 => parse_int(s, i32::MIN as f64, i32::MAX as f64 + 1.0, |d| d as i32).map(|v| Number::Int(v as i64)),
        UINT32 => parse_int(s, 0.0, u32::MAX as f64 + 1.0, |d| d as u32).map(|v| Number::UInt(v as u64)),
        INT64 => parse_int(s, -I64_END, I64_END, |d| d as i64).map(Number::Int),
        UINT64 => parse_int(s, 0.0, U64_END, |d| d as u64).map(Number::UInt),
        FLOAT => parse_float(s).map(Number::Float),
        BOOL => parse_bool(s).map(Number::Bool),
        _ => None,
    };
    v.ok_or_else(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(t: i32, s: &str) -> Option<Number> {
        parse_number(t, s.as_bytes(), false).ok()
    }

    #[test]
    fn integers() {
        assert_eq!(n(INT32, " 42 "), Some(Number::Int(42)));
        assert_eq!(n(INT32, "+42"), Some(Number::Int(42)));
        assert_eq!(n(INT32, "++42"), None);
        assert_eq!(n(INT32, "\"-7\""), Some(Number::Int(-7)));
        assert_eq!(n(INT32, "12.0"), Some(Number::Int(12)));
        assert_eq!(n(INT32, "1e3"), Some(Number::Int(1000)));
        assert_eq!(n(INT32, "true"), Some(Number::Int(1)));
        assert_eq!(n(INT32, "12.5"), None);
        assert_eq!(n(INT32, "2147483647"), Some(Number::Int(2147483647)));
        assert_eq!(n(INT32, "2147483648"), None);
        assert_eq!(n(INT32, "-2147483648"), Some(Number::Int(-2147483648)));
        assert_eq!(n(INT32, "2147483648.0"), None);
        assert_eq!(n(UINT32, "-1"), None);
        assert_eq!(n(UINT32, "-0"), Some(Number::UInt(0)));
        assert_eq!(n(UINT32, "4294967295"), Some(Number::UInt(4294967295)));
        assert_eq!(n(INT64, "-9223372036854775808"), Some(Number::Int(i64::MIN)));
        assert_eq!(n(INT64, "9.3e18"), None); // above i64::MAX
        assert_eq!(n(INT64, "9223372036854775808.0"), None);
        assert_eq!(n(UINT64, "18446744073709551615"), Some(Number::UInt(u64::MAX)));
        assert_eq!(n(UINT64, "1.8e19"), Some(Number::UInt(18000000000000000000)));
        assert_eq!(n(UINT64, "2e19"), None);
        assert_eq!(n(INT32, "inf"), None);
        assert_eq!(n(INT32, "0x10"), None);
        assert_eq!(n(INT32, ""), None);
    }

    #[test]
    fn floats_and_bools() {
        assert_eq!(n(FLOAT, "21.5"), Some(Number::Float(21.5)));
        assert_eq!(n(FLOAT, "+.5"), Some(Number::Float(0.5)));
        assert_eq!(n(FLOAT, "1."), Some(Number::Float(1.0)));
        assert_eq!(n(FLOAT, "TRUE"), Some(Number::Float(1.0)));
        assert_eq!(n(FLOAT, "on"), None);
        assert!(matches!(n(FLOAT, "-inf"), Some(Number::Float(f)) if f == f64::NEG_INFINITY));
        assert!(matches!(n(FLOAT, "nan"), Some(Number::Float(f)) if f.is_nan()));
        assert_eq!(n(FLOAT, "1,5"), None);
        assert_eq!(n(BOOL, "Yes"), Some(Number::Bool(true)));
        assert_eq!(n(BOOL, "off"), Some(Number::Bool(false)));
        assert_eq!(n(BOOL, "0.0"), Some(Number::Bool(false)));
        assert_eq!(n(BOOL, "-2"), Some(Number::Bool(true)));
        assert_eq!(n(BOOL, "+1"), None);
        assert_eq!(n(BOOL, "maybe"), None);
    }

    #[test]
    fn json_strings_and_messages() {
        // a JSON string item keeps its quotes and trailing blanks
        assert_eq!(parse_number(INT32, b"  5", true), Ok(Number::Int(5)));
        assert!(parse_number(INT32, b"5 ", true).is_err());
        assert!(parse_number(INT32, b"\"5\"", true).is_err());
        assert_eq!(
            parse_number(INT32, b"abc", false).unwrap_err(),
            "cannot convert 'abc' to the configured data type 1002"
        );
        let long = "x".repeat(70);
        assert_eq!(shown(long.as_bytes()), format!("'{}...'", "x".repeat(64)));
        assert!(parse_number(STRING, b"1", false).is_err());
        assert_eq!(normalize(0), STRING);
    }
}
