//! # rsmqtt — a C ABI MQTT client
//!
//! A small, stable C interface over the pure-Rust Eclipse Paho MQTT client
//! (`paho-mqtt` 2.x). It supports what Paho C's `MQTTAsync` supports:
//! MQTT 3.1 / 3.1.1 / 5, QoS 0–2, TCP, TLS, WebSocket (ws/wss), HTTP(S)
//! proxies, Last Will, automatic reconnect, offline buffering, several
//! server URIs, file/memory persistence and the full set of v5 properties
//! and reason codes.
//!
//! The API is *poll based*: results and incoming messages are queued and
//! fetched by the host with [`rsmqtt_poll_event`]. Nothing calls back into
//! the host from library threads except the optional notify and trace
//! callbacks, which makes it easy to embed in single-threaded hosts such as
//! WinCC OA managers and drivers.
//!
//! See `include/rsmqtt.h` for the documentation of each function.

#![allow(clippy::missing_safety_doc)]

pub mod client;
pub mod error;
pub mod options;
mod trace;

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::OnceLock;
use std::time::Duration;

use paho_mqtt as mqtt;
use mqtt::{Properties, Property, QoS};

use crate::client::{Client, Event, NotifyFn};
use crate::error::*;
use crate::options::{OptionsData, PROPS_CONNECT, PROPS_WILL};

pub use crate::client::{EventKind, OpKind};

/// Opaque handle types as seen from C.
pub type RsmqttOptions = OptionsData;
pub type RsmqttProperties = Properties;
pub type RsmqttClient = Client;
pub type RsmqttToken = u64;

/// An event handed out to C, with NUL-terminated copies of its strings.
pub struct RsmqttEvent {
    ev: Event,
    topic: CString,
    text: CString,
    server_uri: CString,
}

/// One property, as returned by `rsmqtt_properties_get`.
#[repr(C)]
pub struct RsmqttProperty {
    /// MQTT v5 property identifier (e.g. 0x26 = user property).
    pub id: c_int,
    /// Value of integer properties (byte, two/four byte, variable integer).
    pub int_value: u32,
    /// String / binary value, or the value of a user property.
    /// NOT NUL-terminated; use `data_len`.
    pub data: *const c_char,
    pub data_len: usize,
    /// Name of a user property (NOT NUL-terminated), otherwise NULL.
    pub name: *const c_char,
    pub name_len: usize,
}

// ---------------------------------------------------------------------------
// Helpers

/// Runs `f`, converting panics and errors into return codes and setting the
/// thread's last error text.
fn guard<F>(f: F) -> c_int
where
    F: FnOnce() -> FfiResult<()>,
{
    clear_last_error();
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => RSMQTT_SUCCESS,
        Ok(Err(e)) => {
            set_last_error(&e.message);
            e.rc
        }
        Err(_) => {
            set_last_error("internal error (panic) in rsmqtt");
            RSMQTT_PANIC
        }
    }
}

/// Like `guard` for functions returning a value; `dflt` on panic.
fn guard_val<T, F: FnOnce() -> T>(dflt: T, f: F) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(dflt)
}

unsafe fn cstr<'a>(p: *const c_char, what: &str) -> FfiResult<&'a str> {
    if p.is_null() {
        return Err(FfiError::new(RSMQTT_NULL_PARAMETER, format!("{what} is NULL")));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|_| FfiError::new(RSMQTT_BAD_UTF8_STRING, format!("{what} is not valid UTF-8")))
}

unsafe fn bytes<'a>(p: *const c_void, len: usize) -> FfiResult<&'a [u8]> {
    if len == 0 {
        Ok(&[])
    } else if p.is_null() {
        Err(FfiError::new(RSMQTT_NULL_PARAMETER, "data is NULL"))
    } else {
        Ok(std::slice::from_raw_parts(p as *const u8, len))
    }
}

unsafe fn obj<'a, T>(p: *const T, what: &str) -> FfiResult<&'a T> {
    p.as_ref()
        .ok_or_else(|| FfiError::new(RSMQTT_NULL_PARAMETER, format!("{what} is NULL")))
}

