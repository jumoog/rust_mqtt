//! Sparkplug B (v3.0) Host Application.
//!
//! The driver subscribes `spBv1.0/<group>/+/<edge node>/#` per edge node and
//! hands every message to [`Host::on_message`], which
//! - decodes the protobuf payload (types of `sparkplug_b.proto` from the
//!   srad-types crate),
//! - tracks births and deaths, bdSeq, seq and aliases per edge node / device,
//! - returns the updates for the addresses (values, online state, stale) and
//! - asks for a rebirth (NCMD "Node Control/Rebirth") when it has missed
//!   something: data without birth, a gap in seq, an unknown alias or metric.
//!
//! [`Host::command`] builds NCMD / DCMD for writes, with the data type of the
//! metric from its birth.
//!
//! Values become text for the driver's transformation: numbers and
//! true/false as text, strings as they are, arrays as JSON arrays (dyn
//! DPEs), DateTime as milliseconds since 1970, Bytes / File as raw bytes,
//! DataSet / Template as JSON (PropertySets are metric metadata, not values).

use std::collections::BTreeMap;

use srad_types::payload::{data_set, metric, DataSet, Message, Metric, Payload, Template};

use crate::json;

pub const NAMESPACE: &str = "spBv1.0";
const REBIRTH_METRIC: &str = "Node Control/Rebirth";

/// Sparkplug B data types (sparkplug_b.proto, DataType)
pub mod types {
    pub const UNKNOWN: u32 = 0;
    pub const INT8: u32 = 1;
    pub const INT16: u32 = 2;
    pub const INT32: u32 = 3;
    pub const INT64: u32 = 4;
    pub const UINT8: u32 = 5;
    pub const UINT16: u32 = 6;
    pub const UINT32: u32 = 7;
    pub const UINT64: u32 = 8;
    pub const FLOAT: u32 = 9;
    pub const DOUBLE: u32 = 10;
    pub const BOOLEAN: u32 = 11;
    pub const STRING: u32 = 12;
    pub const DATETIME: u32 = 13;
    pub const TEXT: u32 = 14;
    pub const UUID: u32 = 15;
    pub const DATASET: u32 = 16;
    pub const BYTES: u32 = 17;
    pub const FILE: u32 = 18;
    pub const TEMPLATE: u32 = 19;
    pub const PROPERTYSET: u32 = 20;
    pub const PROPERTYSETLIST: u32 = 21;
    pub const INT8_ARRAY: u32 = 22;
    pub const INT16_ARRAY: u32 = 23;
    pub const INT32_ARRAY: u32 = 24;
    pub const INT64_ARRAY: u32 = 25;
    pub const UINT8_ARRAY: u32 = 26;
    pub const UINT16_ARRAY: u32 = 27;
    pub const UINT32_ARRAY: u32 = 28;
    pub const UINT64_ARRAY: u32 = 29;
    pub const FLOAT_ARRAY: u32 = 30;
    pub const DOUBLE_ARRAY: u32 = 31;
    pub const BOOLEAN_ARRAY: u32 = 32;
    pub const STRING_ARRAY: u32 = 33;
    pub const DATETIME_ARRAY: u32 = 34;
}
use types::*;

const TYPE_NAMES: [&str; 35] = [
    "Unknown", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64", "Float", "Double", "Boolean",
    "String", "DateTime", "Text", "UUID", "DataSet", "Bytes", "File", "Template", "PropertySet", "PropertySetList",
    "Int8Array", "Int16Array", "Int32Array", "Int64Array", "UInt8Array", "UInt16Array", "UInt32Array", "UInt64Array",
    "FloatArray", "DoubleArray", "BooleanArray", "StringArray", "DateTimeArray",
];

pub fn data_type_name(t: u32) -> &'static str {
    TYPE_NAMES.get(t as usize).copied().unwrap_or("Unknown")
}

//----------------------------------------------------------------------- topics

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Topic {
    pub group: String,
    /// NBIRTH, NDATA, ...
    pub kind: String,
    pub node: String,
    /// empty for edge node messages
    pub device: String,
}

/// `spBv1.0/<group>/<type>/<node>[/<device>]`; None for other topics (and STATE).
pub fn parse_topic(topic: &str) -> Option<Topic> {
    let p: Vec<&str> = topic.split('/').collect();
    if p.len() < 4 || p.len() > 5 || p[0] != NAMESPACE || p[1] == "STATE" || p.iter().any(|s| s.is_empty()) {
        return None;
    }
    Some(Topic {
        group: p[1].into(),
        kind: p[2].into(),
        node: p[3].into(),
        device: p.get(4).map(|s| s.to_string()).unwrap_or_default(),
    })
}

/// The subscription filter for one edge node and its devices.
pub fn node_filter(group: &str, node: &str) -> String {
    format!("{NAMESPACE}/{group}/+/{node}/#")
}

/// STATE topic and payload of a host application (Sparkplug 3.0).
pub fn state_topic(host_id: &str) -> String {
    format!("{NAMESPACE}/STATE/{host_id}")
}

pub fn state_payload(online: bool, timestamp_ms: i64) -> String {
    format!("{{\"online\":{online},\"timestamp\":{timestamp_ms}}}")
}

/// TRUE if a group, node, device or host id is usable in a topic (not empty, none of / + #).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '+', '#'])
}

//------------------------------------------------------------------------ values

fn is_signed(t: u32) -> bool {
    matches!(t, INT8 | INT16 | INT32 | INT64)
}

fn is_unsigned(t: u32) -> bool {
    matches!(t, UINT8 | UINT16 | UINT32 | UINT64)
}

fn element_size(t: u32) -> usize {
    match t {
        INT8_ARRAY | UINT8_ARRAY => 1,
        INT16_ARRAY | UINT16_ARRAY => 2,
        INT32_ARRAY | UINT32_ARRAY | FLOAT_ARRAY => 4,
        INT64_ARRAY | UINT64_ARRAY | DOUBLE_ARRAY | DATETIME_ARRAY => 8,
        _ => 0,
    }
}

fn le(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64)
}

fn finite_text<F: Into<f64> + std::fmt::Display + Copy>(f: F) -> Result<String, String> {
    if f.into().is_finite() { Ok(f.to_string()) } else { Err("non-finite float value".into()) }
}

