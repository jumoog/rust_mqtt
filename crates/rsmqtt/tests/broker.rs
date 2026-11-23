//! Integration tests of the C ABI against a real broker.
//!
//! They run only when `RSMQTT_TEST_URI` is set, e.g.
//!   RSMQTT_TEST_URI=tcp://localhost:18830 RSMQTT_TEST_WS_URI=ws://localhost:18831 cargo test
//! (see tests/mosquitto.conf in the repository root for a broker setup).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::time::{Duration, Instant};

use rsmqtt::error::*;
use rsmqtt::*;

fn uri() -> Option<String> {
    std::env::var("RSMQTT_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(rsmqtt_last_error()).to_string_lossy().into_owned() }
}

fn unique(prefix: &str) -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{n}")
}

/// Minimal safe wrapper for the tests.
struct Cli(*mut RsmqttClient);

#[derive(Debug)]
struct Ev {
    kind: i32,
    op: i32,
    token: u64,
    rc: i32,
    topic: String,
    payload: Vec<u8>,
    qos: i32,
    retained: bool,
    user_prop: Option<String>,
    reason_codes: Vec<i32>,
}

impl Cli {
    fn new(opts: &[(&str, &str)]) -> Self {
        unsafe {
            let o = rsmqtt_options_new();
            for (k, v) in opts {
                let rc = rsmqtt_options_set(o, c(k).as_ptr(), c(v).as_ptr());
                assert_eq!(rc, 0, "set {k}={v}: {}", last_error());
            }
            let mut cli = ptr::null_mut();
            let rc = rsmqtt_client_create(o, &mut cli);
            rsmqtt_options_free(o);
            assert_eq!(rc, 0, "create: {}", last_error());
            Cli(cli)
        }
    }

    fn connect(&self) {
        unsafe {
            let mut tok = 0;
            assert_eq!(rsmqtt_connect(self.0, &mut tok), 0, "connect: {}", last_error());
            let rc = rsmqtt_wait_for_completion(self.0, tok, 10_000);
            assert_eq!(rc, 0, "connect wait: {}", last_error());
        }
    }

    fn subscribe(&self, filter: &str, qos: i32) {
        unsafe {
            let mut tok = 0;
            let f = c(filter);
            assert_eq!(rsmqtt_subscribe(self.0, f.as_ptr(), qos, 0, ptr::null(), &mut tok), 0);
            assert_eq!(rsmqtt_wait_for_completion(self.0, tok, 10_000), 0, "{}", last_error());
        }
    }

    fn publish(&self, topic: &str, payload: &[u8], qos: i32, props: *const RsmqttProperties) -> u64 {
        unsafe {
            let mut tok = 0;
            let t = c(topic);
            let rc = rsmqtt_publish(self.0, t.as_ptr(), payload.as_ptr().cast(), payload.len(), qos, 0, props, &mut tok);
            assert_eq!(rc, 0, "publish: {}", last_error());
            tok
        }
    }

    fn disconnect(&self) {
        unsafe {
            let mut tok = 0;
            assert_eq!(rsmqtt_disconnect(self.0, 2000, -1, 0, ptr::null(), &mut tok), 0);
            assert_eq!(rsmqtt_wait_for_completion(self.0, tok, 10_000), 0, "{}", last_error());
        }
    }

    fn poll(&self, timeout: Duration) -> Option<Ev> {
        unsafe {
            let mut e = ptr::null_mut();
            let rc = rsmqtt_poll_event(self.0, timeout.as_millis() as i32, &mut e);
            assert!(rc >= 0, "poll: {}", last_error());
            if rc == 0 {
                return None;
            }
            let mut tlen = 0;
            let t = rsmqtt_event_topic(e, &mut tlen);
            let mut plen = 0;
            let p = rsmqtt_event_payload(e, &mut plen);
            let mut buf = [0 as c_char; 128];
            let n = rsmqtt_properties_get_user(rsmqtt_event_properties(e), c("origin").as_ptr(), buf.as_mut_ptr(), buf.len());
            let ev = Ev {
                kind: rsmqtt_event_type(e),
                op: rsmqtt_event_op(e),
                token: rsmqtt_event_token(e),
                rc: rsmqtt_event_rc(e),
                topic: CStr::from_ptr(t).to_string_lossy().into_owned(),
                payload: if p.is_null() { vec![] } else { std::slice::from_raw_parts(p as *const u8, plen).to_vec() },
                qos: rsmqtt_event_qos(e),
                retained: rsmqtt_event_retained(e) != 0,
                user_prop: (n >= 0).then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()),
                reason_codes: (0..rsmqtt_event_reason_code_count(e)).map(|i| rsmqtt_event_reason_code_at(e, i)).collect(),
            };
            rsmqtt_event_free(e);
            Some(ev)
        }
    }

    /// Collects message events until `n` arrived or the timeout expired.
    fn messages(&self, n: usize, timeout: Duration) -> Vec<Ev> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            if let Some(ev) = self.poll(Duration::from_millis(100)) {
                if ev.kind == EventKind::Message as i32 {
                    out.push(ev);
                }
            }
        }
        out
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        unsafe { rsmqtt_client_destroy(self.0) }
    }
}

