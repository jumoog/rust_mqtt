//! The state file of the driver's MSA service: its subscriptions survive a
//! restart of the driver with their ids, so CTRL clients resume them and get
//! the messages that arrived in between.
//!
//! One line per item, fields separated by a space, text fields escaped
//! (`%`, space, tab, CR and LF as `%XX`):
//!
//! ```text
//! rsmqtt-service 1
//! <id> <qos> <binary 0|1> <broker> <filter>
//! ```
//!
//! The ids are the clients' (`<start time of the driver run>-<n>`); a new
//! driver run gives new subscriptions ids of its own, so an id never names
//! two subscriptions. The caller skips lines that cannot be read; without
//! the header (another version) nothing is restored.

const HEADER: &str = "rsmqtt-service 1";

/// A subscription of the MSA service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub qos: i32,
    pub binary: bool,
    pub broker: String,
    pub filter: String,
}

/// A line of the state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Header,
    Record(Record),
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' | ' ' | '\t' | '\r' | '\n' => out.push_str(&format!("%{:02X}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The header line (without line end).
pub fn header() -> &'static str {
    HEADER
}

/// A subscription line (without line end).
pub fn record(r: &Record) -> String {
    format!("{} {} {} {} {}", escape(&r.id), r.qos, r.binary as u8, escape(&r.broker), escape(&r.filter))
}

/// Reads a line (a trailing CR is ignored); None if it is not valid.
pub fn parse(line: &str) -> Option<Line> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line == HEADER {
        return Some(Line::Header);
    }
    let f: Vec<&str> = line.split(' ').collect();
    if f.len() != 5 {
        return None;
    }
    let qos: i32 = f[1].parse().ok()?;
    let binary = match f[2] {
        "0" => false,
        "1" => true,
        _ => return None,
    };
    let r = Record { id: unescape(f[0])?, qos, binary, broker: unescape(f[3])?, filter: unescape(f[4])? };
    if !(0..=2).contains(&qos) || r.id.is_empty() || r.broker.is_empty() || r.filter.is_empty() {
        return None;
    }
    Some(Line::Record(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let r = Record {
            id: "19a4f3c-42".into(),
            qos: 2,
            binary: true,
            broker: "plant 1".into(),
            filter: "$share/g/a b/%/+/#\tx\r\n\u{e4}".into(),
        };
        let line = record(&r);
        assert!(!line.contains(['\t', '\r', '\n']));
        assert_eq!(line.split(' ').count(), 5);
        assert_eq!(parse(&line), Some(Line::Record(r.clone())));
        assert_eq!(parse(&(line + "\r")), Some(Line::Record(r)));  // CRLF files
        assert_eq!(parse(header()), Some(Line::Header));
    }

    #[test]
    fn invalid() {
        for line in [
            "",
            "rsmqtt-service 2",
            "rsmqtt-service 1 x",
            "i 1 0 b",
            "i 3 0 b f",
            "i -1 0 b f",
            "i x 0 b f",
            "i 1 2 b f",
            "i 1 0 b%2 f",
            "i 1 0 b%zz f",
            "i 1 0 b f g",
            "i 1 0 %FF f",
            "i 1 0  f",
            " 1 0 b f",
        ] {
            assert_eq!(parse(line), None, "{line:?}");
        }
    }
}
