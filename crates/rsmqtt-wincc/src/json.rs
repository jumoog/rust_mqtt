//! The JSON support of rsmqtt addresses (`json=<path>`, `ts=<path>`).
//!
//! Path syntax:
//! - (empty)          the whole payload is the value
//! - `temp`           member "temp" of the top-level object
//! - `data.temp`      nested member
//! - `values[0]`      array element
//! - `["a.b"]`        member whose name contains `.` `[` `]` (also `['a.b']`)
//! - `$` / `$.a.b`    a leading `$` (JSONPath style) is ignored
//!
//! Input: [`get`] / [`Extractor`] find values; strings are unescaped, numbers
//! / true / false are their JSON text, objects and arrays their JSON text.
//! Output: [`build`] nests one or more values along their paths.

use std::collections::BTreeMap;

const MAX_DEPTH: i32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

pub type Path = Vec<Segment>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    String = 0,
    Number = 1,
    Bool = 2,
    Null = 3,
    Object = 4,
    Array = 5,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    pub kind: Kind,
    /// unescaped for strings, the JSON text otherwise
    pub text: Vec<u8>,
}

impl Default for Value {
    fn default() -> Self {
        Value { kind: Kind::Null, text: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Yes = 0,
    Missing = 1,
    NotJson = 2,
}

//------------------------------------------------------------------------ UTF-8

fn append_utf8(out: &mut Vec<u8>, cp: u32) {
    let mut buf = [0u8; 4];
    let c = char::from_u32(cp).unwrap_or('\u{FFFD}');
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// Length of the valid UTF-8 sequence at s[i], 0 if invalid.
fn utf8_len(s: &[u8], i: usize) -> usize {
    let c = s[i];
    let (n, mut cp) = if c < 0x80 {
        return 1;
    } else if c & 0xE0 == 0xC0 {
        (2, (c & 0x1F) as u32)
    } else if c & 0xF0 == 0xE0 {
        (3, (c & 0x0F) as u32)
    } else if c & 0xF8 == 0xF0 {
        (4, (c & 0x07) as u32)
    } else {
        return 0;
    };
    if i + n > s.len() {
        return 0;
    }
    for k in 1..n {
        let cc = s[i + k];
        if cc & 0xC0 != 0x80 {
            return 0;
        }
        cp = (cp << 6) | (cc & 0x3F) as u32;
    }
    if (n == 2 && cp < 0x80) || (n == 3 && cp < 0x800) || (n == 4 && cp < 0x10000) {
        return 0;
    }
    if (0xD800..=0xDFFF).contains(&cp) || cp > 0x10FFFF {
        return 0;
    }
    n
}

//----------------------------------------------------------------------- parser

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    err: String,
}

impl<'a> Parser<'a> {
    fn new(s: &'a [u8]) -> Self {
        Parser { s, i: 0, err: String::new() }
    }

    fn fail(&mut self, what: &str) -> bool {
        if self.err.is_empty() {
            self.err = format!("{} at offset {}", what, self.i);
        }
        false
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn at(&self, c: u8) -> bool {
        self.i < self.s.len() && self.s[self.i] == c
    }

    fn hex4(&mut self, v: &mut u32) -> bool {
        if self.i + 4 > self.s.len() {
            return self.fail("short \\u escape");
        }
        *v = 0;
        for _ in 0..4 {
            let c = self.s[self.i];
            self.i += 1;
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => return self.fail("bad \\u escape"),
            };
            *v = (*v << 4) | d as u32;
        }
        true
    }

    /// At '"': reads the string, unescaped into `out` if given.
    fn string(&mut self, mut out: Option<&mut Vec<u8>>) -> bool {
        self.i += 1; // opening quote
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b'"' {
                self.i += 1;
                return true;
            }
            if c < 0x20 {
                return self.fail("control character in string");
            }
            if c != b'\\' {
                let n = utf8_len(self.s, self.i);
                if n == 0 {
                    return self.fail("invalid UTF-8 in string");
                }
                if let Some(o) = out.as_deref_mut() {
                    o.extend_from_slice(&self.s[self.i..self.i + n]);
                }
                self.i += n;
                continue;
            }
            self.i += 1;
            if self.i >= self.s.len() {
                break;
            }
            let e = self.s[self.i];
            self.i += 1;
            let simple = match e {
                b'"' => b'"',
                b'\\' => b'\\',
                b'/' => b'/',
                b'b' => 0x08,
                b'f' => 0x0C,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'u' => {
                    let mut cp = 0;
                    if !self.hex4(&mut cp) {
                        return false;
                    }
                    if (0xD800..=0xDBFF).contains(&cp) {
                        if self.i + 2 > self.s.len() || self.s[self.i] != b'\\' || self.s[self.i + 1] != b'u' {
                            return self.fail("lone surrogate");
                        }
                        self.i += 2;
                        let mut lo = 0;
                        if !self.hex4(&mut lo) {
                            return false;
                        }
                        if !(0xDC00..=0xDFFF).contains(&lo) {
                            return self.fail("lone surrogate");
                        }
                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                    } else if (0xDC00..=0xDFFF).contains(&cp) {
                        return self.fail("lone surrogate");
                    }
                    if let Some(o) = out.as_deref_mut() {
                        append_utf8(o, cp);
                    }
                    continue;
                }
                _ => return self.fail("bad escape"),
            };
            if let Some(o) = out.as_deref_mut() {
                o.push(simple);
            }
        }
        self.fail("unterminated string")
    }

    fn digit(&self) -> bool {
        self.i < self.s.len() && self.s[self.i].is_ascii_digit()
    }

    fn number(&mut self) -> bool {
        let start = self.i;
        if self.at(b'-') {
            self.i += 1;
        }
        if !self.digit() {
            return self.fail("bad number");
        }
        if self.s[self.i] == b'0' {
            self.i += 1;
        } else {
            while self.digit() {
                self.i += 1;
            }
        }
        if self.at(b'.') {
            self.i += 1;
            if !self.digit() {
                return self.fail("bad number");
            }
            while self.digit() {
                self.i += 1;
            }
        }
        if self.at(b'e') || self.at(b'E') {
            self.i += 1;
            if self.at(b'+') || self.at(b'-') {
                self.i += 1;
            }
            if !self.digit() {
                return self.fail("bad number");
            }
            while self.digit() {
                self.i += 1;
            }
        }
        self.i > start
    }

    fn literal(&mut self, w: &[u8]) -> bool {
        if !self.s[self.i..].starts_with(w) {
            return self.fail("unexpected character");
        }
        self.i += w.len();
        true
    }

    /// Skips (and checks) the value at i.
    fn skip(&mut self, depth: i32) -> bool {
        if depth > MAX_DEPTH {
            return self.fail("nested too deeply");
        }
        self.ws();
        if self.i >= self.s.len() {
            return self.fail("unexpected end");
        }
        let c = self.s[self.i];
        if c == b'"' {
            return self.string(None);
        }
        if c == b'{' || c == b'[' {
            let close = if c == b'{' { b'}' } else { b']' };
            self.i += 1;
            self.ws();
            if self.at(close) {
                self.i += 1;
                return true;
            }
            loop {
                if c == b'{' {
                    self.ws();
                    if !self.at(b'"') {
                        return self.fail("expected member name");
                    }
                    if !self.string(None) {
                        return false;
                    }
                    self.ws();
                    if !self.at(b':') {
                        return self.fail("expected ':'");
                    }
                    self.i += 1;
                }
                if !self.skip(depth + 1) {
                    return false;
                }
                self.ws();
                if self.at(b',') {
                    self.i += 1;
                    continue;
                }
                if self.at(close) {
                    self.i += 1;
                    return true;
                }
                return self.fail(if c == b'{' { "expected ',' or '}'" } else { "expected ',' or ']'" });
            }
        }
        match c {
            b't' => self.literal(b"true"),
            b'f' => self.literal(b"false"),
            b'n' => self.literal(b"null"),
            _ => self.number(),
        }
    }

    fn kind_of(c: u8) -> Kind {
        match c {
            b'{' => Kind::Object,
            b'[' => Kind::Array,
            b't' | b'f' => Kind::Bool,
            b'n' => Kind::Null,
            _ => Kind::Number,
        }
    }

    /// Reads the value at i.
    fn value(&mut self, out: &mut Value) -> bool {
        self.ws();
        if self.i >= self.s.len() {
            return self.fail("unexpected end");
        }
        let c = self.s[self.i];
        let start = self.i;
        if c == b'"' {
            out.kind = Kind::String;
            out.text.clear();
            let mut text = std::mem::take(&mut out.text);
            let ok = self.string(Some(&mut text));
            out.text = text;
            return ok;
        }
        if !self.skip(0) {
            return false;
        }
        out.text.clear();
        out.text.extend_from_slice(&self.s[start..self.i]);
        out.kind = Self::kind_of(c);
        true
    }

    /// Moves i into the value of member `key` of the object at i.
    fn member(&mut self, key: &[u8], found: &mut bool) -> bool {
        *found = false;
        self.i += 1; // '{'
        self.ws();
        if self.at(b'}') {
            self.i += 1;
            return true;
        }
        let mut name = Vec::new();
        loop {
            self.ws();
            if !self.at(b'"') {
                return self.fail("expected member name");
            }
            name.clear();
            if !self.string(Some(&mut name)) {
                return false;
            }
            self.ws();
            if !self.at(b':') {
                return self.fail("expected ':'");
            }
            self.i += 1;
            self.ws();
            if name == key {
                *found = true;
                return true;
            }
            if !self.skip(1) {
                return false;
            }
            self.ws();
            if self.at(b',') {
                self.i += 1;
                continue;
            }
            if self.at(b'}') {
                self.i += 1;
                return true;
            }
            return self.fail("expected ',' or '}'");
        }
    }

    /// Moves i into element `index` of the array at i.
    fn element(&mut self, index: usize, found: &mut bool) -> bool {
        *found = false;
        self.i += 1; // '['
        self.ws();
        if self.at(b']') {
            self.i += 1;
            return true;
        }
        let mut k = 0usize;
        loop {
            self.ws();
            if k == index {
                *found = true;
                return true;
            }
            if !self.skip(1) {
                return false;
            }
            self.ws();
            if self.at(b',') {
                self.i += 1;
                k += 1;
                continue;
            }
            if self.at(b']') {
                self.i += 1;
                return true;
            }
            return self.fail("expected ',' or ']'");
        }
    }
}

//------------------------------------------------------------------------ paths

/// Parses a path.
pub fn parse_path(text: &str) -> Result<Path, String> {
    let t = text.as_bytes();
    let mut out = Path::new();
    let mut i = 0usize;
    // JSONPath style root: "$", "$.a", "$[0]" (but "$value" is a member name)
    if text == "$" {
        return Ok(out);
    }
    if t.len() > 1 && t[0] == b'$' && (t[1] == b'.' || t[1] == b'[') {
        i = if t[1] == b'.' { 2 } else { 1 };
    }
    let mut expect_name = true;
    while i < t.len() {
        let c = t[i];
        if c == b'[' {
            i += 1;
            if i < t.len() && (t[i] == b'"' || t[i] == b'\'') {
                let q = t[i];
                i += 1;
                let end = t[i..].iter().position(|&b| b == q).map(|p| p + i);
                match end {
                    Some(e) if e + 1 < t.len() && t[e + 1] == b']' => {
                        out.push(Segment::Key(text[i..e].to_string()));
                        i = e + 2;
                    }
                    _ => return Err("unterminated [\"...\"] in JSON path".into()),
                }
            } else {
                let end = t[i..].iter().position(|&b| b == b']').map(|p| p + i);
                let e = match end {
                    Some(e) if e != i => e,
                    _ => return Err("expected [<index>] in JSON path".into()),
                };
                match text[i..e].parse::<usize>() {
                    Ok(idx) if idx <= 100_000 && text[i..e].bytes().all(|b| b.is_ascii_digit()) => {
                        out.push(Segment::Index(idx))
                    }
                    _ => return Err("array index in JSON path must be a number up to 100000".into()),
                }
                i = e + 1;
            }
            expect_name = false;
            continue;
        }
        if c == b'.' {
            if expect_name {
                return Err("empty member name in JSON path".into());
            }
            i += 1;
            expect_name = true;
            if i == t.len() {
                return Err("JSON path ends with '.'".into());
            }
            continue;
        }
        if !expect_name {
            return Err("expected '.' or '[' in JSON path".into());
        }
        let end = t[i..].iter().position(|&b| b == b'.' || b == b'[' || b == b']').map(|p| p + i).unwrap_or(t.len());
        if end < t.len() && t[end] == b']' {
            return Err("unexpected ']' in JSON path".into());
        }
        out.push(Segment::Key(text[i..end].to_string()));
        i = end;
        expect_name = false;
    }
    Ok(out)
}

/// The path in a canonical form, for messages ("data.temp", "values[0]", "$").
pub fn path_to_string(path: &Path) -> String {
    if path.is_empty() {
        return "$".into();
    }
    let mut s = String::new();
    for seg in path {
        match seg {
            Segment::Index(i) => s += &format!("[{}]", i),
            Segment::Key(k) if k.is_empty() || k.contains(['.', '[', ']']) => s += &format!("[\"{}\"]", k),
            Segment::Key(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s += k;
            }
        }
    }
    s
}

//--------------------------------------------------------------------- lookups

/// Looks up `path` in `doc`. Missing: the document is JSON but has nothing at
/// that path. NotJson: the error says what is wrong.
pub fn get(doc: &[u8], path: &Path) -> (Found, Value, String) {
    let mut p = Parser::new(doc);
    for seg in path {
        p.ws();
        if p.i >= doc.len() {
            p.fail("unexpected end");
            return (Found::NotJson, Value::default(), p.err);
        }
        let c = doc[p.i];
        let mut found = false;
        let mismatch = match seg {
            Segment::Index(_) => c != b'[',
            Segment::Key(_) => c != b'{',
        };
        if mismatch {
            // a value of another type: check it is JSON at all
            let mut ignored = Value::default();
            if !p.value(&mut ignored) {
                return (Found::NotJson, Value::default(), p.err);
            }
            return (Found::Missing, Value::default(), String::new());
        }
        let ok = match seg {
            Segment::Index(i) => p.element(*i, &mut found),
            Segment::Key(k) => p.member(k.as_bytes(), &mut found),
        };
        if !ok {
            return (Found::NotJson, Value::default(), p.err);
        }
        if !found {
            return (Found::Missing, Value::default(), String::new());
        }
    }
    let mut out = Value::default();
    if !p.value(&mut out) {
        return (Found::NotJson, Value::default(), p.err);
    }
    if path.is_empty() {
        p.ws();
        if p.i != doc.len() {
            return (Found::NotJson, Value::default(), format!("unexpected data after the JSON value at offset {}", p.i));
        }
    }
    (Found::Yes, out, String::new())
}

/// The elements of a JSON array.
pub fn parse_array(array: &[u8]) -> Result<Vec<Value>, String> {
    let mut p = Parser::new(array);
    let mut out = Vec::new();
    p.ws();
    if !p.at(b'[') {
        return Err("not a JSON array".into());
    }
    p.i += 1;
    p.ws();
    if p.at(b']') {
        p.i += 1;
    } else {
        loop {
            let mut v = Value::default();
            if !p.value(&mut v) {
                return Err(p.err);
            }
            out.push(v);
            p.ws();
            if p.at(b',') {
                p.i += 1;
                continue;
            }
            if p.at(b']') {
                p.i += 1;
                break;
            }
            p.fail("expected ',' or ']'");
            return Err(p.err);
        }
    }
    p.ws();
    if p.i != array.len() {
        return Err(format!("unexpected data after the JSON array at offset {}", p.i));
    }
    Ok(out)
}

/// A JSON string literal; invalid UTF-8 becomes U+FFFD.
pub fn quote(text: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(text.len() + 2);
    out.push(b'"');
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c >= 0x80 {
            let n = utf8_len(text, i);
            if n == 0 {
                out.extend_from_slice("\u{FFFD}".as_bytes());
                i += 1;
            } else {
                out.extend_from_slice(&text[i..i + n]);
                i += n;
            }
            continue;
        }
        match c {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0C => out.extend_from_slice(b"\\f"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            c if c < 0x20 => {
                out.extend_from_slice(b"\\u00");
                out.push(HEX[(c >> 4) as usize]);
                out.push(HEX[(c & 15) as usize]);
            }
            c => out.push(c),
        }
        i += 1;
    }
    out.push(b'"');
    out
}

//------------------------------------------------------------------------ build

#[derive(Default)]
enum Node {
    #[default]
    Unset,
    Leaf(Vec<u8>),
    Obj(Vec<(String, Node)>),
    Arr(Vec<Node>),
}

impl Node {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Node::Leaf(raw) => out.extend_from_slice(raw),
            Node::Obj(members) => {
                out.push(b'{');
                for (k, (name, n)) in members.iter().enumerate() {
                    if k > 0 {
                        out.push(b',');
                    }
                    out.extend_from_slice(&quote(name.as_bytes()));
                    out.push(b':');
                    n.write(out);
                }
                out.push(b'}');
            }
            Node::Arr(elements) => {
                out.push(b'[');
                for (k, n) in elements.iter().enumerate() {
                    if k > 0 {
                        out.push(b',');
                    }
                    n.write(out);
                }
                out.push(b']');
            }
            Node::Unset => out.extend_from_slice(b"null"),
        }
    }
}

