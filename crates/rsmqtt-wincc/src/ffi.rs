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
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use crate::{address, json, redu, router, settings, sparkplug, values};

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

fn owned_cstring(s: String) -> CString {
    CString::new(s).unwrap_or_else(|e| cstring(&String::from_utf8_lossy(&e.into_vec())))
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

//----------------------------------------------------------------------- settings

/// Text as (pointer, length), not NUL-terminated.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RsmqttWinccStr {
    pub data: *const c_char,
    pub len: usize,
}

/// The `Config.*` elements of a `_RsMqttBroker` DP.
#[repr(C)]
pub struct RsmqttWinccBrokerConfig {
    pub server_uri: RsmqttWinccStr,
    pub server_uris: *const RsmqttWinccStr,
    pub server_uris_len: usize,
    pub client_id: RsmqttWinccStr,
    pub mqtt_version: c_int,
    pub keep_alive: c_int,
    pub clean_session: c_int,
    pub username: RsmqttWinccStr,
    pub password: RsmqttWinccStr,
    pub tls_trust_store: RsmqttWinccStr,
    pub tls_key_store: RsmqttWinccStr,
    pub tls_private_key: RsmqttWinccStr,
    pub tls_verify: c_int,
    pub will_topic: RsmqttWinccStr,
    pub will_payload: RsmqttWinccStr,
    pub will_qos: c_int,
    pub will_retain: c_int,
    pub subscribe_qos: c_int,
    pub publish_qos: c_int,
    pub publish_retain: c_int,
    pub options: *const RsmqttWinccStr,
    pub options_len: usize,
}

/// Decrypts a stored password: 0 and `*out` = the plain text, or -1 and
/// `*out` = why it failed (allocate `*out` with rsmqtt_wincc_buf_new).
pub type RsmqttWinccRevealFn =
    Option<unsafe extern "C" fn(ctx: *mut c_void, stored: *const c_char, len: usize, out: *mut RsmqttWinccBuf) -> c_int>;

pub type RsmqttWinccSettings = settings::Settings;

pub const SETTINGS_SUBSCRIBE_QOS: c_int = 0;
pub const SETTINGS_PUBLISH_QOS: c_int = 1;
pub const SETTINGS_PUBLISH_RETAIN: c_int = 2;
pub const SETTINGS_RECONNECT_INTERVAL_MS: c_int = 3;
pub const SETTINGS_AUTO_RECONNECT: c_int = 4;
pub const SETTINGS_STATE_TS: c_int = 5;

/// A buffer with a copy of `data` (for callbacks that return a buffer).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_buf_new(data: *const c_char, len: usize) -> RsmqttWinccBuf {
    buf(bytes(data, len).to_vec())
}

unsafe fn take_buf(b: RsmqttWinccBuf) -> Vec<u8> {
    if b.data.is_null() {
        return Vec::new();
    }
    Box::from_raw(ptr::slice_from_raw_parts_mut(b.data as *mut u8, b.len)).into_vec()
}

unsafe fn utf8(s: RsmqttWinccStr, what: &str) -> Result<String, String> {
    String::from_utf8(bytes(s.data, s.len).to_vec()).map_err(|_| format!("{what} is not valid UTF-8"))
}

unsafe fn utf8_list(p: *const RsmqttWinccStr, n: usize, what: &str) -> Result<Vec<String>, String> {
    if n == 0 {
        return Ok(Vec::new());
    }
    if p.is_null() {
        return Err(format!("{what}: null list"));
    }
    std::slice::from_raw_parts(p, n).iter().map(|s| utf8(*s, what)).collect()
}

unsafe fn broker_config(c: &RsmqttWinccBrokerConfig) -> Result<settings::BrokerConfig, String> {
    Ok(settings::BrokerConfig {
        server_uri: utf8(c.server_uri, "Config.ServerURI")?,
        server_uris: utf8_list(c.server_uris, c.server_uris_len, "Config.ServerURIs")?,
        client_id: utf8(c.client_id, "Config.ClientId")?,
        mqtt_version: c.mqtt_version,
        keep_alive: c.keep_alive,
        clean_session: c.clean_session != 0,
        username: utf8(c.username, "Config.Username")?,
        password: utf8(c.password, "Config.Password")?,
        tls_trust_store: utf8(c.tls_trust_store, "Config.TlsTrustStore")?,
        tls_key_store: utf8(c.tls_key_store, "Config.TlsKeyStore")?,
        tls_private_key: utf8(c.tls_private_key, "Config.TlsPrivateKey")?,
        tls_verify: c.tls_verify != 0,
        will_topic: utf8(c.will_topic, "Config.WillTopic")?,
        will_payload: utf8(c.will_payload, "Config.WillPayload")?,
        will_qos: c.will_qos,
        will_retain: c.will_retain != 0,
        subscribe_qos: c.subscribe_qos,
        publish_qos: c.publish_qos,
        publish_retain: c.publish_retain != 0,
        options: utf8_list(c.options, c.options_len, "Config.Options")?,
    })
}