fn roundtrip(server: &str, version: &str) {
    let topic = unique(&format!("rsmqtt/test/v{version}"));
    let sub = Cli::new(&[("serverURI", server), ("clientId", &unique("sub")), ("MQTTVersion", version)]);
    let publ = Cli::new(&[("serverURI", server), ("clientId", &unique("pub")), ("MQTTVersion", version)]);
    sub.connect();
    publ.connect();
    sub.subscribe(&format!("{topic}/#"), 2);

    for qos in 0..=2 {
        publ.publish(&format!("{topic}/q{qos}"), format!("hello {qos}").as_bytes(), qos, ptr::null());
    }
    let msgs = sub.messages(3, Duration::from_secs(10));
    assert_eq!(msgs.len(), 3, "got {msgs:?}");
    for (qos, m) in msgs.iter().enumerate() {
        assert_eq!(m.topic, format!("{topic}/q{qos}"));
        assert_eq!(m.payload, format!("hello {qos}").as_bytes());
        assert_eq!(m.qos, qos as i32);
    }

    // The publisher got one Complete event per publish.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut completes = 0;
    while completes < 3 && Instant::now() < deadline {
        if let Some(ev) = publ.poll(Duration::from_millis(100)) {
            if ev.kind == EventKind::Complete as i32 && ev.op == OpKind::Publish as i32 {
                assert_eq!(ev.rc, 0);
                assert!(ev.token > 0);
                completes += 1;
            }
        }
    }
    assert_eq!(completes, 3);

    publ.disconnect();
    sub.disconnect();
}

#[test]
fn roundtrip_v5() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    roundtrip(&u, "5");
}

#[test]
fn roundtrip_v311() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    roundtrip(&u, "4");
}

#[test]
fn roundtrip_v31() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    roundtrip(&u, "3");
}

#[test]
fn roundtrip_websocket() {
    let Some(u) = std::env::var("RSMQTT_TEST_WS_URI").ok().filter(|s| !s.is_empty()) else {
        return eprintln!("RSMQTT_TEST_WS_URI not set; skipped");
    };
    roundtrip(&u, "5");
}

/// TLS: RSMQTT_TEST_TLS_URI=mqtts://localhost:18883 and RSMQTT_TEST_CA=<ca.crt>
#[test]
fn tls_trust_store() {
    let (Some(u), Some(ca)) = (
        std::env::var("RSMQTT_TEST_TLS_URI").ok().filter(|s| !s.is_empty()),
        std::env::var("RSMQTT_TEST_CA").ok().filter(|s| !s.is_empty()),
    ) else {
        return eprintln!("RSMQTT_TEST_TLS_URI / RSMQTT_TEST_CA not set; skipped");
    };

    // Our CA: chain and host name verify.
    let cli = Cli::new(&[
        ("serverURI", &u),
        ("clientId", &unique("tls")),
        ("ssl.trustStore", &ca),
        ("ssl.verify", "1"),
    ]);
    cli.connect();
    let topic = unique("rsmqtt/test/tls");
    cli.subscribe(&topic, 1);
    cli.publish(&topic, b"secure", 1, ptr::null());
    let msgs = cli.messages(1, Duration::from_secs(5));
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].payload, b"secure");
    cli.disconnect();

    // Without our CA the self-signed chain must be rejected.
    let bad = Cli::new(&[
        ("serverURI", &u),
        ("clientId", &unique("tlsbad")),
        ("ssl.disableDefaultTrustStore", "0"),
    ]);
    unsafe {
        let mut tok = 0;
        let rc = rsmqtt_connect(bad.0, &mut tok);
        let rc = if rc == 0 { rsmqtt_wait_for_completion(bad.0, tok, 10_000) } else { rc };
        assert!(rc < 0, "untrusted server certificate was accepted");
        eprintln!("untrusted cert rejected: rc={rc} {}", last_error());
    }
}