fn array_text(t: u32, b: &[u8]) -> Result<Vec<u8>, String> {
    let mut items: Vec<Vec<u8>> = Vec::new();
    if t == BOOLEAN_ARRAY {
        if b.len() < 4 {
            return Err("BooleanArray shorter than its count".into());
        }
        let count = le(&b[..4]) as usize;
        if count.div_ceil(8) > b.len() - 4 {
            return Err("BooleanArray shorter than its count".into());
        }
        for k in 0..count {
            let bit = (b[4 + k / 8] >> (7 - (k % 8))) & 1 == 1; // most significant bit first
            items.push(if bit { b"true".to_vec() } else { b"false".to_vec() });
        }
    } else if t == STRING_ARRAY {
        let body = b.strip_suffix(b"\0").unwrap_or(b); // the last terminator (tolerated if missing)
        if !b.is_empty() {
            for s in body.split(|&c| c == 0) {
                items.push(json::quote(s));
            }
        }
    } else {
        let n = element_size(t);
        if n == 0 {
            return Err(format!("unsupported array type {}", data_type_name(t)));
        }
        if b.len() % n != 0 {
            return Err(format!("{} of {} bytes", data_type_name(t), b.len()));
        }
        for c in b.chunks(n) {
            let raw = le(c);
            let s = match t {
                INT8_ARRAY => (raw as u8 as i8).to_string(),
                INT16_ARRAY => (raw as u16 as i16).to_string(),
                INT32_ARRAY => (raw as u32 as i32).to_string(),
                INT64_ARRAY | DATETIME_ARRAY => (raw as i64).to_string(),
                FLOAT_ARRAY => finite_text(f32::from_bits(raw as u32)).map_err(|_| "FloatArray with a non-finite value")?,
                DOUBLE_ARRAY => finite_text(f64::from_bits(raw)).map_err(|_| "DoubleArray with a non-finite value")?,
                _ => raw.to_string(),
            };
            items.push(s.into_bytes());
        }
    }
    let mut out = b"[".to_vec();
    for (k, it) in items.iter().enumerate() {
        if k > 0 {
            out.push(b',');
        }
        out.extend_from_slice(it);
    }
    out.push(b']');
    Ok(out)
}

/// A JSON value for the scalar parts of DataSet, Template and PropertySet.
fn json_scalar(t: u32, int: Option<u32>, long: Option<u64>, f: Option<f32>, d: Option<f64>, b: Option<bool>, s: Option<&str>) -> Vec<u8> {
    let num = |x: String| x.into_bytes();
    if let Some(v) = int {
        return num(if is_signed(t) { (v as i32).to_string() } else { v.to_string() });
    }
    if let Some(v) = long {
        return num(if is_signed(t) || t == DATETIME { (v as i64).to_string() } else { v.to_string() });
    }
    if let Some(v) = f {
        return if v.is_finite() { num(v.to_string()) } else { b"null".to_vec() };
    }
    if let Some(v) = d {
        return if v.is_finite() { num(v.to_string()) } else { b"null".to_vec() };
    }
    if let Some(v) = b {
        return if v { b"true".to_vec() } else { b"false".to_vec() };
    }
    if let Some(v) = s {
        return json::quote(v.as_bytes());
    }
    b"null".to_vec()
}

fn join(out: &mut Vec<u8>, open: u8, items: Vec<Vec<u8>>, close: u8) {
    out.push(open);
    for (k, it) in items.into_iter().enumerate() {
        if k > 0 {
            out.push(b',');
        }
        out.extend_from_slice(&it);
    }
    out.push(close);
}

/// DataSet as JSON: {"columns":[...],"types":[...],"rows":[[...],...]}
fn dataset_json(ds: &DataSet) -> Vec<u8> {
    use data_set::data_set_value::Value as V;
    let cols = ds.columns.iter().map(|c| json::quote(c.as_bytes())).collect();
    let types = ds.types.iter().map(|t| json::quote(data_type_name(*t).as_bytes())).collect();
    let rows = ds
        .rows
        .iter()
        .map(|row| {
            let cells = row
                .elements
                .iter()
                .enumerate()
                .map(|(k, e)| {
                    let t = ds.types.get(k).copied().unwrap_or(UNKNOWN);
                    match &e.value {
                        Some(V::IntValue(v)) => json_scalar(t, Some(*v), None, None, None, None, None),
                        Some(V::LongValue(v)) => json_scalar(t, None, Some(*v), None, None, None, None),
                        Some(V::FloatValue(v)) => json_scalar(t, None, None, Some(*v), None, None, None),
                        Some(V::DoubleValue(v)) => json_scalar(t, None, None, None, Some(*v), None, None),
                        Some(V::BooleanValue(v)) => json_scalar(t, None, None, None, None, Some(*v), None),
                        Some(V::StringValue(v)) => json_scalar(t, None, None, None, None, None, Some(v)),
                        _ => b"null".to_vec(),
                    }
                })
                .collect();
            let mut r = Vec::new();
            join(&mut r, b'[', cells, b']');
            r
        })
        .collect();
    let mut out = b"{\"columns\":".to_vec();
    join(&mut out, b'[', cols, b']');
    out.extend_from_slice(b",\"types\":");
    join(&mut out, b'[', types, b']');
    out.extend_from_slice(b",\"rows\":");
    join(&mut out, b'[', rows, b']');
    out.push(b'}');
    out
}

/// Template as JSON: {"<member metric>": value, ...}, nested templates as objects.
fn template_json(t: &Template) -> Result<Vec<u8>, String> {
    let mut members = Vec::new();
    for m in &t.metrics {
        let Some(name) = &m.name else { continue };
        let ty = m.datatype.unwrap_or(UNKNOWN);
        let val = if m.is_null.unwrap_or(false) {
            b"null".to_vec()
        } else {
            let (text, is_string) = value_text(m, ty)?;
            match ty {
                _ if is_string => json::quote(&text),
                BYTES | FILE => json::quote(&text),
                _ => text,
            }
        };
        let mut e = json::quote(name.as_bytes());
        e.push(b':');
        e.extend_from_slice(&val);
        members.push(e);
    }
    let mut out = Vec::new();
    join(&mut out, b'{', members, b'}');
    Ok(out)
}