/// The driver's default settings of a connection.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_new(name: *const c_char, man_num: c_int) -> *mut RsmqttWinccSettings {
    guard(ptr::null_mut(), || Ok(Box::into_raw(Box::new(settings::Settings::new(&text(name), man_num)))))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_free(s: *mut RsmqttWinccSettings) {
    if !s.is_null() {
        drop(Box::from_raw(s));
    }
}

/// A number or flag (RSMQTT_WINCC_SETTINGS_SUBSCRIBE_QOS ...); 0 for an unknown field.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_get(s: *const RsmqttWinccSettings, field: c_int) -> i64 {
    let Some(s) = s.as_ref() else { return 0 };
    match field {
        SETTINGS_SUBSCRIBE_QOS => s.subscribe_qos as i64,
        SETTINGS_PUBLISH_QOS => s.publish_qos as i64,
        SETTINGS_PUBLISH_RETAIN => s.publish_retain as i64,
        SETTINGS_RECONNECT_INTERVAL_MS => s.reconnect_interval_ms,
        SETTINGS_AUTO_RECONNECT => s.auto_reconnect() as i64,
        SETTINGS_STATE_TS => s.state_ts(),
        _ => 0,
    }
}

/// Sets SUBSCRIBE_QOS, PUBLISH_QOS, PUBLISH_RETAIN or RECONNECT_INTERVAL_MS
/// (config file); -1 for other fields.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_set(s: *mut RsmqttWinccSettings, field: c_int, value: i64) -> c_int {
    guard(-1, || {
        let s = s.as_mut().ok_or("null settings")?;
        match field {
            SETTINGS_SUBSCRIBE_QOS => s.subscribe_qos = value as i32,
            SETTINGS_PUBLISH_QOS => s.publish_qos = value as i32,
            SETTINGS_PUBLISH_RETAIN => s.publish_retain = value != 0,
            SETTINGS_RECONNECT_INTERVAL_MS => s.reconnect_interval_ms = value,
            _ => return Err(format!("settings field {field} cannot be set")),
        }
        Ok(0)
    })
}

/// Sets an rsmqtt option (config file).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_set_option(
    s: *mut RsmqttWinccSettings,
    name: *const c_char,
    value: *const c_char,
    len: usize,
) -> c_int {
    guard(-1, || {
        let s = s.as_mut().ok_or("null settings")?;
        let v = utf8(RsmqttWinccStr { data: value, len }, "option value")?;
        s.set_option(&text(name), &v)?;
        Ok(0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_set_spb_host_id(s: *mut RsmqttWinccSettings, id: *const c_char) -> c_int {
    guard(-1, || {
        s.as_mut().ok_or("null settings")?.set_spb_host_id(&text(id))?;
        Ok(0)
    })
}

/// A "redu.<option>" entry (option without "redu."): set on host 2, checked on host 1.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_apply_redu_option(
    s: *mut RsmqttWinccSettings,
    option: *const c_char,
    value: *const c_char,
    len: usize,
    host2: c_int,
) -> c_int {
    guard(-1, || {
        let s = s.as_mut().ok_or("null settings")?;
        let v = utf8(RsmqttWinccStr { data: value, len }, "option value")?;
        s.apply_redu_option(&text(option), &v, host2 != 0)?;
        Ok(0)
    })
}

/// Builds the settings from a broker DP; on failure (-1) they stay as they were.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_apply_broker_config(
    s: *mut RsmqttWinccSettings,
    cfg: *const RsmqttWinccBrokerConfig,
    host2: c_int,
    reveal: RsmqttWinccRevealFn,
    ctx: *mut c_void,
) -> c_int {
    guard(-1, || {
        let s = s.as_mut().ok_or("null settings")?;
        let cfg = broker_config(cfg.as_ref().ok_or("null config")?)?;
        let mut call = |stored: &str| -> Result<String, String> {
            let Some(f) = reveal else { return Ok(stored.to_string()) };
            let mut out = RsmqttWinccBuf { data: ptr::null_mut(), len: 0 };
            let rc = f(ctx, stored.as_ptr() as *const c_char, stored.len(), &mut out);
            let v = take_buf(out);
            if rc == 0 {
                String::from_utf8(v).map_err(|_| "the decrypted password is not valid UTF-8".to_string())
            } else {
                Err(String::from_utf8_lossy(&v).into_owned())
            }
        };
        s.apply_broker_config(&cfg, host2 != 0, &mut call)?;
        Ok(0)
    })
}

/// The Sparkplug host id this driver publishes STATE for ("" for none).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_spb_state_host(
    s: *const RsmqttWinccSettings,
    host2: c_int,
    out: *mut RsmqttWinccBuf,
) -> c_int {
    guard(-1, || Ok(put(out, s.as_ref().ok_or("null settings")?.spb_state_host(host2 != 0).as_bytes().to_vec())))
}

/// The options to create the client with (an rsmqtt_options handle, free it
/// with rsmqtt_options_free); `warnings` gets the warnings for the log, one
/// per line (may be empty).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_settings_client_options(
    s: *mut RsmqttWinccSettings,
    host2: c_int,
    data_dir: *const c_char,
    now_ms: i64,
    warnings: *mut RsmqttWinccBuf,
) -> *mut rsmqtt::RsmqttOptions {
    guard(ptr::null_mut(), || {
        let s = s.as_mut().ok_or("null settings")?;
        let (o, w) = s.client_options(host2 != 0, &text(data_dir), now_ms);
        if !warnings.is_null() {
            put(warnings, w.join("\n").into_bytes());
        }
        Ok(Box::into_raw(Box::new(o)))
    })
}