unsafe fn obj_mut<'a, T>(p: *mut T, what: &str) -> FfiResult<&'a mut T> {
    p.as_mut()
        .ok_or_else(|| FfiError::new(RSMQTT_NULL_PARAMETER, format!("{what} is NULL")))
}

unsafe fn opt_props(p: *const RsmqttProperties) -> Option<Properties> {
    p.as_ref().filter(|p| !p.is_empty()).cloned()
}

unsafe fn put_token(out: *mut RsmqttToken, tok: u64) {
    if !out.is_null() {
        *out = tok;
    }
}

fn qos(q: c_int) -> FfiResult<QoS> {
    u8::try_from(q)
        .ok()
        .and_then(|q| QoS::try_from(q).ok())
        .ok_or_else(|| FfiError::new(RSMQTT_BAD_QOS, format!("invalid QoS {q}")))
}

fn ms(timeout_ms: c_int) -> Duration {
    Duration::from_millis(timeout_ms.max(0) as u64)
}

/// Subscribe option flags use the bit positions of the MQTT v5 wire format.
pub const RSMQTT_SUB_NO_LOCAL: c_int = 0x04;
pub const RSMQTT_SUB_RETAIN_AS_PUBLISHED: c_int = 0x08;
pub const RSMQTT_SUB_RETAIN_SEND_ON_NEW: c_int = 0x10;
pub const RSMQTT_SUB_RETAIN_DONT_SEND: c_int = 0x20;

fn sub_options(flags: c_int) -> FfiResult<mqtt::SubscribeOptions> {
    let handling = match (flags >> 4) & 0x03 {
        0 => mqtt::RetainHandling::SendRetainedOnSubscribe,
        1 => mqtt::RetainHandling::SendRetainedOnNew,
        2 => mqtt::RetainHandling::DontSendRetained,
        _ => {
            return Err(FfiError::new(
                RSMQTT_INVALID_ARGUMENT,
                "invalid retain handling in subscribe flags",
            ))
        }
    };
    Ok(mqtt::SubscribeOptions::new(
        flags & RSMQTT_SUB_NO_LOCAL != 0,
        flags & RSMQTT_SUB_RETAIN_AS_PUBLISHED != 0,
        handling,
    ))
}

// ---------------------------------------------------------------------------
// Library information

/// Version of this library, e.g. "rsmqtt 0.1.0 (paho-mqtt 2.0.0-pre.1)".
#[no_mangle]
pub extern "C" fn rsmqtt_version() -> *const c_char {
    static V: OnceLock<CString> = OnceLock::new();
    V.get_or_init(|| {
        CString::new(format!(
            "rsmqtt {} (Eclipse Paho Rust)",
            env!("CARGO_PKG_VERSION")
        ))
        .unwrap()
    })
    .as_ptr()
}

#[no_mangle]
pub extern "C" fn rsmqtt_strerror(rc: c_int) -> *const c_char {
    static TABLE: OnceLock<std::collections::HashMap<i32, CString>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        (-200..=0)
            .map(|rc| (rc, CString::new(strerror(rc)).unwrap()))
            .collect()
    });
    t.get(&rc)
        .or_else(|| t.get(&-200))
        .map(|c| c.as_ptr())
        .unwrap_or(ptr::null())
}

/// Text for an MQTT reason code (0..255).
#[no_mangle]
pub extern "C" fn rsmqtt_reason_code_string(reason_code: c_int) -> *const c_char {
    static TABLE: OnceLock<Vec<CString>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        (0..=255u8)
            .map(|rc| {
                let s = match mqtt::ReasonCode::try_from(rc) {
                    Ok(r) => r.to_string(),
                    Err(_) => format!("Unknown reason code 0x{rc:02X}"),
                };
                CString::new(s).unwrap_or_default()
            })
            .collect()
    });
    match usize::try_from(reason_code) {
        Ok(i) if i < t.len() => t[i].as_ptr(),
        _ => c"Invalid reason code".as_ptr(),
    }
}

