//! C ABI of rsmqtt-wincc (`include/rsmqtt_wincc.h`).
//!
//! Conventions:
//! - strings in: `const char *` (NUL-terminated) for names, `(ptr, len)` for
//!   payloads and values (they may contain NUL bytes);
//! - owned results: `rsmqtt_wincc_buf`, freed with `rsmqtt_wincc_buf_free`;
//! - results kept in a handle (address fields, extractor results, Sparkplug
//!   updates) stay valid until the next call that changes that handle;
//! - failures: -1 / NULL and `rsmqtt_wincc_last_error()`;
//! - no panic crosses the boundary.

#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use crate::{address, json, redu, sparkplug};

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: &str) {
    let c = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
}

/// The message of the last failed call of this thread ("" if none).
#[no_mangle]
pub extern "C" fn rsmqtt_wincc_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

fn guard<T, F: FnOnce() -> Result<T, String>>(fail: T, f: F) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            set_error(&e);
            fail
        }
        Err(_) => {
            set_error("internal error (panic) in rsmqtt-wincc");
            fail
        }
    }
}

unsafe fn bytes<'a>(p: *const c_char, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 { &[] } else { std::slice::from_raw_parts(p as *const u8, len) }
}

unsafe fn text<'a>(p: *const c_char) -> std::borrow::Cow<'a, str> {
    if p.is_null() { "".into() } else { CStr::from_ptr(p).to_string_lossy() }
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_default()
}

//------------------------------------------------------------------------ buffers

/// An owned byte buffer (not NUL-terminated); free with rsmqtt_wincc_buf_free.
#[repr(C)]
pub struct RsmqttWinccBuf {
    pub data: *mut c_char,
    pub len: usize,
}

fn buf(v: Vec<u8>) -> RsmqttWinccBuf {
    let b = v.into_boxed_slice();
    let len = b.len();
    RsmqttWinccBuf { data: Box::into_raw(b) as *mut c_char, len }
}

unsafe fn put(out: *mut RsmqttWinccBuf, v: Vec<u8>) -> c_int {
    if out.is_null() {
        set_error("null output buffer");
        return -1;
    }
    *out = buf(v);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_buf_free(b: RsmqttWinccBuf) {
    if !b.data.is_null() {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(b.data as *mut u8, b.len)));
    }
}

//------------------------------------------------------------------------ address

pub struct RsmqttWinccAddress {
    a: address::Address,
    texts: Vec<CString>,
}

pub const ADDR_CONNECTION: c_int = 0;
pub const ADDR_TOPIC: c_int = 1;
pub const ADDR_JSON_PATH: c_int = 2;
pub const ADDR_TS_PATH: c_int = 3;
pub const ADDR_SPB_GROUP: c_int = 4;
pub const ADDR_SPB_NODE: c_int = 5;
pub const ADDR_SPB_DEVICE: c_int = 6;
pub const ADDR_SPB_METRIC: c_int = 7;

pub const ADDR_QOS: c_int = 0;
pub const ADDR_RETAIN: c_int = 1;
pub const ADDR_SUB_FLAGS: c_int = 2;
pub const ADDR_IS_STATE: c_int = 3;
pub const ADDR_JSON: c_int = 4;
pub const ADDR_HAS_TS: c_int = 5;
pub const ADDR_SPARKPLUG: c_int = 6;
pub const ADDR_SPB_ONLINE: c_int = 7;