/// Nests JSON values (already JSON text) along their paths into one document.
/// Fails if the paths collide (one is the parent of another, or a level is
/// used as object and as array).
pub fn build(members: &[(&Path, &[u8])]) -> Result<Vec<u8>, String> {
    let mut root = Node::Unset;
    for (path, raw) in members {
        let collide = || format!("JSON path {} collides with another one", path_to_string(path));
        let mut n = &mut root;
        for seg in path.iter() {
            match seg {
                Segment::Index(i) => {
                    if matches!(n, Node::Unset) {
                        *n = Node::Arr(Vec::new());
                    }
                    let Node::Arr(elements) = n else { return Err(collide()) };
                    if elements.len() <= *i {
                        elements.resize_with(*i + 1, || Node::Unset);
                    }
                    n = &mut elements[*i];
                }
                Segment::Key(key) => {
                    if matches!(n, Node::Unset) {
                        *n = Node::Obj(Vec::new());
                    }
                    let Node::Obj(ms) = n else { return Err(collide()) };
                    let pos = match ms.iter().position(|(k, _)| k == key) {
                        Some(p) => p,
                        None => {
                            ms.push((key.clone(), Node::Unset));
                            ms.len() - 1
                        }
                    };
                    n = &mut ms[pos].1;
                }
            }
        }
        if !matches!(n, Node::Unset) {
            return Err(collide());
        }
        *n = Node::Leaf(raw.to_vec());
    }
    let mut out = Vec::new();
    root.write(&mut out);
    Ok(out)
}