/// 1 if the value of this option may be encrypted (password).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_is_secret_option(name: *const c_char) -> c_int {
    guard(0, || Ok(settings::is_secret_option(&text(name)) as c_int))
}

//------------------------------------------------------------------------- router

/// One value of rsmqtt_wincc_router_delivery_get (pointers valid until the
/// next call that changes the router).
#[repr(C)]
pub struct RsmqttWinccDelivery {
    pub address: *const c_char,
    /// 0: the payload of the message as it is
    pub has_value: c_int,
    pub value: *const c_char,
    pub value_len: usize,
    /// 0: now
    pub has_time: c_int,
    pub time_ms: i64,
    pub periph_time: c_int,
    /// 0 keep, 1 valid, 2 invalid
    pub validity: c_int,
}

#[repr(C)]
pub struct RsmqttWinccPublish {
    pub topic: *const c_char,
    pub payload: *const c_char,
    pub payload_len: usize,
    pub qos: c_int,
    pub retain: c_int,
}

pub struct RsmqttWinccRouter {
    r: router::Router,
    deliveries: Vec<(CString, router::Delivery)>,
    publish: Vec<(CString, router::Publish)>,
    subscribe: Vec<(CString, c_int)>,
    warnings: Vec<(CString, bool)>,
}

impl RsmqttWinccRouter {
    fn keep(&mut self, o: router::Outcome) {
        // the strings move into the CStrings (no copies: one message may have many deliveries)
        self.deliveries = o.deliveries.into_iter().map(|mut d| (owned_cstring(std::mem::take(&mut d.address)), d)).collect();
        self.publish = o.publish.into_iter().map(|mut p| (owned_cstring(std::mem::take(&mut p.topic)), p)).collect();
        self.subscribe = o.subscribe.into_iter().map(|(t, q)| (owned_cstring(t), q)).collect();
        self.warnings = o.warnings.into_iter().map(|w| (owned_cstring(w.text), w.sparkplug)).collect();
    }

    /// Before each call: a call that fails (panic) must not leave the
    /// results of the previous one to be applied again.
    fn forget(&mut self) {
        self.deliveries.clear();
        self.publish.clear();
        self.subscribe.clear();
        self.warnings.clear();
    }
}