/// The last error text of the calling thread ("" if none).
#[no_mangle]
pub extern "C" fn rsmqtt_last_error() -> *const c_char {
    last_error_ptr()
}

/// Installs a global trace callback (Paho's `log` output). `level`:
/// 1=error 2=warn 3=info 4=debug 5=trace; 0 or a NULL callback turns it off.
/// The callback is called from library threads.
#[no_mangle]
pub extern "C" fn rsmqtt_set_trace_callback(
    level: c_int,
    cb: Option<trace::TraceFn>,
    ctx: *mut c_void,
) {
    guard_val((), || trace::set(level, cb, ctx));
}

// ---------------------------------------------------------------------------
// Options

#[no_mangle]
pub extern "C" fn rsmqtt_options_new() -> *mut RsmqttOptions {
    guard_val(ptr::null_mut(), || Box::into_raw(Box::default()))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_options_clone(opts: *const RsmqttOptions) -> *mut RsmqttOptions {
    guard_val(ptr::null_mut(), || match opts.as_ref() {
        Some(o) => Box::into_raw(Box::new(o.clone())),
        None => ptr::null_mut(),
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_options_free(opts: *mut RsmqttOptions) {
    if !opts.is_null() {
        guard_val((), || drop(Box::from_raw(opts)));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_options_set(
    opts: *mut RsmqttOptions,
    name: *const c_char,
    value: *const c_char,
) -> c_int {
    guard(|| {
        let o = obj_mut(opts, "options")?;
        let n = cstr(name, "option name")?;
        let v = if value.is_null() { "" } else { cstr(value, "option value")? };
        o.set(n, v)
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_options_set_binary(
    opts: *mut RsmqttOptions,
    name: *const c_char,
    data: *const c_void,
    len: usize,
) -> c_int {
    guard(|| {
        let o = obj_mut(opts, "options")?;
        let n = cstr(name, "option name")?;
        o.set_binary(n, bytes(data, len)?)
    })
}

/// Attaches MQTT v5 properties: `which` = RSMQTT_PROPS_CONNECT or
/// RSMQTT_PROPS_WILL. The properties are copied.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_options_set_properties(
    opts: *mut RsmqttOptions,
    which: c_int,
    props: *const RsmqttProperties,
) -> c_int {
    guard(|| {
        let o = obj_mut(opts, "options")?;
        let p = props.as_ref().cloned().unwrap_or_default();
        match which {
            PROPS_CONNECT => o.connect_props = p,
            PROPS_WILL => o.will_props = p,
            _ => return Err(FfiError::new(RSMQTT_INVALID_ARGUMENT, "invalid property list selector")),
        }
        Ok(())
    })
}

/// 1 if `name` is an option name understood by `rsmqtt_options_set`.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_option_is_known(name: *const c_char) -> c_int {
    guard_val(0, || match cstr(name, "name") {
        Ok(n) => options::is_known_option(n) as c_int,
        Err(_) => 0,
    })
}

// ---------------------------------------------------------------------------
// Properties

#[no_mangle]
pub extern "C" fn rsmqtt_properties_new() -> *mut RsmqttProperties {
    guard_val(ptr::null_mut(), || Box::into_raw(Box::new(Properties::new())))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_clone(p: *const RsmqttProperties) -> *mut RsmqttProperties {
    guard_val(ptr::null_mut(), || match p.as_ref() {
        Some(p) => Box::into_raw(Box::new(p.clone())),
        None => ptr::null_mut(),
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_free(p: *mut RsmqttProperties) {
    if !p.is_null() {
        guard_val((), || drop(Box::from_raw(p)));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_clear(p: *mut RsmqttProperties) {
    if let Some(p) = p.as_mut() {
        p.clear();
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_count(p: *const RsmqttProperties) -> usize {
    p.as_ref().map(|p| p.len()).unwrap_or(0)
}

fn bad_prop(id: c_int) -> FfiError {
    FfiError::new(
        RSMQTT_INVALID_ARGUMENT,
        format!("property 0x{id:02X} is unknown or has another value type"),
    )
}

fn push_prop(p: &mut Properties, prop: Property) -> FfiResult<()> {
    p.try_push(prop).map_err(|e| FfiError::new(RSMQTT_INVALID_ARGUMENT, e.to_string()))
}

/// Adds an integer property (byte, two byte, four byte or variable int).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_add_int(
    p: *mut RsmqttProperties,
    id: c_int,
    value: u32,
) -> c_int {
    guard(|| {
        let p = obj_mut(p, "properties")?;
        let b = || u8::try_from(value).map_err(|_| bad_prop(id));
        let w = || u16::try_from(value).map_err(|_| bad_prop(id));
        let prop = match id {
            0x01 => Property::PayloadFormatIndicator(b()?),
            0x02 => Property::MessageExpiryInterval(value),
            0x0B => Property::SubscriptionIdentifier(value),
            0x11 => Property::SessionExpiryInterval(value),
            0x13 => Property::ServerKeepAlive(w()?),
            0x17 => Property::RequestProblemInformation(b()?),
            0x18 => Property::WillDelayInterval(value),
            0x19 => Property::RequestResponseInformation(b()?),
            0x21 => Property::ReceiveMaximum(w()?),
            0x22 => Property::TopicAliasMaximum(w()?),
            0x23 => Property::TopicAlias(w()?),
            0x24 => Property::MaximumQoS(b()?),
            0x25 => Property::RetainAvailable(b()?),
            0x27 => Property::MaximumPacketSize(value),
            0x28 => Property::WildcardSubscriptionAvailable(b()?),
            0x29 => Property::SubscriptionIdentifierAvailable(b()?),
            0x2A => Property::SharedSubscriptionAvailable(b()?),
            _ => return Err(bad_prop(id)),
        };
        push_prop(p, prop)
    })
}

/// Adds a string property (content type, response topic, ...).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_add_string(
    p: *mut RsmqttProperties,
    id: c_int,
    value: *const c_char,
) -> c_int {
    guard(|| {
        let p = obj_mut(p, "properties")?;
        let v = cstr(value, "value")?.to_string();
        let prop = match id {
            0x03 => Property::ContentType(v),
            0x08 => Property::ResponseTopic(v),
            0x12 => Property::AssignedClientIdentifier(v),
            0x15 => Property::AuthenticationMethod(v),
            0x1A => Property::ResponseInformation(v),
            0x1C => Property::ServerReference(v),
            0x1F => Property::ReasonString(v),
            _ => return Err(bad_prop(id)),
        };
        push_prop(p, prop)
    })
}

/// Adds a binary property (correlation data, authentication data).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_add_binary(
    p: *mut RsmqttProperties,
    id: c_int,
    data: *const c_void,
    len: usize,
) -> c_int {
    guard(|| {
        let p = obj_mut(p, "properties")?;
        let v = mqtt::bytes::Bytes::copy_from_slice(bytes(data, len)?);
        let prop = match id {
            0x09 => Property::CorrelationData(v),
            0x16 => Property::AuthenticationData(v),
            _ => return Err(bad_prop(id)),
        };
        push_prop(p, prop)
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_add_user(
    p: *mut RsmqttProperties,
    name: *const c_char,
    value: *const c_char,
) -> c_int {
    guard(|| {
        let p = obj_mut(p, "properties")?;
        let n = cstr(name, "name")?;
        let v = cstr(value, "value")?;
        push_prop(p, Property::UserProperty(n.to_string(), v.to_string()))
    })
}

/// Reads property `index` (0-based). Pointers stay valid as long as the
/// properties object (or the event owning it) lives.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_get(
    p: *const RsmqttProperties,
    index: usize,
    out: *mut RsmqttProperty,
) -> c_int {
    guard(|| {
        let p = obj(p, "properties")?;
        let out = obj_mut(out, "out")?;
        let prop = p
            .iter()
            .nth(index)
            .ok_or_else(|| FfiError::new(RSMQTT_INVALID_ARGUMENT, "property index out of range"))?;
        *out = RsmqttProperty {
            id: prop.property_id() as u8 as c_int,
            int_value: 0,
            data: ptr::null(),
            data_len: 0,
            name: ptr::null(),
            name_len: 0,
        };
        let set_str = |out: &mut RsmqttProperty, s: &[u8]| {
            out.data = s.as_ptr() as *const c_char;
            out.data_len = s.len();
        };
        match prop {
            Property::PayloadFormatIndicator(v)
            | Property::RequestProblemInformation(v)
            | Property::RequestResponseInformation(v)
            | Property::MaximumQoS(v)
            | Property::RetainAvailable(v)
            | Property::WildcardSubscriptionAvailable(v)
            | Property::SubscriptionIdentifierAvailable(v)
            | Property::SharedSubscriptionAvailable(v) => out.int_value = *v as u32,
            Property::ServerKeepAlive(v)
            | Property::ReceiveMaximum(v)
            | Property::TopicAliasMaximum(v)
            | Property::TopicAlias(v) => out.int_value = *v as u32,
            Property::MessageExpiryInterval(v)
            | Property::SubscriptionIdentifier(v)
            | Property::SessionExpiryInterval(v)
            | Property::WillDelayInterval(v)
            | Property::MaximumPacketSize(v) => out.int_value = *v,
            Property::ContentType(s)
            | Property::ResponseTopic(s)
            | Property::AssignedClientIdentifier(s)
            | Property::AuthenticationMethod(s)
            | Property::ResponseInformation(s)
            | Property::ServerReference(s)
            | Property::ReasonString(s) => set_str(out, s.as_bytes()),
            Property::CorrelationData(b) | Property::AuthenticationData(b) => set_str(out, b),
            Property::UserProperty(k, v) => {
                out.name = k.as_ptr() as *const c_char;
                out.name_len = k.len();
                set_str(out, v.as_bytes());
            }
        }
        Ok(())
    })
}

/// Copies the value of the first user property `name` into `buf`
/// (NUL-terminated, truncated to `buf_len`). Returns the full value length,
/// or -1 if there is no such user property.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_properties_get_user(
    p: *const RsmqttProperties,
    name: *const c_char,
    buf: *mut c_char,
    buf_len: usize,
) -> isize {
    guard_val(-1, || {
        let (Some(p), Ok(n)) = (p.as_ref(), cstr(name, "name")) else {
            return -1;
        };
        match p.get_user(n) {
            Some(v) => {
                copy_out(v.as_bytes(), buf, buf_len);
                v.len() as isize
            }
            None => -1,
        }
    })
}

unsafe fn copy_out(src: &[u8], buf: *mut c_char, buf_len: usize) {
    if buf.is_null() || buf_len == 0 {
        return;
    }
    let n = src.len().min(buf_len - 1);
    ptr::copy_nonoverlapping(src.as_ptr(), buf as *mut u8, n);
    *buf.add(n) = 0;
}

// ---------------------------------------------------------------------------
// Client

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_client_create(
    opts: *const RsmqttOptions,
    out: *mut *mut RsmqttClient,
) -> c_int {
    guard(|| {
        let o = obj(opts, "options")?;
        let out = obj_mut(out, "out")?;
        *out = ptr::null_mut();
        let cli = Client::new(o.clone())?;
        *out = Box::into_raw(Box::new(cli));
        Ok(())
    })
}

/// Destroys the client. Does not disconnect gracefully; call
/// `rsmqtt_disconnect` and wait for it first if that matters.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_client_destroy(cli: *mut RsmqttClient) {
    if !cli.is_null() {
        guard_val((), || drop(Box::from_raw(cli)));
    }
}

/// Replaces the options used by the next `rsmqtt_connect` (the create
/// options inside are ignored).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_client_set_options(
    cli: *mut RsmqttClient,
    opts: *const RsmqttOptions,
) -> c_int {
    guard(|| {
        let c = obj(cli, "client")?;
        let o = obj(opts, "options")?;
        o.connect_options()?; // validate now
        c.set_options(o.clone());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_set_event_notify(
    cli: *mut RsmqttClient,
    cb: Option<NotifyFn>,
    ctx: *mut c_void,
) -> c_int {
    guard(|| {
        obj(cli, "client")?.set_notify(cb, ctx);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_connect(cli: *mut RsmqttClient, token: *mut RsmqttToken) -> c_int {
    guard(|| {
        let t = obj(cli, "client")?.connect()?;
        put_token(token, t);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_reconnect(cli: *mut RsmqttClient, token: *mut RsmqttToken) -> c_int {
    guard(|| {
        let t = obj(cli, "client")?.reconnect()?;
        put_token(token, t);
        Ok(())
    })
}

/// `reason_code` < 0: normal disconnection (0x00).
/// `flags` bit 0: publish the will message (reason 0x04).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_disconnect(
    cli: *mut RsmqttClient,
    timeout_ms: c_int,
    reason_code: c_int,
    flags: c_int,
    props: *const RsmqttProperties,
    token: *mut RsmqttToken,
) -> c_int {
    guard(|| {
        let c = obj(cli, "client")?;
        let reason = if reason_code < 0 {
            None
        } else {
            let rc = u8::try_from(reason_code)
                .ok()
                .and_then(|r| mqtt::ReasonCode::try_from(r).ok())
                .ok_or_else(|| FfiError::new(RSMQTT_INVALID_ARGUMENT, "invalid reason code"))?;
            Some(rc)
        };
        let t = c.disconnect(ms(timeout_ms), reason, opt_props(props), flags & 1 != 0)?;
        put_token(token, t);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_is_connected(cli: *const RsmqttClient) -> c_int {
    guard_val(0, || cli.as_ref().map(|c| c.is_connected() as c_int).unwrap_or(0))
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_publish(
    cli: *mut RsmqttClient,
    topic: *const c_char,
    payload: *const c_void,
    len: usize,
    qos_: c_int,
    retained: c_int,
    props: *const RsmqttProperties,
    token: *mut RsmqttToken,
) -> c_int {
    guard(|| {
        let c = obj(cli, "client")?;
        let t = cstr(topic, "topic")?;
        let data = bytes(payload, len)?;
        let tok = c.publish(t, data, qos(qos_)?, retained != 0, opt_props(props))?;
        put_token(token, tok);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_subscribe(
    cli: *mut RsmqttClient,
    filter: *const c_char,
    qos_: c_int,
    flags: c_int,
    props: *const RsmqttProperties,
    token: *mut RsmqttToken,
) -> c_int {
    rsmqtt_subscribe_many(cli, 1, &filter, &qos_, &flags, props, token)
}

/// `flags` may be NULL (all 0).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_subscribe_many(
    cli: *mut RsmqttClient,
    count: usize,
    filters: *const *const c_char,
    qos_: *const c_int,
    flags: *const c_int,
    props: *const RsmqttProperties,
    token: *mut RsmqttToken,
) -> c_int {
    guard(|| {
        let c = obj(cli, "client")?;
        if count == 0 || filters.is_null() || qos_.is_null() {
            return Err(FfiError::new(RSMQTT_NULL_PARAMETER, "no topic filters given"));
        }
        let mut fs = Vec::with_capacity(count);
        let mut qs = Vec::with_capacity(count);
        let mut os = Vec::with_capacity(count);
        for i in 0..count {
            fs.push(cstr(*filters.add(i), "topic filter")?.to_string());
            qs.push(qos(*qos_.add(i))?);
            let f = if flags.is_null() { 0 } else { *flags.add(i) };
            os.push(sub_options(f)?);
        }
        let tok = c.subscribe_many(&fs, &qs, &os, opt_props(props))?;
        put_token(token, tok);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_unsubscribe(
    cli: *mut RsmqttClient,
    filter: *const c_char,
    props: *const RsmqttProperties,
    token: *mut RsmqttToken,
) -> c_int {
    rsmqtt_unsubscribe_many(cli, 1, &filter, props, token)
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_unsubscribe_many(
    cli: *mut RsmqttClient,
    count: usize,
    filters: *const *const c_char,
    props: *const RsmqttProperties,
    token: *mut RsmqttToken,
) -> c_int {
    guard(|| {
        let c = obj(cli, "client")?;
        if count == 0 || filters.is_null() {
            return Err(FfiError::new(RSMQTT_NULL_PARAMETER, "no topic filters given"));
        }
        let fs = (0..count)
            .map(|i| cstr(*filters.add(i), "topic filter").map(String::from))
            .collect::<FfiResult<Vec<_>>>()?;
        let tok = c.unsubscribe_many(&fs, opt_props(props))?;
        put_token(token, tok);
        Ok(())
    })
}

/// Blocks until the operation completed; returns its result code
/// (RSMQTT_TIMEOUT if it did not complete within `timeout_ms`).
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_wait_for_completion(
    cli: *mut RsmqttClient,
    token: RsmqttToken,
    timeout_ms: c_int,
) -> c_int {
    guard(|| obj(cli, "client")?.wait_for_completion(token, ms(timeout_ms)))
}

/// Returns the tokens of all operations not completed yet. Free the array
/// with `rsmqtt_free_tokens`.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_get_pending_tokens(
    cli: *mut RsmqttClient,
    tokens: *mut *mut RsmqttToken,
    count: *mut usize,
) -> c_int {
    guard(|| {
        let c = obj(cli, "client")?;
        let tokens = obj_mut(tokens, "tokens")?;
        let count = obj_mut(count, "count")?;
        let v = c.pending_tokens().into_boxed_slice();
        *count = v.len();
        *tokens = if v.is_empty() {
            ptr::null_mut()
        } else {
            Box::into_raw(v) as *mut RsmqttToken
        };
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_free_tokens(tokens: *mut RsmqttToken, count: usize) {
    if !tokens.is_null() && count > 0 {
        guard_val((), || {
            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(tokens, count)));
        });
    }
}

/// Number of events waiting in the queue.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_queued_events(cli: *const RsmqttClient) -> usize {
    guard_val(0, || cli.as_ref().map(|c| c.queued_events()).unwrap_or(0))
}

/// Number of incoming messages dropped because of `maxQueuedEvents`.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_dropped_events(cli: *const RsmqttClient) -> u64 {
    guard_val(0, || cli.as_ref().map(|c| c.dropped_events()).unwrap_or(0))
}

/// Copies the client ID into `buf`; returns its full length.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_client_id(
    cli: *const RsmqttClient,
    buf: *mut c_char,
    buf_len: usize,
) -> isize {
    guard_val(-1, || match cli.as_ref() {
        Some(c) => {
            let id = c.client_id();
            copy_out(id.as_bytes(), buf, buf_len);
            id.len() as isize
        }
        None => -1,
    })
}

// ---------------------------------------------------------------------------
// Events

/// Fetches the next event. `timeout_ms` 0 = don't wait, < 0 = wait forever.
/// Returns 1 and sets `*ev` if there is one, 0 if none, or a negative code.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_poll_event(
    cli: *mut RsmqttClient,
    timeout_ms: c_int,
    ev: *mut *mut RsmqttEvent,
) -> c_int {
    let mut got = 0;
    let rc = guard(|| {
        let c = obj(cli, "client")?;
        let out = obj_mut(ev, "event")?;
        *out = ptr::null_mut();
        let timeout = match timeout_ms {
            0 => None,
            t if t < 0 => Some(Duration::from_secs(u64::MAX / 4)),
            t => Some(ms(t)),
        };
        if let Some(e) = c.poll(timeout) {
            *out = Box::into_raw(Box::new(wrap_event(e)));
            got = 1;
        }
        Ok(())
    });
    if rc == RSMQTT_SUCCESS {
        got
    } else {
        rc
    }
}

fn to_cstring(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_default()
}

fn wrap_event(ev: Event) -> RsmqttEvent {
    let topic = ev
        .message
        .as_ref()
        .map(|m| to_cstring(m.topic()))
        .unwrap_or_default();
    RsmqttEvent {
        topic,
        text: to_cstring(&ev.text),
        server_uri: to_cstring(&ev.server_uri),
        ev,
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_event_free(ev: *mut RsmqttEvent) {
    if !ev.is_null() {
        guard_val((), || drop(Box::from_raw(ev)));
    }
}

macro_rules! ev_getter {
    ($name:ident, $ty:ty, $dflt:expr, |$e:ident| $body:expr) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(ev: *const RsmqttEvent) -> $ty {
            match ev.as_ref() {
                Some($e) => $body,
                None => $dflt,
            }
        }
    };
}

ev_getter!(rsmqtt_event_type, c_int, 0, |e| e.ev.kind as c_int);
ev_getter!(rsmqtt_event_op, c_int, 0, |e| e.ev.op as c_int);
ev_getter!(rsmqtt_event_token, RsmqttToken, 0, |e| e.ev.token);
ev_getter!(rsmqtt_event_rc, c_int, RSMQTT_NULL_PARAMETER, |e| e.ev.rc);
ev_getter!(rsmqtt_event_reason_code, c_int, 0, |e| e.ev.reason_code as c_int);
ev_getter!(rsmqtt_event_reason_code_count, usize, 0, |e| e.ev.reason_codes.len());
ev_getter!(rsmqtt_event_session_present, c_int, 0, |e| e.ev.session_present as c_int);
ev_getter!(rsmqtt_event_mqtt_version, c_int, 0, |e| e.ev.mqtt_version);
ev_getter!(rsmqtt_event_server_uri, *const c_char, ptr::null(), |e| e.server_uri.as_ptr());
ev_getter!(rsmqtt_event_text, *const c_char, ptr::null(), |e| e.text.as_ptr());
ev_getter!(rsmqtt_event_qos, c_int, 0, |e| e.ev.message.as_ref().map(|m| m.qos() as c_int).unwrap_or(0));
ev_getter!(rsmqtt_event_retained, c_int, 0, |e| e.ev.message.as_ref().map(|m| m.retained() as c_int).unwrap_or(0));
ev_getter!(rsmqtt_event_dup, c_int, 0, |e| e.ev.message.as_ref().map(|m| m.dup() as c_int).unwrap_or(0));
ev_getter!(rsmqtt_event_msgid, c_int, 0, |e| e
    .ev
    .message
    .as_ref()
    .and_then(|m| m.msgid())
    .map(|id| id as c_int)
    .unwrap_or(0));
ev_getter!(rsmqtt_event_properties, *const RsmqttProperties, ptr::null(), |e| &e.ev.properties as *const _);

#[no_mangle]
pub unsafe extern "C" fn rsmqtt_event_reason_code_at(ev: *const RsmqttEvent, index: usize) -> c_int {
    ev.as_ref()
        .and_then(|e| e.ev.reason_codes.get(index))
        .map(|r| *r as c_int)
        .unwrap_or(-1)
}

/// Topic of a message event (NUL-terminated); `len` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_event_topic(ev: *const RsmqttEvent, len: *mut usize) -> *const c_char {
    match ev.as_ref() {
        Some(e) => {
            if !len.is_null() {
                *len = e.topic.as_bytes().len();
            }
            e.topic.as_ptr()
        }
        None => ptr::null(),
    }
}

/// Payload of a message event (not NUL-terminated); `len` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn rsmqtt_event_payload(ev: *const RsmqttEvent, len: *mut usize) -> *const c_void {
    let payload = ev.as_ref().and_then(|e| e.ev.message.as_ref()).map(|m| m.payload());
    if !len.is_null() {
        *len = payload.map(|p| p.len()).unwrap_or(0);
    }
    match payload {
        Some(p) if !p.is_empty() => p.as_ptr() as *const c_void,
        _ => ptr::null(),
    }
}