/// The value of a metric as text (see the module documentation). The bool
/// tells whether it is a string value (a dyn DPE then takes it as one item).
pub fn value_text(m: &Metric, mut t: u32) -> Result<(Vec<u8>, bool), String> {
    use metric::Value as V;
    let v = m.value.as_ref();
    if t == UNKNOWN {
        // no data type: by the value field
        t = match v {
            Some(V::IntValue(_)) => UINT32,
            Some(V::LongValue(_)) => UINT64,
            Some(V::FloatValue(_)) => FLOAT,
            Some(V::DoubleValue(_)) => DOUBLE,
            Some(V::BooleanValue(_)) => BOOLEAN,
            Some(V::StringValue(_)) => STRING,
            Some(V::BytesValue(_)) => BYTES,
            Some(V::DatasetValue(_)) => DATASET,
            Some(V::TemplateValue(_)) => TEMPLATE,
            _ => UNKNOWN,
        };
    }
    let name = data_type_name(t);
    let plain = |s: String| Ok((s.into_bytes(), false));
    if is_signed(t) {
        return match v {
            Some(V::IntValue(x)) => plain((*x as i32).to_string()),
            Some(V::LongValue(x)) => plain((*x as i64).to_string()),
            _ => Err(format!("{name} metric without an integer value")),
        };
    }
    if is_unsigned(t) {
        return match v {
            Some(V::IntValue(x)) => plain(x.to_string()),
            Some(V::LongValue(x)) => plain(x.to_string()),
            _ => Err(format!("{name} metric without an integer value")),
        };
    }
    match t {
        FLOAT | DOUBLE => match v {
            Some(V::FloatValue(x)) => plain(finite_text(*x)?),
            Some(V::DoubleValue(x)) => plain(finite_text(*x)?),
            _ => Err(format!("{name} metric without a float value")),
        },
        BOOLEAN => match v {
            Some(V::BooleanValue(x)) => plain(x.to_string()),
            _ => Err("Boolean metric without a boolean value".into()),
        },
        STRING | TEXT | UUID => match v {
            Some(V::StringValue(x)) => Ok((x.clone().into_bytes(), true)),
            _ => Err(format!("{name} metric without a string value")),
        },
        DATETIME => match v {
            Some(V::LongValue(x)) => plain((*x as i64).to_string()),
            _ => Err("DateTime metric without a long value".into()),
        },
        BYTES | FILE => match v {
            Some(V::BytesValue(x)) => Ok((x.clone(), false)),
            _ => Err(format!("{name} metric without bytes")),
        },
        DATASET => match v {
            Some(V::DatasetValue(ds)) => Ok((dataset_json(ds), false)),
            _ => Err("DataSet metric without a data set".into()),
        },
        TEMPLATE => match v {
            Some(V::TemplateValue(tp)) => Ok((template_json(tp)?, false)),
            _ => Err("Template metric without a template".into()),
        },
        PROPERTYSET | PROPERTYSETLIST => Err(format!("data type {name} has no metric value")),
        INT8_ARRAY..=DATETIME_ARRAY => match v {
            Some(V::BytesValue(x)) => Ok((array_text(t, x)?, false)),
            _ => Err(format!("{name} metric without bytes")),
        },
        _ => Err(format!("data type {name} is not supported")),
    }
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().strip_prefix('+').unwrap_or(s.trim()).parse().ok()
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" => Some(true),
        "false" | "0" | "off" | "no" => Some(false),
        other => parse_f64(other).map(|d| d != 0.0),
    }
}

/// Integer text (also "12.0", "1e3") within [lo, hi].
fn parse_signed(s: &str, lo: i64, hi: i64) -> Option<i64> {
    let t = s.trim();
    let t = t.strip_prefix('+').unwrap_or(t);
    let v = match t.parse::<i64>() {
        Ok(v) => v,
        Err(_) => {
            let d = parse_f64(t)?;
            if d.floor() != d || d < lo as f64 || d > hi as f64 {
                return None;
            }
            d as i64
        }
    };
    (lo..=hi).contains(&v).then_some(v)
}

fn parse_unsigned(s: &str, hi: u64) -> Option<u64> {
    let t = s.trim();
    let t = t.strip_prefix('+').unwrap_or(t);
    let v = match t.parse::<u64>() {
        Ok(v) => v,
        Err(_) => {
            let d = parse_f64(t)?;
            if d.floor() != d || d < 0.0 || d > hi as f64 {
                return None;
            }
            d as u64
        }
    };
    (v <= hi).then_some(v)
}

fn int_range(t: u32) -> Option<(i64, i64)> {
    match t {
        INT8 | INT8_ARRAY => Some((i8::MIN as i64, i8::MAX as i64)),
        INT16 | INT16_ARRAY => Some((i16::MIN as i64, i16::MAX as i64)),
        INT32 | INT32_ARRAY => Some((i32::MIN as i64, i32::MAX as i64)),
        INT64 | INT64_ARRAY | DATETIME | DATETIME_ARRAY => Some((i64::MIN, i64::MAX)),
        _ => None,
    }
}

fn uint_max(t: u32) -> u64 {
    match t {
        UINT8 | UINT8_ARRAY => u8::MAX as u64,
        UINT16 | UINT16_ARRAY => u16::MAX as u64,
        UINT32 | UINT32_ARRAY => u32::MAX as u64,
        _ => u64::MAX,
    }
}