#[test]
fn v5_properties_and_suback() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    let topic = unique("rsmqtt/test/props");
    let cli = Cli::new(&[("serverURI", &u), ("clientId", &unique("props")), ("MQTTVersion", "5")]);
    cli.connect();

    unsafe {
        // subscribe_many: one valid filter, one the broker must refuse
        // ('#' not last is invalid -> rejected locally), so use two valid
        // ones and check both reason codes.
        let f1 = c(&format!("{topic}/a"));
        let f2 = c(&format!("{topic}/b"));
        let filters = [f1.as_ptr(), f2.as_ptr()];
        let qos = [1, 2];
        let flags = [RSMQTT_SUB_NO_LOCAL, 0];
        let mut tok = 0;
        assert_eq!(rsmqtt_subscribe_many(cli.0, 2, filters.as_ptr(), qos.as_ptr(), flags.as_ptr(), ptr::null(), &mut tok), 0);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut codes = None;
        while codes.is_none() && Instant::now() < deadline {
            if let Some(ev) = cli.poll(Duration::from_millis(100)) {
                if ev.token == tok {
                    assert_eq!(ev.op, OpKind::Subscribe as i32);
                    codes = Some(ev.reason_codes);
                }
            }
        }
        assert_eq!(codes.unwrap(), vec![1, 2]);

        // no-local on /a: our own message must not come back; /b must.
        let props = rsmqtt_properties_new();
        assert_eq!(rsmqtt_properties_add_user(props, c("origin").as_ptr(), c("rsmqtt").as_ptr()), 0);
        assert_eq!(rsmqtt_properties_add_string(props, RSMQTT_PROP_CONTENT_TYPE_TEST, c("text/plain").as_ptr()), 0);
        // wrong value type is refused
        assert_eq!(rsmqtt_properties_add_int(props, RSMQTT_PROP_CONTENT_TYPE_TEST, 1), RSMQTT_INVALID_ARGUMENT);
        cli.publish(&format!("{topic}/a"), b"local", 1, props);
        cli.publish(&format!("{topic}/b"), b"remote", 1, props);
        rsmqtt_properties_free(props);
    }

    let msgs = cli.messages(2, Duration::from_secs(3));
    assert_eq!(msgs.len(), 1, "no-local not honoured: {msgs:?}");
    assert_eq!(msgs[0].payload, b"remote");
    assert_eq!(msgs[0].user_prop.as_deref(), Some("rsmqtt"));
    cli.disconnect();
}

const RSMQTT_PROP_CONTENT_TYPE_TEST: i32 = 0x03;

#[test]
fn will_message_and_retained() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    let topic = unique("rsmqtt/test/will");
    let watcher = Cli::new(&[("serverURI", &u), ("clientId", &unique("watch"))]);
    watcher.connect();
    watcher.subscribe(&format!("{topic}/#"), 1);

    let dying = Cli::new(&[
        ("serverURI", &u),
        ("clientId", &unique("dying")),
        ("will.topic", &format!("{topic}/lwt")),
        ("will.payload", "offline"),
        ("will.qos", "1"),
    ]);
    dying.connect();
    unsafe {
        let mut tok = 0;
        // Disconnect "with will message" (v5 reason 0x04).
        assert_eq!(rsmqtt_disconnect(dying.0, 1000, -1, 1, ptr::null(), &mut tok), 0, "{}", last_error());
        assert_eq!(rsmqtt_wait_for_completion(dying.0, tok, 5000), 0);
    }
    let msgs = watcher.messages(1, Duration::from_secs(5));
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].payload, b"offline");

    // retained message is delivered to a late subscriber with retained=1
    let publ = Cli::new(&[("serverURI", &u), ("clientId", &unique("ret"))]);
    publ.connect();
    unsafe {
        let mut tok = 0;
        let t = c(&format!("{topic}/state"));
        assert_eq!(rsmqtt_publish(publ.0, t.as_ptr(), b"on".as_ptr().cast(), 2, 1, 1, ptr::null(), &mut tok), 0);
        assert_eq!(rsmqtt_wait_for_completion(publ.0, tok, 5000), 0);
    }
    let late = Cli::new(&[("serverURI", &u), ("clientId", &unique("late"))]);
    late.connect();
    late.subscribe(&format!("{topic}/state"), 1);
    let msgs = late.messages(1, Duration::from_secs(5));
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].retained);
    // clear the retained message
    publ_clear(&publ, &format!("{topic}/state"));
}