//------------------------------------------------------------------------- time

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn digits(s: &[u8], i: &mut usize, n: usize) -> Option<i64> {
    if *i + n > s.len() {
        return None;
    }
    let mut v = 0i64;
    for k in 0..n {
        let c = s[*i + k];
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as i64;
    }
    *i += n;
    Some(v)
}

fn parse_iso(s: &[u8]) -> Option<i64> {
    let mut i = 0usize;
    let y = digits(s, &mut i, 4)?;
    let expect = |i: &mut usize, c: u8| -> Option<()> {
        if *i < s.len() && s[*i] == c {
            *i += 1;
            Some(())
        } else {
            None
        }
    };
    expect(&mut i, b'-')?;
    let mo = digits(s, &mut i, 2)?;
    expect(&mut i, b'-')?;
    let d = digits(s, &mut i, 2)?;
    if i >= s.len() || !matches!(s[i], b'T' | b't' | b' ') {
        return None;
    }
    i += 1;
    let h = digits(s, &mut i, 2)?;
    expect(&mut i, b':')?;
    let mi = digits(s, &mut i, 2)?;
    let mut sec = 0;
    if i < s.len() && s[i] == b':' {
        i += 1;
        sec = digits(s, &mut i, 2)?;
    }
    let mut frac = 0i64;
    if i < s.len() && (s[i] == b'.' || s[i] == b',') {
        i += 1;
        let mut n = 0;
        while i < s.len() && s[i].is_ascii_digit() {
            if n < 3 {
                frac = frac * 10 + (s[i] - b'0') as i64;
            }
            n += 1;
            i += 1;
        }
        if n == 0 {
            return None;
        }
        while n < 3 {
            frac *= 10;
            n += 1;
        }
    }
    let mut offset = 0i64; // minutes east of UTC
    if i < s.len() {
        let c = s[i];
        i += 1;
        if c == b'Z' || c == b'z' {
        } else if c == b'+' || c == b'-' {
            let oh = digits(s, &mut i, 2)?;
            if i < s.len() && s[i] == b':' {
                i += 1;
            }
            let om = if i < s.len() { digits(s, &mut i, 2)? } else { 0 };
            offset = (oh * 60 + om) * if c == b'-' { -1 } else { 1 };
        } else {
            return None;
        }
    }
    if i != s.len() || !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(y, mo as u32, d as u32);
    Some((((days * 24 + h) * 60 + mi - offset) * 60 + sec) * 1000 + frac)
}

