//! Forwards the `log` output of the Paho library to a C callback, like
//! Paho C's `MQTTAsync_setTraceCallback`.

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::{Once, RwLock};

/// `level`: 1=error 2=warn 3=info 4=debug 5=trace.
pub type TraceFn = extern "C" fn(level: c_int, message: *const c_char, ctx: *mut c_void);

struct Sink {
    func: TraceFn,
    ctx: usize,
}

static SINK: RwLock<Option<Sink>> = RwLock::new(None);
static INIT: Once = Once::new();

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        meta.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let guard = SINK.read().unwrap_or_else(|e| e.into_inner());
        if let Some(sink) = guard.as_ref() {
            let text = format!("{}: {}", record.target(), record.args());
            let c = CString::new(text.replace('\0', " ")).unwrap_or_default();
            (sink.func)(record.level() as c_int, c.as_ptr(), sink.ctx as *mut c_void);
        }
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

pub fn set(level: c_int, cb: Option<TraceFn>, ctx: *mut c_void) {
    INIT.call_once(|| {
        // Fails only if the process already installed another logger; then
        // tracing is simply not available through this function.
        let _ = log::set_logger(&LOGGER);
    });
    let filter = match (cb, level) {
        (None, _) | (_, i32::MIN..=0) => log::LevelFilter::Off,
        (_, 1) => log::LevelFilter::Error,
        (_, 2) => log::LevelFilter::Warn,
        (_, 3) => log::LevelFilter::Info,
        (_, 4) => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = cb.map(|func| Sink {
        func,
        ctx: ctx as usize,
    });
    log::set_max_level(filter);
}
