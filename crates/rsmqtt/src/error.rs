//! Return codes of the C API and the thread-local "last error" text.
//!
//! The negative codes up to -19 have the same values and meaning as the
//! `MQTTASYNC_*` codes of the Paho C library, so code written against Paho C
//! can keep its error handling. Codes from -100 down are specific to rsmqtt.

use std::cell::RefCell;
use std::ffi::CString;

use paho_mqtt as mqtt;

pub const RSMQTT_SUCCESS: i32 = 0;
pub const RSMQTT_FAILURE: i32 = -1;
pub const RSMQTT_PERSISTENCE_ERROR: i32 = -2;
pub const RSMQTT_DISCONNECTED: i32 = -3;
pub const RSMQTT_MAX_MESSAGES_INFLIGHT: i32 = -4;
pub const RSMQTT_BAD_UTF8_STRING: i32 = -5;
pub const RSMQTT_NULL_PARAMETER: i32 = -6;
pub const RSMQTT_BAD_STRUCTURE: i32 = -8;
pub const RSMQTT_BAD_QOS: i32 = -9;
pub const RSMQTT_OPERATION_INCOMPLETE: i32 = -11;
pub const RSMQTT_MAX_BUFFERED_MESSAGES: i32 = -12;
pub const RSMQTT_SSL_NOT_SUPPORTED: i32 = -13;
pub const RSMQTT_BAD_PROTOCOL: i32 = -14;
pub const RSMQTT_BAD_MQTT_OPTION: i32 = -15;
pub const RSMQTT_WRONG_MQTT_VERSION: i32 = -16;
pub const RSMQTT_ZERO_LEN_WILL_TOPIC: i32 = -17;
pub const RSMQTT_COMMAND_IGNORED: i32 = -18;

pub const RSMQTT_TIMEOUT: i32 = -100;
pub const RSMQTT_BAD_TOPIC: i32 = -101;
pub const RSMQTT_TLS_ERROR: i32 = -102;
pub const RSMQTT_IO_ERROR: i32 = -103;
pub const RSMQTT_REASON_CODE: i32 = -104;
pub const RSMQTT_PROTOCOL_ERROR: i32 = -105;
pub const RSMQTT_ALREADY_CONNECTED: i32 = -106;
pub const RSMQTT_CONNECT_IN_PROGRESS: i32 = -107;
pub const RSMQTT_PROXY_ERROR: i32 = -108;
pub const RSMQTT_WEBSOCKET_ERROR: i32 = -109;
pub const RSMQTT_INVALID_ARGUMENT: i32 = -110;
pub const RSMQTT_SESSION_NOT_RESUMED: i32 = -111;
pub const RSMQTT_UNKNOWN_OPTION: i32 = -112;
pub const RSMQTT_PANIC: i32 = -199;

/// An error of the C layer: the return code plus a human readable text.
#[derive(Debug, Clone)]
pub struct FfiError {
    pub rc: i32,
    pub reason_code: u8,
    pub message: String,
}

impl FfiError {
    pub fn new(rc: i32, message: impl Into<String>) -> Self {
        Self {
            rc,
            reason_code: 0,
            message: message.into(),
        }
    }
}

pub type FfiResult<T> = std::result::Result<T, FfiError>;

impl From<mqtt::Error> for FfiError {
    fn from(err: mqtt::Error) -> Self {
        let message = err.to_string();
        let mut reason_code = 0u8;
        let rc = error_rc(&err, &mut reason_code);
        Self {
            rc,
            reason_code,
            message,
        }
    }
}