fn publ_clear(cli: &Cli, topic: &str) {
    unsafe {
        let mut tok = 0;
        let t = c(topic);
        rsmqtt_publish(cli.0, t.as_ptr(), ptr::null(), 0, 1, 1, ptr::null(), &mut tok);
        rsmqtt_wait_for_completion(cli.0, tok, 5000);
    }
}

#[test]
fn errors_are_reported() {
    unsafe {
        // unknown option
        let o = rsmqtt_options_new();
        assert_eq!(rsmqtt_options_set(o, c("noSuchOption").as_ptr(), c("1").as_ptr()), RSMQTT_UNKNOWN_OPTION);
        assert!(last_error().contains("noSuchOption"));
        // missing server URI
        let mut cli = ptr::null_mut();
        assert_eq!(rsmqtt_client_create(o, &mut cli), RSMQTT_NULL_PARAMETER);
        assert!(cli.is_null());
        // bad scheme fails at connect time
        rsmqtt_options_set(o, c("serverURI").as_ptr(), c("foo://localhost:1").as_ptr());
        let rc = rsmqtt_client_create(o, &mut cli);
        if rc == 0 {
            let mut tok = 0;
            let rc = rsmqtt_connect(cli, &mut tok);
            let rc = if rc == 0 { rsmqtt_wait_for_completion(cli, tok, 5000) } else { rc };
            assert!(rc < 0);
            rsmqtt_client_destroy(cli);
        }
        rsmqtt_options_free(o);

        // publish while not connected (no offline buffering) fails
        let cli = Cli::new(&[("serverURI", "tcp://127.0.0.1:1")]);
        let mut tok = 0;
        let rc = rsmqtt_publish(cli.0, c("a").as_ptr(), ptr::null(), 0, 1, 0, ptr::null(), &mut tok);
        let rc = if rc == 0 { rsmqtt_wait_for_completion(cli.0, tok, 2000) } else { rc };
        assert!(rc < 0, "expected failure, got {rc}");

        // bad QoS / NULL handles
        assert_eq!(rsmqtt_publish(cli.0, c("a").as_ptr(), ptr::null(), 0, 3, 0, ptr::null(), &mut tok), RSMQTT_BAD_QOS);
        assert_eq!(rsmqtt_publish(ptr::null_mut(), c("a").as_ptr(), ptr::null(), 0, 1, 0, ptr::null(), &mut tok), RSMQTT_NULL_PARAMETER);
        let mut ev = ptr::null_mut();
        assert_eq!(rsmqtt_poll_event(ptr::null_mut(), 0, &mut ev), RSMQTT_NULL_PARAMETER);
    }
}

#[test]
fn offline_buffering_delivers_after_connect() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    let topic = unique("rsmqtt/test/offline");
    let sub = Cli::new(&[("serverURI", &u), ("clientId", &unique("osub"))]);
    sub.connect();
    sub.subscribe(&topic, 1);

    let publ = Cli::new(&[
        ("serverURI", &u),
        ("clientId", &unique("opub")),
        ("sendWhileDisconnected", "1"),
        ("allowDisconnectedSendAtAnyTime", "1"),
        ("maxBufferedMessages", "10"),
    ]);
    let tok = publ.publish(&topic, b"buffered", 1, ptr::null());
    unsafe {
        let mut toks = ptr::null_mut();
        let mut n = 0;
        assert_eq!(rsmqtt_get_pending_tokens(publ.0, &mut toks, &mut n), 0);
        let pending = std::slice::from_raw_parts(toks, n).to_vec();
        rsmqtt_free_tokens(toks, n);
        assert!(pending.contains(&tok));
    }
    publ.connect();
    let msgs = sub.messages(1, Duration::from_secs(5));
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].payload, b"buffered");
}