unsafe fn router_call<F: FnOnce(&mut router::Router) -> router::Outcome>(r: *mut RsmqttWinccRouter, f: F) {
    guard((), || {
        let r = r.as_mut().ok_or("null router")?;
        r.forget();
        let o = f(&mut r.r);
        r.keep(o);
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rsmqtt_wincc_router_new() -> *mut RsmqttWinccRouter {
    Box::into_raw(Box::new(RsmqttWinccRouter {
        r: router::Router::new(),
        deliveries: vec![],
        publish: vec![],
        subscribe: vec![],
        warnings: vec![],
    }))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_free(r: *mut RsmqttWinccRouter) {
    if !r.is_null() {
        drop(Box::from_raw(r));
    }
}

/// Registers an address: 0 nothing to do, 1 a $state address (write the
/// state), 2 subscribe `*filter`; -1 and rsmqtt_wincc_last_error() if the
/// address is not valid.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_add(
    r: *mut RsmqttWinccRouter,
    full: *const c_char,
    is_dyn: c_int,
    input: c_int,
    default_qos: c_int,
    filter: *mut RsmqttWinccBuf,
) -> c_int {
    guard(-1, || {
        let r = r.as_mut().ok_or("null router")?;
        Ok(match r.r.add(&text(full), is_dyn != 0, input != 0, default_qos)? {
            router::Added::Nothing => 0,
            router::Added::State => 1,
            router::Added::Subscribe(f) => {
                put(filter, f.into_bytes());
                2
            }
        })
    })
}

/// Unregisters an address: 1 and `*filter` to unsubscribe, else 0.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_remove(
    r: *mut RsmqttWinccRouter,
    full: *const c_char,
    input: c_int,
    filter: *mut RsmqttWinccBuf,
) -> c_int {
    guard(0, || {
        let r = r.as_mut().ok_or("null router")?;
        Ok(match r.r.remove(&text(full), input != 0) {
            Some(f) => {
                put(filter, f.into_bytes());
                1
            }
            None => 0,
        })
    })
}

/// 1 and QoS / flags of a subscription, 0 if there is none for this filter.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_filter(
    r: *const RsmqttWinccRouter,
    filter: *const c_char,
    qos: *mut c_int,
    flags: *mut c_int,
) -> c_int {
    guard(0, || {
        let r = r.as_ref().ok_or("null router")?;
        let Some(f) = r.r.filter(&text(filter)) else { return Ok(0) };
        if let Some(q) = qos.as_mut() {
            *q = f.qos;
        }
        if let Some(x) = flags.as_mut() {
            *x = f.flags;
        }
        Ok(1)
    })
}

/// All subscription filters, each followed by a NUL byte.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_filters(r: *const RsmqttWinccRouter, out: *mut RsmqttWinccBuf) -> c_int {
    guard(-1, || {
        let r = r.as_ref().ok_or("null router")?;
        let mut v = Vec::new();
        for f in r.r.filters() {
            v.extend_from_slice(f.as_bytes());
            v.push(0);
        }
        Ok(put(out, v))
    })
}

/// 1 if there are addresses that need a broker (subscriptions or $state).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_has_addresses(r: *const RsmqttWinccRouter) -> c_int {
    r.as_ref().map_or(0, |r| r.r.has_addresses() as c_int)
}

/// One message (results: rsmqtt_wincc_router_*_count / _get).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_message(
    r: *mut RsmqttWinccRouter,
    topic: *const c_char,
    payload: *const c_char,
    len: usize,
    now_ms: i64,
    state_host: *const c_char,
    state_ts: i64,
) {
    let (t, h) = (text(topic), text(state_host));
    router_call(r, |r| r.message(&t, bytes(payload, len), now_ms, &h, state_ts))
}

/// The values of the $state addresses.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_state(r: *mut RsmqttWinccRouter, connected: c_int) {
    router_call(r, |r| r.state(connected != 0))
}

/// The connection is established (Sparkplug STATE, edge nodes offline until their birth).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_connected(r: *mut RsmqttWinccRouter, state_host: *const c_char, state_ts: i64) {
    let h = text(state_host);
    router_call(r, |r| r.connected(&h, state_ts))
}

