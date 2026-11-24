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

/// `prefix` plus 64 random bits: client IDs and topics must not collide with
/// other users of a public test broker (a second client with the same ID
/// makes the broker drop the first).
fn unique(prefix: &str) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // RandomState is seeded from the OS RNG
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    format!("{prefix}-{:016x}", h.finish())
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
            // as long as Paho's default connect timeout: a public broker whose
            // IPv6 addresses refuse MQTT costs ~2 s per address before IPv4
            let rc = rsmqtt_wait_for_completion(self.0, tok, 30_000);
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
    // MQTT orders messages only per topic (and QoS), so the three topics
    // may arrive in any order; check each one by its topic.
    for qos in 0..=2 {
        let t = format!("{topic}/q{qos}");
        let m = msgs.iter().find(|m| m.topic == t).unwrap_or_else(|| panic!("nothing on {t}: {msgs:?}"));
        assert_eq!(m.payload, format!("hello {qos}").as_bytes());
        assert_eq!(m.qos, qos);
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

/// Public broker with a publicly trusted certificate, e.g.
/// RSMQTT_TEST_PUBLIC_TLS_URI=mqtts://broker.hivemq.com:8883 and
/// (optional) RSMQTT_TEST_PUBLIC_WSS_URI=wss://broker.hivemq.com:8884/mqtt
///
/// Shows that no CA file is needed (the system trust store is used), that
/// the host name is checked by default, and what `ssl.verify=0` and
/// `ssl.disableDefaultTrustStore=1` change.
#[test]
fn tls_public_ca_and_hostname() {
    let Some(u) = std::env::var("RSMQTT_TEST_PUBLIC_TLS_URI").ok().filter(|s| !s.is_empty()) else {
        return eprintln!("RSMQTT_TEST_PUBLIC_TLS_URI not set; skipped");
    };
    let try_connect = |opts: &[(&str, &str)]| -> (i32, String) {
        let cli = Cli::new(opts);
        unsafe {
            let mut tok = 0;
            let rc = rsmqtt_connect(cli.0, &mut tok);
            let rc = if rc == 0 { rsmqtt_wait_for_completion(cli.0, tok, 30_000) } else { rc };
            (rc, last_error())
        }
    };

    // 1. no CA file: the system trust store verifies the public certificate
    let cli = Cli::new(&[("serverURI", &u), ("clientId", &unique("tls-public"))]);
    cli.connect();
    let topic = unique("rsmqtt/test/tls-public");
    cli.subscribe(&topic, 1);
    cli.publish(&topic, b"over tls", 1, ptr::null());
    let msgs = cli.messages(1, Duration::from_secs(10));
    assert_eq!(msgs.len(), 1);
    cli.disconnect();
    eprintln!("1. {u} without CA file: connected, round trip ok");

    // 2. same over secure WebSocket
    if let Some(w) = std::env::var("RSMQTT_TEST_PUBLIC_WSS_URI").ok().filter(|s| !s.is_empty()) {
        let cli = Cli::new(&[("serverURI", &w), ("clientId", &unique("wss-public"))]);
        cli.connect();
        cli.disconnect();
        eprintln!("2. {w} without CA file: connected");
    }

    // 3. host name check: connect by IPv4 address -> name mismatch
    let rest = u.split_once("://").unwrap().1;
    let (host, port) = rest.rsplit_once(':').unwrap();
    use std::net::ToSocketAddrs;
    let ip = (host, port.parse::<u16>().unwrap())
        .to_socket_addrs()
        .unwrap()
        .find(|a| a.is_ipv4())
        .expect("IPv4 address")
        .ip();
    let by_ip = format!("mqtts://{ip}:{port}");
    let (rc, err) = try_connect(&[("serverURI", &by_ip), ("clientId", &unique("tls-ip"))]);
    assert!(rc < 0, "certificate accepted for {by_ip} although it is issued for {host}");
    eprintln!("3. {by_ip} (default, host name checked): refused: rc={rc} {err}");

    // 4. ssl.verify=0 waives only the name check; the chain is still verified
    let (rc, err) = try_connect(&[("serverURI", &by_ip), ("clientId", &unique("tls-ip-noverify")), ("ssl.verify", "0")]);
    assert_eq!(rc, 0, "ssl.verify=0 should accept the right certificate under another name: {err}");
    eprintln!("4. {by_ip} with ssl.verify=0: connected");

    // 5. without the system trust store and without a CA file: refused
    let (rc, err) = try_connect(&[("serverURI", &u), ("clientId", &unique("tls-nostore")), ("ssl.disableDefaultTrustStore", "1")]);
    assert!(rc < 0, "connected without any trust anchor");
    eprintln!("5. {u} with ssl.disableDefaultTrustStore=1: refused: rc={rc} {err}");
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


// ----- MQTT v5 enhanced authentication (SCRAM)
//
// Need a broker with SCRAM authentication: RSMQTT_TEST_SCRAM_URI plus
// RSMQTT_TEST_SCRAM_USER / RSMQTT_TEST_SCRAM_PASSWORD (SCRAM-SHA-256), e.g.
// EMQX with EMQX_AUTHENTICATION__1__MECHANISM=scram (see the README).

fn scram_env() -> Option<(String, String, String)> {
    let get = |n: &str| std::env::var(n).ok().filter(|s| !s.is_empty());
    Some((get("RSMQTT_TEST_SCRAM_URI")?, get("RSMQTT_TEST_SCRAM_USER")?, get("RSMQTT_TEST_SCRAM_PASSWORD")?))
}

/// Connects and returns the result code of the connect.
fn try_connect(cli: &Cli) -> i32 {
    unsafe {
        let mut tok = 0;
        let rc = rsmqtt_connect(cli.0, &mut tok);
        if rc != 0 {
            return rc;
        }
        rsmqtt_wait_for_completion(cli.0, tok, 15_000)
    }
}

#[test]
fn scram_built_in() {
    let Some((server, user, password)) = scram_env() else { return };
    let topic = unique("rsmqtt/test/scram");
    let cli = Cli::new(&[
        ("serverURI", &server),
        ("clientId", &unique("scram")),
        ("MQTTVersion", "5"),
        ("auth.method", "SCRAM-SHA-256"),
        ("username", &user),
        ("password", &password),
    ]);
    assert_eq!(try_connect(&cli), 0, "SCRAM connect: {}", last_error());
    cli.subscribe(&topic, 1);
    cli.publish(&topic, b"authenticated", 1, ptr::null());
    let msgs = cli.messages(1, Duration::from_secs(10));
    assert_eq!(msgs.first().map(|m| m.payload.as_slice()), Some(&b"authenticated"[..]));
    cli.disconnect();

    // a wrong password: the broker refuses the client's proof
    let bad = Cli::new(&[
        ("serverURI", &server),
        ("clientId", &unique("scram")),
        ("MQTTVersion", "5"),
        ("auth.method", "SCRAM-SHA-256"),
        ("username", &user),
        ("password", "wrong"),
    ]);
    let rc = try_connect(&bad);
    assert_ne!(rc, 0, "a wrong password must not connect");
    // without enhanced authentication the broker does not accept the user
    let plain = Cli::new(&[("serverURI", &server), ("clientId", &unique("scram")), ("MQTTVersion", "5"),
                          ("username", &user), ("password", &password)]);
    assert_ne!(try_connect(&plain), 0, "the broker must require SCRAM for this test");
}

/// A host callback that answers with the library's own SCRAM code: the
/// callback path end to end.
extern "C" fn scram_callback(
    ctx: *mut std::os::raw::c_void,
    method: *const c_char,
    data: *const std::os::raw::c_void,
    len: usize,
    reply: *mut rsmqtt::options::AuthReply,
) -> i32 {
    unsafe {
        let s = &*(ctx as *const rsmqtt::scram::Scram);
        assert_eq!(CStr::from_ptr(method).to_str().unwrap(), "SCRAM-SHA-256");
        let data = if len == 0 { &[][..] } else { std::slice::from_raw_parts(data as *const u8, len) };
        match s.respond(data) {
            Ok(answer) => rsmqtt_auth_reply_set(reply, answer.as_ptr().cast(), answer.len()),
            Err(_) => 1,
        }
    }
}

#[test]
fn auth_callback() {
    let Some((server, user, password)) = scram_env() else { return };
    let scram = Box::new(rsmqtt::scram::Scram::new(rsmqtt::scram::Mechanism::Sha256, &user, password.as_bytes()).unwrap());
    let first = scram.client_first();
    unsafe {
        let o = rsmqtt_options_new();
        for (k, v) in [("serverURI", server.as_str()), ("clientId", &unique("authcb")), ("MQTTVersion", "5"),
                       ("auth.method", "SCRAM-SHA-256"), ("username", &user)] {
            assert_eq!(rsmqtt_options_set(o, c(k).as_ptr(), c(v).as_ptr()), 0, "{k}: {}", last_error());
        }
        assert_eq!(rsmqtt_options_set_binary(o, c("auth.data").as_ptr(), first.as_ptr().cast(), first.len()), 0);
        let ctx = &*scram as *const rsmqtt::scram::Scram as *mut std::os::raw::c_void;
        assert_eq!(rsmqtt_options_set_auth_callback(o, Some(scram_callback), ctx), 0);
        let mut cli = ptr::null_mut();
        assert_eq!(rsmqtt_client_create(o, &mut cli), 0, "{}", last_error());
        rsmqtt_options_free(o);
        let cli = Cli(cli);
        assert_eq!(try_connect(&cli), 0, "callback connect: {}", last_error());
        cli.disconnect();
    }
}