/// Timestamp of a value: a number or digit string is seconds since 1970
/// (milliseconds when it is 1e11 or more), a string otherwise ISO 8601
/// ("2026-11-23T10:00:00.123Z", "+01:00" offsets; no offset = UTC).
pub fn parse_time(v: &Value) -> Option<i64> {
    let t = &v.text;
    let numeric = match v.kind {
        Kind::Number => true,
        Kind::String => {
            !t.is_empty() && t.iter().all(|&c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'e' | b'E'))
        }
        _ => false,
    };
    if v.kind == Kind::String && !numeric {
        return parse_iso(t);
    }
    if !numeric {
        return None;
    }
    let s = std::str::from_utf8(t).ok()?;
    let s = s.strip_prefix('+').unwrap_or(s);
    let d: f64 = s.parse().ok()?;
    if !d.is_finite() {
        return None;
    }
    Some(if d.abs() >= 1e11 { d.round() as i64 } else { (d * 1000.0).round() as i64 })
}

/// ISO 8601 in UTC with milliseconds, e.g. "2026-11-23T10:00:00.123Z".
pub fn iso_time(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        rest / 3_600_000,
        (rest / 60_000) % 60,
        (rest / 1000) % 60,
        rest % 1000
    )
}

//------------------------------------------------------------------- extractor