fn pack_array(t: u32, text: &[u8]) -> Result<Vec<u8>, String> {
    let name = data_type_name(t);
    let items = json::parse_array(text).map_err(|e| format!("{name} needs a JSON array: {e}"))?;
    let mut out = Vec::new();
    let item = |k: usize, it: &json::Value| String::from_utf8(it.text.clone()).map_err(|_| format!("item {k} is not UTF-8"));
    if t == BOOLEAN_ARRAY {
        out.extend_from_slice(&(items.len() as u32).to_le_bytes());
        let mut bits = vec![0u8; items.len().div_ceil(8)];
        for (k, it) in items.iter().enumerate() {
            if parse_bool(&item(k, it)?).ok_or(format!("item {k} is not a boolean"))? {
                bits[k / 8] |= 0x80 >> (k % 8);
            }
        }
        out.extend_from_slice(&bits);
        return Ok(out);
    }
    if t == STRING_ARRAY {
        for it in &items {
            out.extend_from_slice(&it.text);
            out.push(0);
        }
        return Ok(out);
    }
    let n = element_size(t);
    if n == 0 {
        return Err(format!("unsupported array type {name}"));
    }
    for (k, it) in items.iter().enumerate() {
        let s = item(k, it)?;
        match t {
            FLOAT_ARRAY => out.extend_from_slice(&(parse_f64(&s).ok_or(format!("item {k} is not a number"))? as f32).to_le_bytes()),
            DOUBLE_ARRAY => out.extend_from_slice(&parse_f64(&s).ok_or(format!("item {k} is not a number"))?.to_le_bytes()),
            _ => {
                let raw = if let Some((lo, hi)) = int_range(t) {
                    parse_signed(&s, lo, hi).ok_or(format!("item {k} is not a {name} value"))? as u64
                } else {
                    parse_unsigned(&s, uint_max(t)).ok_or(format!("item {k} is not a {name} value"))?
                };
                out.extend_from_slice(&raw.to_le_bytes()[..n]);
            }
        }
    }
    Ok(out)
}

/// Sets the value of `m` from the text the driver's transformation writes
/// (dyn: a JSON array).
pub fn set_value(m: &mut Metric, t: u32, text: &[u8]) -> Result<(), String> {
    use metric::Value as V;
    m.datatype = Some(t);
    let name = data_type_name(t);
    let s = || String::from_utf8_lossy(text).into_owned();
    let bad = || {
        let shown: String = s().chars().take(40).collect();
        format!("'{shown}' is not a {name} value")
    };
    m.value = Some(match t {
        INT8 | INT16 | INT32 => {
            let (lo, hi) = int_range(t).unwrap();
            V::IntValue(parse_signed(&s(), lo, hi).ok_or_else(bad)? as i32 as u32)
        }
        INT64 | DATETIME => V::LongValue(parse_signed(&s(), i64::MIN, i64::MAX).ok_or_else(bad)? as u64),
        UINT8 | UINT16 => V::IntValue(parse_unsigned(&s(), uint_max(t)).ok_or_else(bad)? as u32),
        UINT32 | UINT64 => V::LongValue(parse_unsigned(&s(), uint_max(t)).ok_or_else(bad)?), // Sparkplug 3.0: long_value
        FLOAT => V::FloatValue(parse_f64(&s()).ok_or_else(bad)? as f32),
        DOUBLE => V::DoubleValue(parse_f64(&s()).ok_or_else(bad)?),
        BOOLEAN => V::BooleanValue(parse_bool(&s()).ok_or_else(bad)?),
        STRING | TEXT | UUID => V::StringValue(String::from_utf8(text.to_vec()).map_err(|_| format!("{name} value is not UTF-8"))?),
        BYTES | FILE => V::BytesValue(text.to_vec()),
        INT8_ARRAY..=DATETIME_ARRAY => V::BytesValue(pack_array(t, text)?),
        _ => return Err(format!("data type {name} cannot be written")),
    });
    Ok(())
}