/// Parses an address; NULL and rsmqtt_wincc_last_error() if it is not valid.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_address_parse(t: *const c_char, len: usize) -> *mut RsmqttWinccAddress {
    guard(ptr::null_mut(), || {
        let s = String::from_utf8_lossy(bytes(t, len));
        let a = address::parse(&s)?;
        let texts = [&a.connection, &a.topic, &a.json_path, &a.ts_path, &a.spb_group, &a.spb_node, &a.spb_device, &a.spb_metric]
            .iter()
            .map(|s| cstring(s))
            .collect();
        Ok(Box::into_raw(Box::new(RsmqttWinccAddress { a, texts })))
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_address_free(a: *mut RsmqttWinccAddress) {
    if !a.is_null() {
        drop(Box::from_raw(a));
    }
}

/// A text field (RSMQTT_WINCC_ADDR_CONNECTION ...); "" for an unknown field.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_address_text(a: *const RsmqttWinccAddress, field: c_int) -> *const c_char {
    match a.as_ref().and_then(|a| a.texts.get(field as usize)) {
        Some(s) => s.as_ptr(),
        None => c"".as_ptr(),
    }
}

/// A number or flag field (RSMQTT_WINCC_ADDR_QOS ...).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_address_int(a: *const RsmqttWinccAddress, field: c_int) -> c_int {
    let Some(a) = a.as_ref() else { return 0 };
    let a = &a.a;
    match field {
        ADDR_QOS => a.qos,
        ADDR_RETAIN => a.retain,
        ADDR_SUB_FLAGS => a.sub_flags,
        ADDR_IS_STATE => a.is_state as c_int,
        ADDR_JSON => a.json as c_int,
        ADDR_HAS_TS => a.has_ts as c_int,
        ADDR_SPARKPLUG => a.sparkplug as c_int,
        ADDR_SPB_ONLINE => a.spb_online as c_int,
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_topic_matches(filter: *const c_char, topic: *const c_char) -> c_int {
    guard(0, || Ok(address::topic_matches(&text(filter), &text(topic)) as c_int))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_is_wildcard_filter(filter: *const c_char) -> c_int {
    guard(0, || Ok(address::is_wildcard_filter(&text(filter)) as c_int))
}

//--------------------------------------------------------------------------- json

pub struct RsmqttWinccExtractor {
    x: json::Extractor,
    results: Vec<json::ExResult>,
    error: CString,
}

#[no_mangle]
pub extern "C" fn rsmqtt_wincc_extractor_new() -> *mut RsmqttWinccExtractor {
    Box::into_raw(Box::new(RsmqttWinccExtractor { x: json::Extractor::new(), results: Vec::new(), error: CString::default() }))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_extractor_free(x: *mut RsmqttWinccExtractor) {
    if !x.is_null() {
        drop(Box::from_raw(x));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_extractor_clear(x: *mut RsmqttWinccExtractor) {
    if let Some(x) = x.as_mut() {
        x.x.clear();
        x.results.clear();
    }
}

/// Adds a path; its id, or -1 if the path is not valid.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_extractor_add(x: *mut RsmqttWinccExtractor, path: *const c_char, len: usize) -> c_int {
    guard(-1, || {
        let x = x.as_mut().ok_or("null extractor")?;
        let p = json::parse_path(&String::from_utf8_lossy(bytes(path, len)))?;
        Ok(x.x.add(&p) as c_int)
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_extractor_size(x: *const RsmqttWinccExtractor) -> c_int {
    x.as_ref().map_or(0, |x| x.x.len() as c_int)
}

/// One pass over `doc` for all paths.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_extractor_run(x: *mut RsmqttWinccExtractor, doc: *const c_char, len: usize) {
    guard((), || {
        let x = x.as_mut().ok_or("null extractor")?;
        let err = x.x.run(bytes(doc, len), &mut x.results);
        x.error = cstring(&err);
        Ok(())
    })
}

/// The result of path `id` of the last run: 0 found, 1 missing, 2 not JSON
/// (-1 for an unknown id). kind / text describe a found value; the text
/// stays valid until the next run.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_extractor_result(
    x: *const RsmqttWinccExtractor,
    id: c_int,
    kind: *mut c_int,
    text: *mut *const c_char,
    len: *mut usize,
) -> c_int {
    let Some(r) = x.as_ref().and_then(|x| x.results.get(id as usize)) else { return -1 };
    if let Some(k) = kind.as_mut() {
        *k = r.value.kind as c_int;
    }
    if let Some(t) = text.as_mut() {
        *t = r.value.text.as_ptr() as *const c_char;
    }
    if let Some(l) = len.as_mut() {
        *l = r.value.text.len();
    }
    r.found as c_int
}

/// What was wrong with the document of the last run ("" if nothing).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_extractor_error(x: *const RsmqttWinccExtractor) -> *const c_char {
    x.as_ref().map_or(c"".as_ptr(), |x| x.error.as_ptr())
}

/// The canonical form of a path for messages ("data.temp", "$").
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_json_path_string(path: *const c_char, len: usize, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || {
        let p = json::parse_path(&String::from_utf8_lossy(bytes(path, len)))?;
        Ok(put(out, json::path_to_string(&p).into_bytes()))
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_json_quote(t: *const c_char, len: usize, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || Ok(put(out, json::quote(bytes(t, len)))))
}

/// Nests n JSON values (JSON text) along their paths into one document.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_json_build(
    n: usize,
    paths: *const *const c_char,
    path_lens: *const usize,
    values: *const *const c_char,
    value_lens: *const usize,
    out: *mut RsmqttWinccBuf,
) -> c_int {
    guard(-1, || {
        if n > 0 && (paths.is_null() || path_lens.is_null() || values.is_null() || value_lens.is_null()) {
            return Err("null argument".into());
        }
        let mut ps = Vec::with_capacity(n);
        for k in 0..n {
            ps.push(json::parse_path(&String::from_utf8_lossy(bytes(*paths.add(k), *path_lens.add(k))))?);
        }
        let members: Vec<(&json::Path, &[u8])> =
            (0..n).map(|k| (&ps[k], bytes(*values.add(k), *value_lens.add(k)))).collect();
        Ok(put(out, json::build(&members)?))
    })
}

/// 1 and the time in ms if the value (kind as in the extractor) is a timestamp.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_json_parse_time(kind: c_int, t: *const c_char, len: usize, ms: *mut i64) -> c_int {
    guard(0, || {
        let k = match kind {
            0 => json::Kind::String,
            1 => json::Kind::Number,
            2 => json::Kind::Bool,
            4 => json::Kind::Object,
            5 => json::Kind::Array,
            _ => json::Kind::Null,
        };
        match json::parse_time(&json::Value { kind: k, text: bytes(t, len).to_vec() }) {
            Some(v) => {
                if let Some(m) = ms.as_mut() {
                    *m = v;
                }
                Ok(1)
            }
            None => Ok(0),
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_json_iso_time(ms: i64, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || Ok(put(out, json::iso_time(ms).into_bytes())))
}

pub struct RsmqttWinccItems {
    items: Vec<(Vec<u8>, bool)>,
}

/// The items of a payload for a dyn DPE (see json::dyn_items); NULL on error.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_dyn_items(payload: *const c_char, len: usize) -> *mut RsmqttWinccItems {
    guard(ptr::null_mut(), || Ok(Box::into_raw(Box::new(RsmqttWinccItems { items: json::dyn_items(bytes(payload, len))? }))))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_items_count(i: *const RsmqttWinccItems) -> usize {
    i.as_ref().map_or(0, |i| i.items.len())
}

/// Item `k`: returns 1 for a string item, 0 for others, -1 if k is out of range.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_items_get(i: *const RsmqttWinccItems, k: usize, t: *mut *const c_char, len: *mut usize) -> c_int {
    let Some((v, is_string)) = i.as_ref().and_then(|i| i.items.get(k)) else { return -1 };
    if let Some(t) = t.as_mut() {
        *t = v.as_ptr() as *const c_char;
    }
    if let Some(l) = len.as_mut() {
        *l = v.len();
    }
    *is_string as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_items_free(i: *mut RsmqttWinccItems) {
    if !i.is_null() {
        drop(Box::from_raw(i));
    }
}

//--------------------------------------------------------------------------- redu

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_redu_for_host(base: *const c_char, host2: c_int, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || Ok(put(out, redu::for_host(&text(base), host2 != 0).into_bytes())))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_redu_ends_with_suffix(name: *const c_char) -> c_int {
    guard(0, || Ok(redu::ends_with_suffix(&text(name)) as c_int))
}

/// 1 and the option name if `name` is "redu.<option>", else 0.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_redu_split_option(name: *const c_char, option: *mut RsmqttWinccBuf) -> c_int {
    guard(0, || {
        let n = text(name);
        match redu::split_redu_option(&n) {
            Some(o) => {
                put(option, o.as_bytes().to_vec());
                Ok(1)
            }
            None => Ok(0),
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_redu_client_id(
    client_id: *const c_char,
    host2: c_int,
    redu_given: c_int,
    out: *mut RsmqttWinccBuf,
) -> c_int {
    guard(-1, || Ok(put(out, redu::client_id_for_host(&text(client_id), host2 != 0, redu_given != 0).into_bytes())))
}

//---------------------------------------------------------------------- sparkplug

/// One update of rsmqtt_wincc_spb_update_get (pointers valid until the next
/// on_message / connection_lost of that host).
#[repr(C)]
pub struct RsmqttWinccSpbUpdate {
    pub kind: c_int,
    pub group: *const c_char,
    pub node: *const c_char,
    pub device: *const c_char,
    pub metric: *const c_char,
    pub text: *const c_char,
    pub text_len: usize,
    pub is_string: c_int,
    pub online: c_int,
    pub has_time: c_int,
    pub time_ms: i64,
}

struct CUpdate {
    u: sparkplug::Update,
    group: CString,
    node: CString,
    device: CString,
    metric: CString,
}

pub struct RsmqttWinccSpbHost {
    host: sparkplug::Host,
    updates: Vec<CUpdate>,
    publish: Vec<(CString, Vec<u8>)>,
    warnings: Vec<CString>,
}

impl RsmqttWinccSpbHost {
    /// Before each call: a call that fails (panic) must not leave the
    /// results of the previous one to be applied again.
    fn forget(&mut self) {
        self.updates.clear();
        self.publish.clear();
        self.warnings.clear();
    }

    fn keep(&mut self, o: sparkplug::Outcome) {
        self.updates = o
            .updates
            .into_iter()
            .map(|u| CUpdate { group: cstring(&u.group), node: cstring(&u.node), device: cstring(&u.device), metric: cstring(&u.metric), u })
            .collect();
        self.publish = o.publish.into_iter().map(|m| (cstring(&m.topic), m.payload)).collect();
        self.warnings = o.warnings.iter().map(|w| cstring(w)).collect();
    }
}

#[no_mangle]
pub extern "C" fn rsmqtt_wincc_spb_host_new() -> *mut RsmqttWinccSpbHost {
    Box::into_raw(Box::new(RsmqttWinccSpbHost { host: sparkplug::Host::new(), updates: vec![], publish: vec![], warnings: vec![] }))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_host_free(h: *mut RsmqttWinccSpbHost) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_set_rebirth_interval(h: *mut RsmqttWinccSpbHost, ms: i64) {
    if let Some(h) = h.as_mut() {
        h.host.rebirth_interval_ms = ms;
    }
}

/// Handles one message; the updates, rebirth requests and warnings are
/// then available through the _count / _get functions.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_on_message(
    h: *mut RsmqttWinccSpbHost,
    topic: *const c_char,
    payload: *const c_char,
    len: usize,
    now_ms: i64,
) {
    guard((), || {
        let h = h.as_mut().ok_or("null host")?;
        h.forget();
        let o = h.host.on_message(&text(topic), bytes(payload, len), now_ms);
        h.keep(o);
        Ok(())
    })
}

/// The broker connection is gone: every edge node and device goes stale.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_connection_lost(h: *mut RsmqttWinccSpbHost) {
    guard((), || {
        let h = h.as_mut().ok_or("null host")?;
        h.forget();
        let o = h.host.on_connection_lost();
        h.keep(o);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_update_count(h: *const RsmqttWinccSpbHost) -> usize {
    h.as_ref().map_or(0, |h| h.updates.len())
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_update_get(h: *const RsmqttWinccSpbHost, i: usize, out: *mut RsmqttWinccSpbUpdate) -> c_int {
    let (Some(h), Some(out)) = (h.as_ref(), out.as_mut()) else { return -1 };
    let Some(c) = h.updates.get(i) else { return -1 };
    *out = RsmqttWinccSpbUpdate {
        kind: c.u.kind as c_int,
        group: c.group.as_ptr(),
        node: c.node.as_ptr(),
        device: c.device.as_ptr(),
        metric: c.metric.as_ptr(),
        text: c.u.text.as_ptr() as *const c_char,
        text_len: c.u.text.len(),
        is_string: c.u.is_string as c_int,
        online: c.u.online as c_int,
        has_time: c.u.time_ms.is_some() as c_int,
        time_ms: c.u.time_ms.unwrap_or(0),
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_publish_count(h: *const RsmqttWinccSpbHost) -> usize {
    h.as_ref().map_or(0, |h| h.publish.len())
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_publish_get(
    h: *const RsmqttWinccSpbHost,
    i: usize,
    topic: *mut *const c_char,
    payload: *mut *const c_char,
    len: *mut usize,
) -> c_int {
    let Some((t, p)) = h.as_ref().and_then(|h| h.publish.get(i)) else { return -1 };
    if let Some(x) = topic.as_mut() {
        *x = t.as_ptr();
    }
    if let Some(x) = payload.as_mut() {
        *x = p.as_ptr() as *const c_char;
    }
    if let Some(x) = len.as_mut() {
        *x = p.len();
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_warning_count(h: *const RsmqttWinccSpbHost) -> usize {
    h.as_ref().map_or(0, |h| h.warnings.len())
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_warning_get(h: *const RsmqttWinccSpbHost, i: usize) -> *const c_char {
    h.as_ref().and_then(|h| h.warnings.get(i)).map_or(c"".as_ptr(), |w| w.as_ptr())
}

/// NCMD (device "" ) or DCMD that writes `text` to a metric; 0, or -1 and
/// rsmqtt_wincc_last_error().
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_command(
    h: *const RsmqttWinccSpbHost,
    group: *const c_char,
    node: *const c_char,
    device: *const c_char,
    metric: *const c_char,
    value: *const c_char,
    len: usize,
    now_ms: i64,
    topic: *mut RsmqttWinccBuf,
    payload: *mut RsmqttWinccBuf,
) -> c_int {
    guard(-1, || {
        let h = h.as_ref().ok_or("null host")?;
        let m = h.host.command(&text(group), &text(node), &text(device), &text(metric), bytes(value, len), now_ms)?;
        put(topic, m.topic.into_bytes());
        Ok(put(payload, m.payload))
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_is_online(
    h: *const RsmqttWinccSpbHost,
    group: *const c_char,
    node: *const c_char,
    device: *const c_char,
) -> c_int {
    h.as_ref().map_or(0, |h| h.host.is_online(&text(group), &text(node), &text(device)) as c_int)
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_data_type(
    h: *const RsmqttWinccSpbHost,
    group: *const c_char,
    node: *const c_char,
    device: *const c_char,
    metric: *const c_char,
) -> u32 {
    h.as_ref().map_or(0, |h| h.host.data_type(&text(group), &text(node), &text(device), &text(metric)))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_node_filter(group: *const c_char, node: *const c_char, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || Ok(put(out, sparkplug::node_filter(&text(group), &text(node)).into_bytes())))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_state_topic(host_id: *const c_char, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || Ok(put(out, sparkplug::state_topic(&text(host_id)).into_bytes())))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_state_payload(online: c_int, timestamp_ms: i64, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || Ok(put(out, sparkplug::state_payload(online != 0, timestamp_ms).into_bytes())))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_spb_valid_id(id: *const c_char) -> c_int {
    guard(0, || Ok(sparkplug::valid_id(&text(id)) as c_int))
}