#[derive(Default, Clone)]
struct ExNode {
    id: Option<usize>,
    members: BTreeMap<String, usize>,
    elements: BTreeMap<usize, usize>,
}

/// The result of one path in [`Extractor::run`].
#[derive(Debug, Clone)]
pub struct ExResult {
    pub found: Found,
    pub value: Value,
}

/// Looks up several paths in one pass over a document, with the results
/// [`get`] would give for each path. Paths are added once (e.g. for all
/// addresses on a topic) and the extractor runs per message.
#[derive(Clone)]
pub struct Extractor {
    nodes: Vec<ExNode>,
    count: usize,
}

impl Default for Extractor {
    fn default() -> Self {
        Extractor { nodes: vec![ExNode::default()], count: 0 }
    }
}

struct Walker<'a, 'b> {
    nodes: &'a [ExNode],
    results: &'b mut [ExResult],
    p: Parser<'a>,
    visited: Vec<bool>,
    done: Vec<bool>,
    remaining: usize,
    open_targets: i32,
    stop: bool,
}

impl<'a, 'b> Walker<'a, 'b> {
    fn settle(&mut self, id: Option<usize>) {
        let Some(id) = id else { return };
        if self.done[id] {
            return;
        }
        self.done[id] = true;
        self.remaining -= 1;
        if self.remaining == 0 && self.open_targets == 0 {
            self.stop = true; // nothing left to look for
        }
    }

    /// Requested paths below node `ni` can no longer be found: Missing.
    fn missing_below(&mut self, ni: usize) {
        let children: Vec<usize> = {
            let n = &self.nodes[ni];
            n.members.values().chain(n.elements.values()).copied().collect()
        };
        for c in children {
            self.missing_below(c);
            self.settle(self.nodes[c].id);
        }
    }

    /// The end of an object or array: what was not in it is missing.
    fn not_visited(&mut self, ni: usize) {
        let children: Vec<usize> = {
            let n = &self.nodes[ni];
            n.members.values().chain(n.elements.values()).copied().collect()
        };
        for c in children {
            if !self.visited[c] {
                self.missing_below(c);
                self.settle(self.nodes[c].id);
            }
        }
    }

    fn found(&mut self, id: usize) {
        self.results[id].found = Found::Yes;
        self.settle(Some(id));
    }

    fn child(&mut self, ni: Option<usize>, depth: i32) -> bool {
        match ni {
            Some(ni) if !self.visited[ni] => {
                self.visited[ni] = true;
                self.walk(ni, depth)
            }
            _ => self.p.skip(depth),
        }
    }