fn error_rc(err: &mqtt::Error, reason: &mut u8) -> i32 {
    use mqtt::Error as E;
    match err {
        E::Failure => RSMQTT_FAILURE,
        E::Disconnected => RSMQTT_DISCONNECTED,
        E::AlreadyConnected => RSMQTT_ALREADY_CONNECTED,
        E::ConnectInProgress => RSMQTT_CONNECT_IN_PROGRESS,
        E::SessionNotResumed => RSMQTT_SESSION_NOT_RESUMED,
        E::TlsNotSupported => RSMQTT_SSL_NOT_SUPPORTED,
        E::WebSocketNotSupported => RSMQTT_BAD_PROTOCOL,
        E::Tls(_) | E::MissingTlsOptions => RSMQTT_TLS_ERROR,
        E::WebSocket(_) => RSMQTT_WEBSOCKET_ERROR,
        E::Proxy(_) => RSMQTT_PROXY_ERROR,
        E::WrongMqttVersion => RSMQTT_WRONG_MQTT_VERSION,
        E::Protocol(_) | E::ProtocolViolation(_) => RSMQTT_PROTOCOL_ERROR,
        E::Publish(inner, _) => error_rc(inner, reason),
        E::ReasonCode(rc, _) => {
            *reason = *rc as u8;
            RSMQTT_REASON_CODE
        }
        E::Persistence(_) => RSMQTT_PERSISTENCE_ERROR,
        E::BadTopicFilter => RSMQTT_BAD_TOPIC,
        E::InvalidArgument(_) | E::BadProperty | E::Conversion => RSMQTT_INVALID_ARGUMENT,
        E::Io(_) => RSMQTT_IO_ERROR,
        E::Utf8(_) => RSMQTT_BAD_UTF8_STRING,
        E::Timeout => RSMQTT_TIMEOUT,
        #[allow(unreachable_patterns)]
        _ => RSMQTT_FAILURE,
    }
}

/// Text for a return code (static, never freed).
pub fn strerror(rc: i32) -> &'static str {
    match rc {
        RSMQTT_SUCCESS => "Success",
        RSMQTT_FAILURE => "General failure",
        RSMQTT_PERSISTENCE_ERROR => "Persistence error",
        RSMQTT_DISCONNECTED => "Client disconnected",
        RSMQTT_MAX_MESSAGES_INFLIGHT => "Maximum in-flight messages reached",
        RSMQTT_BAD_UTF8_STRING => "Invalid UTF-8 string",
        RSMQTT_NULL_PARAMETER => "NULL parameter",
        RSMQTT_BAD_STRUCTURE => "Bad structure",
        RSMQTT_BAD_QOS => "Invalid QoS value",
        RSMQTT_OPERATION_INCOMPLETE => "Operation incomplete",
        RSMQTT_MAX_BUFFERED_MESSAGES => "Maximum buffered messages reached",
        RSMQTT_SSL_NOT_SUPPORTED => "TLS not supported",
        RSMQTT_BAD_PROTOCOL => "Unsupported protocol in server URI",
        RSMQTT_BAD_MQTT_OPTION => "Invalid MQTT option value",
        RSMQTT_WRONG_MQTT_VERSION => "Wrong MQTT version",
        RSMQTT_ZERO_LEN_WILL_TOPIC => "Zero length will topic",
        RSMQTT_COMMAND_IGNORED => "Command ignored",
        RSMQTT_TIMEOUT => "Timeout",
        RSMQTT_BAD_TOPIC => "Invalid topic or topic filter",
        RSMQTT_TLS_ERROR => "TLS error",
        RSMQTT_IO_ERROR => "I/O error",
        RSMQTT_REASON_CODE => "The server returned an error reason code",
        RSMQTT_PROTOCOL_ERROR => "MQTT protocol error",
        RSMQTT_ALREADY_CONNECTED => "Already connected",
        RSMQTT_CONNECT_IN_PROGRESS => "A connect is already in progress",
        RSMQTT_PROXY_ERROR => "HTTP proxy error",
        RSMQTT_WEBSOCKET_ERROR => "WebSocket error",
        RSMQTT_INVALID_ARGUMENT => "Invalid argument",
        RSMQTT_SESSION_NOT_RESUMED => "The server did not resume the session",
        RSMQTT_UNKNOWN_OPTION => "Unknown option name",
        RSMQTT_PANIC => "Internal error (panic)",
        _ => "Unknown error",
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

pub fn set_last_error(msg: &str) {
    let c = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
}

pub fn clear_last_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::default());
}

/// Pointer to the calling thread's last error text; valid until the next
/// rsmqtt call on this thread.
pub fn last_error_ptr() -> *const std::os::raw::c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}