//-------------------------------------------------------------------------- host

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKind {
    Value = 0,
    Online = 1,
    Stale = 2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub kind: UpdateKind,
    pub group: String,
    pub node: String,
    /// empty: the edge node itself
    pub device: String,
    /// empty for Online
    pub metric: String,
    /// Value: the value
    pub text: Vec<u8>,
    pub is_string: bool,
    /// Online
    pub online: bool,
    pub time_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutMessage {
    pub topic: String,
    pub payload: Vec<u8>,
}

#[derive(Default)]
struct Entity {
    online: bool,
    /// metric name -> data type
    types: BTreeMap<String, u32>,
    /// alias -> metric name
    aliases: BTreeMap<u64, String>,
}

struct Node {
    this: Entity,
    devices: BTreeMap<String, Entity>,
    seq: Option<u64>,
    bd_seq: Option<u64>,
    last_rebirth_ms: i64,
}

impl Default for Node {
    fn default() -> Self {
        Node { this: Entity::default(), devices: BTreeMap::new(), seq: None, bd_seq: None, last_rebirth_ms: i64::MIN / 2 }
    }
}

/// What one message or event leads to.
#[derive(Default, Debug)]
pub struct Outcome {
    pub updates: Vec<Update>,
    /// rebirth requests to publish (QoS 0)
    pub publish: Vec<OutMessage>,
    pub warnings: Vec<String>,
}

pub struct Host {
    nodes: BTreeMap<(String, String), Node>,
    /// minimum time between two rebirth requests to one edge node
    pub rebirth_interval_ms: i64,
}

impl Default for Host {
    fn default() -> Self {
        Host { nodes: BTreeMap::new(), rebirth_interval_ms: 5000 }
    }
}

fn place(t: &Topic) -> String {
    if t.device.is_empty() { format!("{}/{}", t.group, t.node) } else { format!("{}/{}/{}", t.group, t.node, t.device) }
}

fn metric_uint(m: &Metric) -> Option<u64> {
    match m.value {
        Some(metric::Value::LongValue(v)) => Some(v),
        Some(metric::Value::IntValue(v)) => Some(v as u64),
        _ => None,
    }
}

fn bd_seq(p: &Payload) -> Option<u64> {
    p.metrics.iter().find(|m| m.name.as_deref() == Some("bdSeq")).and_then(metric_uint)
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one message.
    pub fn on_message(&mut self, topic: &str, payload: &[u8], now_ms: i64) -> Outcome {
        let mut o = Outcome::default();
        let Some(t) = parse_topic(topic) else { return o };
        if t.kind == "NCMD" || t.kind == "DCMD" {
            return o; // commands of (other) host applications
        }
        let node_level = t.kind.starts_with('N');
        if node_level != t.device.is_empty() {
            return o; // NBIRTH with a device or DBIRTH without one: not Sparkplug
        }
        let p = match Payload::decode(payload) {
            Ok(p) => p,
            Err(e) => {
                o.warnings.push(format!("{} from {}: {}", t.kind, place(&t), e));
                return o;
            }
        };
        let interval = self.rebirth_interval_ms;
        let n = self.nodes.entry((t.group.clone(), t.node.clone())).or_default();
        match t.kind.as_str() {
            "NBIRTH" => {
                // a new session: the devices of the previous one are gone until their DBIRTH
                for (dev, e) in n.devices.iter_mut() {
                    if e.online {
                        died(&Topic { device: dev.clone(), ..t.clone() }, e, &mut o);
                    }
                }
                n.devices.clear();
                n.last_rebirth_ms = i64::MIN / 2; // the birth came: a new problem may ask again at once
                n.seq = p.seq;
                n.bd_seq = bd_seq(&p);
                born(&t, &mut n.this, &p, &mut o);
            }
            "NDEATH" => {
                if let (Some(cur), Some(bd)) = (n.bd_seq, bd_seq(&p)) {
                    if bd != cur {
                        return o; // the death of an earlier session (its will arrived late)
                    }
                }
                for (dev, e) in n.devices.iter_mut() {
                    if e.online {
                        died(&Topic { device: dev.clone(), ..t.clone() }, e, &mut o);
                    }
                }
                if n.this.online {
                    died(&t, &mut n.this, &mut o);
                }
                n.seq = None;
            }
            "DBIRTH" => {
                if !n.this.online {
                    rebirth(&t, n, now_ms, interval, &mut o, "DBIRTH without NBIRTH");
                    return o;
                }
                if !check_seq(&t, n, &p, &mut o) {
                    rebirth(&t, n, now_ms, interval, &mut o, "missed messages");
                }
                let e = n.devices.entry(t.device.clone()).or_default();
                born(&t, e, &p, &mut o);
            }
            "DDEATH" => {
                if !n.this.online {
                    return o;
                }
                if !check_seq(&t, n, &p, &mut o) {
                    rebirth(&t, n, now_ms, interval, &mut o, "missed messages");
                }
                if let Some(e) = n.devices.get_mut(&t.device) {
                    if e.online {
                        died(&t, e, &mut o);
                    }
                }
            }
            "NDATA" => {
                if !n.this.online {
                    rebirth(&t, n, now_ms, interval, &mut o, "NDATA without NBIRTH");
                    return o;
                }
                if !check_seq(&t, n, &p, &mut o) {
                    rebirth(&t, n, now_ms, interval, &mut o, "missed messages");
                }
                let missing = data(&t, &n.this, &p, &mut o);
                if let Some(why) = missing {
                    rebirth(&t, n, now_ms, interval, &mut o, &why);
                }
            }
            "DDATA" => {
                if !n.this.online {
                    rebirth(&t, n, now_ms, interval, &mut o, "DDATA without NBIRTH");
                    return o;
                }
                if !n.devices.get(&t.device).is_some_and(|e| e.online) {
                    rebirth(&t, n, now_ms, interval, &mut o, "DDATA without DBIRTH");
                    return o;
                }
                if !check_seq(&t, n, &p, &mut o) {
                    rebirth(&t, n, now_ms, interval, &mut o, "missed messages");
                }
                let missing = data(&t, &n.devices[&t.device], &p, &mut o);
                if let Some(why) = missing {
                    rebirth(&t, n, now_ms, interval, &mut o, &why);
                }
            }
            _ => {}
        }
        o
    }

    /// The connection to the broker is gone: every edge node and device is
    /// offline and its metrics stale until the next birth.
    pub fn on_connection_lost(&mut self) -> Outcome {
        let mut o = Outcome::default();
        for ((group, node), n) in self.nodes.iter_mut() {
            let t = Topic { group: group.clone(), kind: String::new(), node: node.clone(), device: String::new() };
            for (dev, e) in n.devices.iter_mut() {
                if e.online {
                    died(&Topic { device: dev.clone(), ..t.clone() }, e, &mut o);
                }
            }
            if n.this.online {
                died(&t, &mut n.this, &mut o);
            }
        }
        self.nodes.clear();
        o
    }

    fn entity(&self, group: &str, node: &str, device: &str) -> Option<&Entity> {
        let n = self.nodes.get(&(group.to_string(), node.to_string()))?;
        if device.is_empty() { Some(&n.this) } else { n.devices.get(device) }
    }

    /// NCMD (device empty) or DCMD that writes `text` to a metric.
    pub fn command(&self, group: &str, node: &str, device: &str, metric: &str, text: &[u8], now_ms: i64) -> Result<OutMessage, String> {
        let what = if device.is_empty() { format!("{group}/{node}") } else { format!("{group}/{node}/{device}") };
        let e = self.entity(group, node, device).filter(|e| e.online).ok_or(format!("{what} is offline (no birth received)"))?;
        let t = *e.types.get(metric).ok_or(format!("metric '{metric}' is not in the birth of {what}"))?;
        let mut m = Metric::new();
        m.name = Some(metric.to_string());
        m.timestamp = Some(now_ms as u64);
        set_value(&mut m, t, text).map_err(|e| format!("metric '{metric}' of {what}: {e}"))?;
        let p = Payload { timestamp: Some(now_ms as u64), metrics: vec![m], seq: None, uuid: None, body: None };
        let topic = if device.is_empty() {
            format!("{NAMESPACE}/{group}/NCMD/{node}")
        } else {
            format!("{NAMESPACE}/{group}/DCMD/{node}/{device}")
        };
        Ok(OutMessage { topic, payload: p.encode_to_vec() })
    }

    /// The data type of a metric from its birth (UNKNOWN if not born).
    pub fn data_type(&self, group: &str, node: &str, device: &str, metric: &str) -> u32 {
        self.entity(group, node, device).and_then(|e| e.types.get(metric).copied()).unwrap_or(UNKNOWN)
    }

    pub fn is_online(&self, group: &str, node: &str, device: &str) -> bool {
        self.entity(group, node, device).is_some_and(|e| e.online)
    }
}

fn check_seq(t: &Topic, n: &mut Node, p: &Payload, o: &mut Outcome) -> bool {
    let Some(seq) = p.seq else { return true };
    let ok = n.seq.is_none_or(|cur| seq == (cur + 1) % 256);
    if !ok {
        o.warnings.push(format!("{} from {}: seq {}, expected {}", t.kind, place(t), seq, (n.seq.unwrap() + 1) % 256));
    }
    n.seq = Some(seq);
    ok
}

fn rebirth(t: &Topic, n: &mut Node, now_ms: i64, interval: i64, o: &mut Outcome, why: &str) {
    if now_ms - n.last_rebirth_ms < interval {
        return;
    }
    n.last_rebirth_ms = now_ms;
    let mut m = Metric::new();
    m.name = Some(REBIRTH_METRIC.into());
    m.timestamp = Some(now_ms as u64);
    m.datatype = Some(BOOLEAN);
    m.value = Some(metric::Value::BooleanValue(true));
    let p = Payload { timestamp: Some(now_ms as u64), metrics: vec![m], seq: None, uuid: None, body: None };
    o.publish.push(OutMessage { topic: format!("{NAMESPACE}/{}/NCMD/{}", t.group, t.node), payload: p.encode_to_vec() });
    let dev = if t.device.is_empty() { String::new() } else { format!(" (device {})", t.device) };
    o.warnings.push(format!("edge node {}/{}: {}{}, rebirth requested", t.group, t.node, why, dev));
}

fn update(t: &Topic, kind: UpdateKind, metric: &str) -> Update {
    Update {
        kind,
        group: t.group.clone(),
        node: t.node.clone(),
        device: t.device.clone(),
        metric: metric.to_string(),
        text: Vec::new(),
        is_string: false,
        online: false,
        time_ms: None,
    }
}

fn born(t: &Topic, e: &mut Entity, p: &Payload, o: &mut Outcome) {
    e.types.clear();
    e.aliases.clear();
    for m in &p.metrics {
        let Some(name) = m.name.as_deref().filter(|n| !n.is_empty()) else { continue }; // births name every metric
        let ty = m.datatype.unwrap_or(UNKNOWN);
        e.types.insert(name.to_string(), ty);
        if let Some(a) = m.alias {
            e.aliases.insert(a, name.to_string());
        }
        metric_update(t, name, ty, m, p, o);
    }
    e.online = true;
    let mut u = update(t, UpdateKind::Online, "");
    u.online = true;
    o.updates.push(u);
}

/// The values of NDATA / DDATA; Some(reason) if something asks for a rebirth.
fn data(t: &Topic, e: &Entity, p: &Payload, o: &mut Outcome) -> Option<String> {
    let mut rebirth_reason = None;
    for m in &p.metrics {
        let name = match (m.name.as_deref().filter(|n| !n.is_empty()), m.alias) {
            (Some(n), _) => n.to_string(),
            (None, Some(a)) => match e.aliases.get(&a) {
                Some(n) => n.clone(),
                None => {
                    rebirth_reason.get_or_insert(format!("unknown alias {a}"));
                    continue;
                }
            },
            (None, None) => continue,
        };
        let Some(&ty) = e.types.get(&name) else {
            rebirth_reason.get_or_insert(format!("metric '{name}' not in its birth"));
            continue;
        };
        metric_update(t, &name, m.datatype.unwrap_or(ty), m, p, o);
    }
    rebirth_reason
}

fn metric_update(t: &Topic, name: &str, ty: u32, m: &Metric, p: &Payload, o: &mut Outcome) {
    if m.is_historical.unwrap_or(false) {
        return; // older than the current values (WinCC OA keeps values in time order)
    }
    let mut u = update(t, UpdateKind::Value, name);
    u.time_ms = m.timestamp.or(p.timestamp).map(|x| x as i64);
    if m.is_null.unwrap_or(false) {
        u.kind = UpdateKind::Stale; // no value: the last one becomes invalid
        o.updates.push(u);
        return;
    }
    match value_text(m, ty) {
        Ok((text, is_string)) => {
            u.text = text;
            u.is_string = is_string;
            o.updates.push(u);
        }
        Err(e) => o.warnings.push(format!("metric '{}' of {}: {}", name, place(t), e)),
    }
}

fn died(t: &Topic, e: &mut Entity, o: &mut Outcome) {
    e.online = false;
    for name in e.types.keys() {
        o.updates.push(update(t, UpdateKind::Stale, name));
    }
    o.updates.push(update(t, UpdateKind::Online, ""));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(name: Option<&str>, t: u32, text: Option<&str>, alias: Option<u64>) -> Metric {
        let mut m = Metric::new();
        m.name = name.map(|s| s.to_string());
        m.alias = alias;
        m.datatype = Some(t);
        if let Some(text) = text {
            set_value(&mut m, t, text.as_bytes()).unwrap_or_else(|e| panic!("{name:?}: {e}"));
        }
        m
    }

    fn payload(metrics: Vec<Metric>, seq: Option<u64>) -> Vec<u8> {
        Payload { timestamp: Some(1700000000000), metrics, seq, uuid: None, body: None }.encode_to_vec()
    }

    fn text(m: &Metric, t: u32) -> String {
        match value_text(m, t) {
            Ok((v, _)) => String::from_utf8_lossy(&v).into_owned(),
            Err(e) => format!("ERR: {e}"),
        }
    }

    #[test]
    fn codec_round_trip() {
        let ms = vec![
            metric(Some("i8"), INT8, Some("-5"), None),
            metric(Some("i64"), INT64, Some("-9007199254740993"), None),
            metric(Some("u32"), UINT32, Some("4000000000"), None),
            metric(Some("f"), FLOAT, Some("1.5"), None),
            metric(Some("d"), DOUBLE, Some("-0.125"), None),
            metric(Some("b"), BOOLEAN, Some("on"), None),
            metric(Some("s"), STRING, Some("h\u{e9}llo"), None),
            metric(Some("dt"), DATETIME, Some("1700000000000"), None),
            metric(Some("by"), BYTES, Some("ab"), Some(300)),
        ];
        let q = Payload::decode(&payload(ms, Some(7))[..]).unwrap();
        assert_eq!(q.seq, Some(7));
        let expect = ["-5", "-9007199254740993", "4000000000", "1.5", "-0.125", "true", "h\u{e9}llo", "1700000000000", "ab"];
        let types = [INT8, INT64, UINT32, FLOAT, DOUBLE, BOOLEAN, STRING, DATETIME, BYTES];
        for k in 0..expect.len() {
            assert_eq!(text(&q.metrics[k], types[k]), expect[k]);
        }
        assert_eq!(q.metrics[0].value, Some(metric::Value::IntValue(0xFFFF_FFFB))); // two's complement
        assert_eq!(q.metrics[2].value, Some(metric::Value::LongValue(4000000000)));
        assert_eq!(q.metrics[8].alias, Some(300));
        assert_eq!(text(&q.metrics[3], UNKNOWN), "1.5"); // by the value field

        let mut r = Metric::new();
        assert!(set_value(&mut r, INT8, b"200").is_err() && set_value(&mut r, UINT16, b"-1").is_err());
        assert!(set_value(&mut r, INT32, b"12.0").is_ok() && set_value(&mut r, INT32, b"12.5").is_err());
        assert!(set_value(&mut r, BOOLEAN, b"maybe").is_err() && set_value(&mut r, DATASET, b"x").is_err());
        assert!(Payload::decode(&b"\x12\x05\x0a\x03"[..]).is_err());
    }

    #[test]
    fn arrays() {
        for (t, j) in [
            (INT8_ARRAY, "[-1,0,127]"), (INT16_ARRAY, "[-300,300]"), (INT32_ARRAY, "[-70000,1]"),
            (INT64_ARRAY, "[-9007199254740993]"), (UINT8_ARRAY, "[0,255]"), (UINT16_ARRAY, "[65535]"),
            (UINT32_ARRAY, "[4000000000]"), (UINT64_ARRAY, "[18446744073709551615]"), (FLOAT_ARRAY, "[1.5,-2]"),
            (DOUBLE_ARRAY, "[0.1,2]"), (DATETIME_ARRAY, "[1700000000000]"),
            (BOOLEAN_ARRAY, "[true,false,true,true,false,false,false,false,true]"),
            (STRING_ARRAY, r#"["a","","c\"d"]"#), (INT32_ARRAY, "[]"),
        ] {
            let mut m = Metric::new();
            set_value(&mut m, t, j.as_bytes()).unwrap();
            assert_eq!(text(&m, t), j, "{}", data_type_name(t));
        }
        let bytes = |t: u32, j: &str| {
            let mut m = Metric::new();
            set_value(&mut m, t, j.as_bytes()).unwrap();
            match m.value {
                Some(metric::Value::BytesValue(b)) => b,
                _ => panic!(),
            }
        };
        assert_eq!(bytes(BOOLEAN_ARRAY, "[true,false,true]"), b"\x03\x00\x00\x00\xA0"); // count, MSB first
        assert_eq!(bytes(STRING_ARRAY, r#"["ab","c"]"#), b"ab\0c\0");
        assert_eq!(bytes(INT16_ARRAY, "[1,-2]"), b"\x01\x00\xFE\xFF");
        let mut m = Metric::new();
        assert!(set_value(&mut m, INT8_ARRAY, b"[300]").is_err() && set_value(&mut m, INT8_ARRAY, b"1").is_err());
        m.value = Some(metric::Value::BytesValue(b"\x01\x02\x03".to_vec()));
        assert!(text(&m, INT16_ARRAY).starts_with("ERR"));
    }

    #[test]
    fn complex_types() {
        use data_set::{data_set_value, DataSetValue, Row};
        let ds = DataSet {
            num_of_columns: Some(2),
            columns: vec!["id".into(), "name".into()],
            types: vec![INT32, STRING],
            rows: vec![Row {
                elements: vec![
                    DataSetValue { value: Some(data_set_value::Value::IntValue((-1i32) as u32)) },
                    DataSetValue { value: Some(data_set_value::Value::StringValue("a\"b".into())) },
                ],
            }],
        };
        let mut m = Metric::new();
        m.value = Some(metric::Value::DatasetValue(ds));
        assert_eq!(text(&m, DATASET), r#"{"columns":["id","name"],"types":["Int32","String"],"rows":[[-1,"a\"b"]]}"#);

        let member = |n: &str, t: u32, v: &str| metric(Some(n), t, Some(v), None);
        let tpl = Template {
            version: None,
            metrics: vec![member("speed", INT32, "1450"), member("label", STRING, "M1"), member("on", BOOLEAN, "true")],
            parameters: vec![],
            template_ref: Some("Motor".into()),
            is_definition: Some(false),
        };
        let mut t = Metric::new();
        t.value = Some(metric::Value::TemplateValue(tpl));
        assert_eq!(text(&t, TEMPLATE), r#"{"speed":1450,"label":"M1","on":true}"#);
    }

    #[test]
    fn topics() {
        let t = parse_topic("spBv1.0/Factory/DDATA/Line1/PLC7").unwrap();
        assert!(t.group == "Factory" && t.kind == "DDATA" && t.node == "Line1" && t.device == "PLC7");
        assert!(parse_topic("spBv1.0/Factory/NBIRTH/Line1").unwrap().device.is_empty());
        assert!(parse_topic("spBv1.0/STATE/host").is_none() && parse_topic("spBv1.0/G/NDATA").is_none());
        assert_eq!(node_filter("G", "N"), "spBv1.0/G/+/N/#");
        assert_eq!(state_topic("scada"), "spBv1.0/STATE/scada");
        assert_eq!(state_payload(false, 1700000000000), r#"{"online":false,"timestamp":1700000000000}"#);
        assert!(valid_id("Line1") && !valid_id("a/b") && !valid_id("a+") && !valid_id(""));
    }

    fn find<'a>(o: &'a Outcome, kind: UpdateKind, metric: &str, device: &str) -> Option<&'a Update> {
        o.updates.iter().find(|u| u.kind == kind && u.metric == metric && u.device == device)
    }

    #[test]
    fn host() {
        let mut h = Host::new();
        let now = 1_000_000;
        let d = |t: u32, v: &str| metric(Some("temp"), t, Some(v), None);

        // data before any birth: rebirth request
        let o = h.on_message("spBv1.0/G/NDATA/N", &payload(vec![d(DOUBLE, "1")], Some(1)), now);
        assert!(o.updates.is_empty() && o.publish.len() == 1 && o.publish[0].topic == "spBv1.0/G/NCMD/N");
        let rb = Payload::decode(&o.publish[0].payload[..]).unwrap();
        assert_eq!(rb.metrics[0].name.as_deref(), Some("Node Control/Rebirth"));
        assert_eq!(rb.metrics[0].value, Some(metric::Value::BooleanValue(true)));

        // NBIRTH
        let birth = vec![
            metric(Some("bdSeq"), INT64, Some("3"), None),
            metric(Some("temp"), DOUBLE, Some("21.5"), Some(1)),
            metric(Some("Node Control/Rebirth"), BOOLEAN, Some("false"), None),
        ];
        let o = h.on_message("spBv1.0/G/NBIRTH/N", &payload(birth, Some(0)), now);
        let x = find(&o, UpdateKind::Value, "temp", "").unwrap();
        assert!(x.text == b"21.5" && x.time_ms == Some(1700000000000));
        assert!(find(&o, UpdateKind::Online, "", "").unwrap().online && o.publish.is_empty());
        assert!(h.is_online("G", "N", "") && h.data_type("G", "N", "", "temp") == DOUBLE);

        // alias only
        let o = h.on_message("spBv1.0/G/NDATA/N", &payload(vec![metric(None, DOUBLE, Some("22.25"), Some(1))], Some(1)), now);
        assert!(find(&o, UpdateKind::Value, "temp", "").unwrap().text == b"22.25" && o.publish.is_empty() && o.warnings.is_empty());

        // a gap in seq: the data counts, a rebirth is requested (at most once per interval)
        let o = h.on_message("spBv1.0/G/NDATA/N", &payload(vec![d(DOUBLE, "23")], Some(3)), now);
        assert!(find(&o, UpdateKind::Value, "temp", "").is_some() && o.publish.len() == 1 && !o.warnings.is_empty());
        let o = h.on_message("spBv1.0/G/NDATA/N", &payload(vec![d(DOUBLE, "23")], Some(9)), now + 100);
        assert!(o.publish.is_empty());

        // unknown alias
        let o = h.on_message("spBv1.0/G/NDATA/N", &payload(vec![metric(None, DOUBLE, Some("1"), Some(99))], Some(10)), now + 10_000);
        assert!(o.updates.is_empty() && o.publish.len() == 1);

        // device birth and data
        let o = h.on_message(
            "spBv1.0/G/DBIRTH/N/PLC7",
            &payload(vec![metric(Some("Motor/speed"), INT32, Some("-12"), None), metric(Some("Motor/run"), BOOLEAN, Some("true"), None)], Some(11)),
            now,
        );
        assert!(find(&o, UpdateKind::Value, "Motor/speed", "PLC7").unwrap().text == b"-12");
        assert!(find(&o, UpdateKind::Online, "", "PLC7").is_some());
        let o = h.on_message("spBv1.0/G/DDATA/N/PLC7", &payload(vec![metric(Some("Motor/speed"), INT32, Some("1450"), None)], Some(12)), now);
        assert!(find(&o, UpdateKind::Value, "Motor/speed", "PLC7").unwrap().text == b"1450" && o.publish.is_empty());

        // DDATA of a device without birth
        let o = h.on_message("spBv1.0/G/DDATA/N/PLC9", &payload(vec![metric(Some("x"), INT32, Some("1"), None)], Some(13)), now + 20_000);
        assert!(o.updates.is_empty() && o.publish.len() == 1);

        // commands with the data type of the birth
        let c = h.command("G", "N", "PLC7", "Motor/speed", b"42", now).unwrap();
        let cp = Payload::decode(&c.payload[..]).unwrap();
        assert!(c.topic == "spBv1.0/G/DCMD/N/PLC7" && cp.metrics[0].datatype == Some(INT32));
        assert_eq!(cp.metrics[0].value, Some(metric::Value::IntValue(42)));
        assert_eq!(h.command("G", "N", "", "temp", b"19.5", now).unwrap().topic, "spBv1.0/G/NCMD/N");
        assert!(h.command("G", "N", "PLC7", "Motor/speed", b"fast", now).is_err());
        assert!(h.command("G", "N", "PLC7", "unknown", b"1", now).is_err());
        assert!(h.command("G", "X", "", "temp", b"1", now).is_err());

        // NDEATH of an earlier session, then the real one
        let o = h.on_message("spBv1.0/G/NDEATH/N", &payload(vec![metric(Some("bdSeq"), INT64, Some("2"), None)], None), now);
        assert!(o.updates.is_empty() && h.is_online("G", "N", ""));
        let o = h.on_message("spBv1.0/G/NDEATH/N", &payload(vec![metric(Some("bdSeq"), INT64, Some("3"), None)], None), now);
        assert!(find(&o, UpdateKind::Stale, "temp", "").is_some() && find(&o, UpdateKind::Stale, "Motor/speed", "PLC7").is_some());
        assert!(!find(&o, UpdateKind::Online, "", "").unwrap().online && !find(&o, UpdateKind::Online, "", "PLC7").unwrap().online);
        assert!(!h.is_online("G", "N", "") && !h.is_online("G", "N", "PLC7") && h.command("G", "N", "", "temp", b"1", now).is_err());

        // a null value makes the metric stale
        h.on_message("spBv1.0/G/NBIRTH/N", &payload(vec![metric(Some("bdSeq"), INT64, Some("4"), None), d(DOUBLE, "20")], Some(0)), now);
        let mut nul = metric(Some("temp"), DOUBLE, None, None);
        nul.is_null = Some(true);
        let o = h.on_message("spBv1.0/G/NDATA/N", &payload(vec![nul], Some(1)), now);
        assert!(find(&o, UpdateKind::Stale, "temp", "").is_some() && find(&o, UpdateKind::Value, "temp", "").is_none());

        // the broker connection is lost
        let o = h.on_connection_lost();
        assert!(find(&o, UpdateKind::Stale, "temp", "").is_some() && !h.is_online("G", "N", ""));

        // NCMD of other hosts and STATE are not ours
        assert!(h.on_message("spBv1.0/G/NCMD/N", &payload(vec![d(INT32, "1")], None), now).updates.is_empty());
        assert!(h.on_message("spBv1.0/STATE/other", b"{\"online\":true}", now).updates.is_empty());
    }
}