    fn walk(&mut self, ni: usize, depth: i32) -> bool {
        if depth > MAX_DEPTH {
            return self.p.fail("nested too deeply");
        }
        self.p.ws();
        if self.p.i >= self.p.s.len() {
            return self.p.fail("unexpected end");
        }
        let c = self.p.s[self.p.i];
        let start = self.p.i;
        let id = self.nodes[ni].id;

        if c == b'"' {
            match id {
                None => {
                    if !self.p.string(None) {
                        return false;
                    }
                    self.missing_below(ni);
                }
                Some(id) => {
                    let mut text = std::mem::take(&mut self.results[id].value.text);
                    text.clear();
                    let ok = self.p.string(Some(&mut text));
                    self.results[id].value.text = text;
                    self.results[id].value.kind = Kind::String;
                    if !ok {
                        return false;
                    }
                    self.missing_below(ni);
                    self.found(id);
                }
            }
            return true;
        }

        if id.is_some() {
            self.open_targets += 1;
        }
        let has_members = !self.nodes[ni].members.is_empty();
        let has_elements = !self.nodes[ni].elements.is_empty();
        let ok = if c == b'{' && has_members {
            self.members(ni, depth)
        } else if c == b'[' && has_elements {
            self.elements(ni, depth)
        } else {
            let ok = self.p.skip(depth);
            if ok {
                self.missing_below(ni); // another type than the paths below need
            }
            ok
        };
        if id.is_some() {
            self.open_targets -= 1;
        }
        if !ok || self.stop {
            return ok;
        }
        if let Some(id) = id {
            let r = &mut self.results[id].value;
            r.text.clear();
            r.text.extend_from_slice(&self.p.s[start..self.p.i]);
            r.kind = Parser::kind_of(c);
            self.found(id);
        }
        true
    }

    fn members(&mut self, ni: usize, depth: i32) -> bool {
        self.p.i += 1; // '{'
        self.p.ws();
        if self.p.at(b'}') {
            self.p.i += 1;
            self.not_visited(ni);
            return true;
        }
        let mut name = Vec::new();
        loop {
            self.p.ws();
            if !self.p.at(b'"') {
                return self.p.fail("expected member name");
            }
            name.clear();
            if !self.p.string(Some(&mut name)) {
                return false;
            }
            self.p.ws();
            if !self.p.at(b':') {
                return self.p.fail("expected ':'");
            }
            self.p.i += 1;
            let target = std::str::from_utf8(&name).ok().and_then(|n| self.nodes[ni].members.get(n).copied());
            if !self.child(target, depth + 1) {
                return false;
            }
            if self.stop {
                return true;
            }
            self.p.ws();
            if self.p.at(b',') {
                self.p.i += 1;
                continue;
            }
            if self.p.at(b'}') {
                self.p.i += 1;
                self.not_visited(ni);
                return true;
            }
            return self.p.fail("expected ',' or '}'");
        }
    }

    fn elements(&mut self, ni: usize, depth: i32) -> bool {
        self.p.i += 1; // '['
        self.p.ws();
        if self.p.at(b']') {
            self.p.i += 1;
            self.not_visited(ni);
            return true;
        }
        let mut k = 0usize;
        loop {
            let target = self.nodes[ni].elements.get(&k).copied();
            if !self.child(target, depth + 1) {
                return false;
            }
            if self.stop {
                return true;
            }
            self.p.ws();
            if self.p.at(b',') {
                self.p.i += 1;
                k += 1;
                continue;
            }
            if self.p.at(b']') {
                self.p.i += 1;
                self.not_visited(ni);
                return true;
            }
            return self.p.fail("expected ',' or ']'");
        }
    }
}

impl Extractor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a path and returns its id (an index into the results); the same
    /// path twice gives the same id.
    pub fn add(&mut self, path: &Path) -> usize {
        let mut n = 0usize;
        for seg in path {
            let existing = match seg {
                Segment::Index(i) => self.nodes[n].elements.get(i).copied(),
                Segment::Key(k) => self.nodes[n].members.get(k).copied(),
            };
            n = match existing {
                Some(next) => next,
                None => {
                    let next = self.nodes.len();
                    self.nodes.push(ExNode::default());
                    match seg {
                        Segment::Index(i) => self.nodes[n].elements.insert(*i, next),
                        Segment::Key(k) => self.nodes[n].members.insert(k.clone(), next),
                    };
                    next
                }
            };
        }
        if self.nodes[n].id.is_none() {
            self.nodes[n].id = Some(self.count);
            self.count += 1;
        }
        self.nodes[n].id.unwrap()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn len(&self) -> usize {
        self.count
    }

    /// results[id] for every added path; the error says what is wrong when
    /// a result is NotJson. `results` is reused to avoid allocations.
    pub fn run(&self, doc: &[u8], results: &mut Vec<ExResult>) -> String {
        results.resize_with(self.count, || ExResult { found: Found::Missing, value: Value::default() });
        for r in results.iter_mut() {
            r.found = Found::Missing;
            r.value.kind = Kind::Null;
            r.value.text.clear();
        }
        if self.count == 0 {
            return String::new();
        }
        let mut w = Walker {
            nodes: &self.nodes,
            results: &mut results[..],
            p: Parser::new(doc),
            visited: vec![false; self.nodes.len()],
            done: vec![false; self.count],
            remaining: self.count,
            open_targets: 0,
            stop: false,
        };
        w.visited[0] = true;
        if !w.walk(0, 0) {
            // like get(): what was settled before the error stands, the rest is NotJson
            let err = std::mem::take(&mut w.p.err);
            let done = std::mem::take(&mut w.done);
            for (k, r) in results.iter_mut().enumerate() {
                if !done[k] {
                    r.found = Found::NotJson;
                }
            }
            return err;
        }
        let end = w.p.i;
        drop(w);
        // the whole payload as the value (empty path): nothing may follow it
        if let Some(root) = self.nodes[0].id {
            if results[root].found == Found::Yes {
                let mut p = Parser::new(doc);
                p.i = end;
                p.ws();
                if p.i != doc.len() {
                    results[root].found = Found::NotJson;
                    return format!("unexpected data after the JSON value at offset {}", p.i);
                }
            }
        }
        String::new()
    }
}