/// The connection is gone: Sparkplug edge nodes offline, metrics stale.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_connection_lost(r: *mut RsmqttWinccRouter) {
    router_call(r, |r| r.connection_lost())
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_delivery_count(r: *const RsmqttWinccRouter) -> usize {
    r.as_ref().map_or(0, |r| r.deliveries.len())
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_delivery_get(r: *const RsmqttWinccRouter, i: usize, out: *mut RsmqttWinccDelivery) -> c_int {
    let (Some(r), Some(out)) = (r.as_ref(), out.as_mut()) else { return -1 };
    let Some((a, d)) = r.deliveries.get(i) else { return -1 };
    let (has_value, value, value_len) = match &d.value {
        Some(v) => (1, v.as_ptr() as *const c_char, v.len()),
        None => (0, c"".as_ptr(), 0),
    };
    *out = RsmqttWinccDelivery {
        address: a.as_ptr(),
        has_value,
        value,
        value_len,
        has_time: d.time_ms.is_some() as c_int,
        time_ms: d.time_ms.unwrap_or(0),
        periph_time: d.periph_time as c_int,
        validity: d.validity as c_int,
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_publish_count(r: *const RsmqttWinccRouter) -> usize {
    r.as_ref().map_or(0, |r| r.publish.len())
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_publish_get(r: *const RsmqttWinccRouter, i: usize, out: *mut RsmqttWinccPublish) -> c_int {
    let (Some(r), Some(out)) = (r.as_ref(), out.as_mut()) else { return -1 };
    let Some((t, p)) = r.publish.get(i) else { return -1 };
    *out = RsmqttWinccPublish {
        topic: t.as_ptr(),
        payload: p.payload.as_ptr() as *const c_char,
        payload_len: p.payload.len(),
        qos: p.qos,
        retain: p.retain as c_int,
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_subscribe_count(r: *const RsmqttWinccRouter) -> usize {
    r.as_ref().map_or(0, |r| r.subscribe.len())
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_subscribe_get(
    r: *const RsmqttWinccRouter,
    i: usize,
    filter: *mut *const c_char,
    qos: *mut c_int,
) -> c_int {
    let Some((f, q)) = r.as_ref().and_then(|r| r.subscribe.get(i)) else { return -1 };
    if let Some(x) = filter.as_mut() {
        *x = f.as_ptr();
    }
    if let Some(x) = qos.as_mut() {
        *x = *q;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_warning_count(r: *const RsmqttWinccRouter) -> usize {
    r.as_ref().map_or(0, |r| r.warnings.len())
}

/// Warning i ("" if out of range); `*sparkplug` = 1 for one of the Sparkplug host application.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_warning_get(r: *const RsmqttWinccRouter, i: usize, sparkplug: *mut c_int) -> *const c_char {
    let Some((w, s)) = r.as_ref().and_then(|r| r.warnings.get(i)) else { return c"".as_ptr() };
    if let Some(x) = sparkplug.as_mut() {
        *x = *s as c_int;
    }
    w.as_ptr()
}

/// The message that writes `value` (payload text of data type `value_type`)
/// to an output address: 0 and topic / payload / QoS / retain, or -1 and
/// rsmqtt_wincc_last_error() (the message for the log).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_router_write(
    r: *const RsmqttWinccRouter,
    full: *const c_char,
    value: *const c_char,
    len: usize,
    value_type: c_int,
    time_ms: i64,
    now_ms: i64,
    publish_qos: c_int,
    publish_retain: c_int,
    topic: *mut RsmqttWinccBuf,
    payload: *mut RsmqttWinccBuf,
    qos: *mut c_int,
    retain: *mut c_int,
) -> c_int {
    guard(-1, || {
        let r = r.as_ref().ok_or("null router")?;
        let p = r.r.write(&text(full), bytes(value, len), value_type, time_ms, now_ms, publish_qos, publish_retain != 0)?;
        if let Some(q) = qos.as_mut() {
            *q = p.qos;
        }
        if let Some(x) = retain.as_mut() {
            *x = p.retain as c_int;
        }
        put(topic, p.topic.into_bytes());
        Ok(put(payload, p.payload))
    })
}

//------------------------------------------------------------------------- values

/// A number or boolean of rsmqtt_wincc_parse_number: the field of its type is set.
#[repr(C)]
pub struct RsmqttWinccNumber {
    pub i: i64,
    pub u: u64,
    pub f: f64,
    pub b: c_int,
}

/// A value of data type `type` (1002 int32 .. 1007 bool) from payload text;
/// `json_string`: the text came from a JSON string. 0, or -1 and
/// rsmqtt_wincc_last_error() (why it cannot be converted).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wincc_parse_number(
    value_type: c_int,
    text: *const c_char,
    len: usize,
    json_string: c_int,
    out: *mut RsmqttWinccNumber,
) -> c_int {
    guard(-1, || {
        let out = out.as_mut().ok_or("null output")?;
        *out = RsmqttWinccNumber { i: 0, u: 0, f: 0.0, b: 0 };
        match values::parse_number(value_type, bytes(text, len), json_string != 0)? {
            values::Number::Int(v) => out.i = v,
            values::Number::UInt(v) => out.u = v,
            values::Number::Float(v) => out.f = v,
            values::Number::Bool(v) => out.b = v as c_int,
        }
        Ok(0)
    })
}