//------------------------------------------------------------------------- dyn

/// The items of a payload for a dyn DPE: a JSON array gives one item per
/// element, a single value (JSON or not) one item, an empty payload or `[]`
/// none. Items are (text, is_string); null items are an error.
pub fn dyn_items(payload: &[u8]) -> Result<Vec<(Vec<u8>, bool)>, String> {
    let trimmed = {
        let s = payload;
        let a = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
        let b = s.iter().rposition(|c| !c.is_ascii_whitespace()).map(|p| p + 1).unwrap_or(a);
        &s[a..b.max(a)]
    };
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let (found, v, _) = get(trimmed, &Path::new());
    if found != Found::Yes {
        return Ok(vec![(payload.to_vec(), false)]); // not JSON: the payload is a single value
    }
    let items = match v.kind {
        Kind::Array => parse_array(&v.text)?,
        Kind::Null => Vec::new(),
        _ => vec![v],
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, it) in items.into_iter().enumerate() {
        if it.kind == Kind::Null {
            return Err(format!("null at index {}", i));
        }
        out.push((it.text, it.kind == Kind::String));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str) -> Path {
        parse_path(t).unwrap()
    }

    #[test]
    fn paths() {
        let x = p("values[2].x");
        assert_eq!(x, vec![Segment::Key("values".into()), Segment::Index(2), Segment::Key("x".into())]);
        assert_eq!(p("$.a[\"b.c\"]['d']"), vec![Segment::Key("a".into()), Segment::Key("b.c".into()), Segment::Key("d".into())]);
        assert_eq!(p("$value"), vec![Segment::Key("$value".into())]);
        assert!(p("$").is_empty());
        assert_eq!(p("[0]"), vec![Segment::Index(0)]);
        for bad in ["a[x]", "a.", "a]", "a..b"] {
            assert!(parse_path(bad).is_err(), "{bad}");
        }
        assert_eq!(path_to_string(&p("data.temp")), "data.temp");
        assert_eq!(path_to_string(&Path::new()), "$");
    }

    const DOC: &str = r#" {"data": {"temp": 21.5, "on": true, "name": "Pump \"1\" é😀",
                         "n": null, "list": [1, {"x": [10, 20]}], "obj": {"a": 1} }, "t": 1700000000} "#;

    fn g(doc: &str, path: &str) -> (Found, Value) {
        let (f, v, _) = get(doc.as_bytes(), &p(path));
        (f, v)
    }

    #[test]
    fn lookups() {
        assert_eq!(g(DOC, "data.temp"), (Found::Yes, Value { kind: Kind::Number, text: b"21.5".to_vec() }));
        assert_eq!(g(DOC, "data.on").1.kind, Kind::Bool);
        assert_eq!(g(DOC, "data.name").1.text, "Pump \"1\" \u{e9}\u{1F600}".as_bytes());
        assert_eq!(g(DOC, "data.n").1.kind, Kind::Null);
        assert_eq!(g(DOC, "data.list[1].x[1]").1.text, b"20");
        assert_eq!(g(DOC, "data.obj").1.text, br#"{"a": 1}"#);
        for missing in ["data.missing", "data.list[5]", "data.temp.x"] {
            assert_eq!(g(DOC, missing).0, Found::Missing, "{missing}");
        }
        assert_eq!(g("hello", "a").0, Found::NotJson);
        assert_eq!(g("{\"a\":", "a").0, Found::NotJson);
        assert_eq!(g(" 42 ", "").1.text, b"42");
        assert_eq!(g("\"x\"", "").1, Value { kind: Kind::String, text: b"x".to_vec() });
        assert_eq!(g("42 x", "").0, Found::NotJson);
        assert_eq!(g("01", "").0, Found::NotJson);
    }

    #[test]
    fn quoting_building_time() {
        assert_eq!(quote(b"a\"b\\c\n\x01"), br#""a\"b\\c\n\u0001""#.to_vec());
        assert_eq!(quote("\u{e9}".as_bytes()), "\"\u{e9}\"".as_bytes());
        assert_eq!(quote(b"\xFF"), "\"\u{FFFD}\"".as_bytes());

        let (v, t) = (p("data.value"), p("data.ts"));
        assert_eq!(build(&[(&v, b"30"), (&t, b"\"T\"")]).unwrap(), br#"{"data":{"value":30,"ts":"T"}}"#);
        assert_eq!(build(&[(&p("values[2]"), b"true")]).unwrap(), br#"{"values":[null,null,true]}"#);
        assert_eq!(build(&[(&Path::new(), b"\"x\"")]).unwrap(), br#""x""#);
        assert!(build(&[(&p("a"), b"1"), (&p("a.b"), b"2")]).is_err());

        let t = |kind, s: &str| parse_time(&Value { kind, text: s.as_bytes().to_vec() });
        assert_eq!(t(Kind::Number, "1700000000"), Some(1700000000000));
        assert_eq!(t(Kind::Number, "1700000000123"), Some(1700000000123));
        assert_eq!(t(Kind::Number, "1700000000.5"), Some(1700000000500));
        assert_eq!(t(Kind::String, "2023-11-14T22:13:20.123Z"), Some(1700000000123));
        assert_eq!(t(Kind::String, "2023-11-14T23:13:20+01:00"), Some(1700000000000));
        assert_eq!(t(Kind::String, "2023-11-14 22:13:20"), Some(1700000000000));
        assert_eq!(t(Kind::String, "yesterday"), None);
        assert_eq!(t(Kind::Bool, "true"), None);
        assert_eq!(iso_time(1700000000123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso_time(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn arrays_and_dyn() {
        let items = parse_array(br#" [1, "a\"b", true, null, [2], {"x": 1}] "#).unwrap();
        assert_eq!(items.len(), 6);
        assert_eq!(items[1], Value { kind: Kind::String, text: b"a\"b".to_vec() });
        assert_eq!(items[4].text, b"[2]");
        assert!(parse_array(b"[1,]").is_err() && parse_array(b"[1] x").is_err() && parse_array(b"{}").is_err());

        let d = |s: &str| dyn_items(s.as_bytes());
        assert_eq!(d("[1.5, 2]").unwrap(), vec![(b"1.5".to_vec(), false), (b"2".to_vec(), false)]);
        assert_eq!(d("42.5").unwrap(), vec![(b"42.5".to_vec(), false)]);
        assert_eq!(d("hello").unwrap(), vec![(b"hello".to_vec(), false)]);
        assert_eq!(d("\"[x]\"").unwrap(), vec![(b"[x]".to_vec(), true)]);
        assert!(d("").unwrap().is_empty() && d("[]").unwrap().is_empty() && d("null").unwrap().is_empty());
        assert!(d("[1, null]").is_err());
    }

    // the extractor gives what get() gives, for every path and every cut of every document
    #[test]
    fn extractor_matches_get() {
        let docs = [
            r#"{"a": {"b": 1, "c": [10, {"d": "x\"y"}, 30]}, "s": "t", "arr": [{"y": 1}, {"y": 2}, {"y": 3}], "n": null}"#,
            r#"{"dup": {"x": 1}, "dup": {"x": 2, "y": 3}, "a": [1, 2]}"#,
            r#"{"a": {"b": 1}, "bad": [1, , 2], "z": 5}"#,
            r#"{"a": 1, "b": "unterminated"#,
            r#"[0, [1, 2], {"a": true}]"#,
            "  42  ", "42 x", r#""just text""#, "hello", "", "{}", "[]",
            r#"{"a": {"b": {"c": {"d": [[[{"e": false}]]]}}}, "after": 1}"#,
            r#"{"a": 5, "x": [1, 2, 3], "s": "é😀"} trailing"#,
        ];
        let paths = ["", "a", "a.b", "a.c", "a.c[1].d", "a.c[5]", "a.b.x", "s", "arr[2].y", "arr[0]", "n", "dup", "dup.x",
                     "dup.y", "bad", "z", "[0]", "[1][1]", "[2].a", "[3]", "x[2]", "a.b.c.d[0][0][0].e", "after", "missing", "a[1]"];
        let ps: Vec<Path> = paths.iter().map(|t| p(t)).collect();
        let mut all = Extractor::new();
        let ids: Vec<usize> = ps.iter().map(|x| all.add(x)).collect();
        let mut compared = 0;
        let mut res = Vec::new();
        for doc in docs {
            for cut in 0..=doc.len() {
                if !doc.is_char_boundary(cut) {
                    continue;
                }
                let d = &doc.as_bytes()[..cut];
                all.run(d, &mut res);
                for (k, path) in ps.iter().enumerate() {
                    let (gf, gv, _) = get(d, path);
                    let r = &res[ids[k]];
                    assert_eq!(r.found, gf, "doc {:?} cut {} path {:?}", doc, cut, paths[k]);
                    if gf == Found::Yes {
                        assert_eq!(r.value, gv, "doc {:?} cut {} path {:?}", doc, cut, paths[k]);
                    }
                    // each path alone, too (full documents only)
                    if cut == doc.len() {
                        let mut one = Extractor::new();
                        let id = one.add(path);
                        let mut r1 = Vec::new();
                        one.run(d, &mut r1);
                        assert_eq!(r1[id].found, gf);
                    }
                    compared += 1;
                }
            }
        }
        assert!(compared > 9_000, "{compared}");
        let mut x = Extractor::new();
        assert_eq!(x.add(&p("a.b")), 0);
        assert_eq!(x.add(&p("a.b")), 0);
        assert_eq!(x.len(), 1);
    }
}
