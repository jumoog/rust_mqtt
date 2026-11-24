//! Settings of one broker connection: the rsmqtt options the client gets,
//! the driver's own settings (default QoS and retain flag of the addresses,
//! reconnect interval) and the Sparkplug host ids.
//!
//! A broker is configured in the config file (`[rsmqtt]`,
//! `connection = "<name>"`, one entry per option, see
//! [`Settings::set_option`]) or by a `_RsMqttBroker` DP
//! ([`Settings::apply_broker_config`]). `redu.<option>` entries apply to the
//! second redundant host only ([`Settings::apply_redu_option`]).
//!
//! Passwords may be encrypted with `_DriverSecurity.PublicKey`; decrypting
//! them needs the WinCC OA API, so the driver passes a `reveal` function.

use std::path::{Component, Path, PathBuf};

use rsmqtt::options::{is_known_option, normalize_name, OptionsData, Persistence};

use crate::{redu, sparkplug};

/// The settings of a `_RsMqttBroker` DP (`Config.*`).
#[derive(Debug, Clone)]
pub struct BrokerConfig {
    pub server_uri: String,
    pub server_uris: Vec<String>,
    pub client_id: String,
    pub mqtt_version: i32,
    pub keep_alive: i32,
    pub clean_session: bool,
    pub username: String,
    pub password: String,
    pub tls_trust_store: String,
    pub tls_key_store: String,
    pub tls_private_key: String,
    pub tls_verify: bool,
    pub will_topic: String,
    pub will_payload: String,
    pub will_qos: i32,
    pub will_retain: bool,
    pub subscribe_qos: i32,
    pub publish_qos: i32,
    pub publish_retain: bool,
    /// `name=value`, any rsmqtt option, `sparkplug.hostId` or `redu.<option>`
    pub options: Vec<String>,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            server_uri: String::new(),
            server_uris: Vec::new(),
            client_id: String::new(),
            mqtt_version: 0,
            keep_alive: 0,
            clean_session: true,
            username: String::new(),
            password: String::new(),
            tls_trust_store: String::new(),
            tls_key_store: String::new(),
            tls_private_key: String::new(),
            tls_verify: false,
            will_topic: String::new(),
            will_payload: String::new(),
            will_qos: 0,
            will_retain: false,
            subscribe_qos: 1,
            publish_qos: 1,
            publish_retain: false,
            options: Vec::new(),
        }
    }
}

/// Decrypts a stored password (returns plain values unchanged); the error
/// says why it failed, never the secret.
pub type Reveal<'a> = dyn FnMut(&str) -> Result<String, String> + 'a;

/// Options whose value may be encrypted with `_DriverSecurity.PublicKey`.
pub fn is_secret_option(name: &str) -> bool {
    matches!(normalize_name(name).as_str(), "password" | "auth.data")
}

fn is_spb_host_id(name: &str) -> bool {
    normalize_name(name) == "sparkplug.hostid"
}

const SPB_HOST_ID_INVALID: &str = "sparkplug.hostId must not contain / + #";

/// `<client id>` of a connection without `clientId`: `WCCOArsmqtt_<num>_<name>`.
pub fn default_client_id(name: &str, man_num: i32) -> String {
    format!("WCCOArsmqtt_{man_num}_{name}")
}

#[derive(Debug, Clone)]
pub struct Settings {
    name: String,
    man_num: i32,
    /// The rsmqtt options as configured (see [`Settings::client_options`]).
    pub options: OptionsData,
    pub subscribe_qos: i32,
    pub publish_qos: i32,
    pub publish_retain: bool,
    /// Retry of failed connects (config file `reconnectInterval`).
    pub reconnect_interval_ms: i64,
    /// `redu.clientId` was given: host 2 uses it as it is.
    redu_client_id: bool,
    /// `sparkplug.hostId`: STATE messages and will of the host application.
    spb_host_id: String,
    /// `redu.sparkplug.hostId`: the second redundant host's own host id.
    spb_redu_host_id: String,
    /// Timestamp of the STATE messages (online and will) of the current client.
    state_ts: i64,
}

impl Settings {
    /// The driver's defaults for a connection of this name; every one can be
    /// overridden.
    pub fn new(name: &str, man_num: i32) -> Self {
        let mut options = OptionsData::default();
        let client_id = default_client_id(name, man_num);
        for (n, v) in [
            ("clientId", client_id.as_str()),
            ("automaticReconnect", "1"),
            ("minRetryInterval", "1"),
            ("maxRetryInterval", "30"),
            ("sendWhileDisconnected", "1"),
            ("allowDisconnectedSendAtAnyTime", "1"),
            ("maxBufferedMessages", "10000"),
            ("deleteOldestMessages", "1"),
            ("completionEvents", "errors"),
        ] {
            options.set(n, v).expect("valid default option");
        }
        Self {
            name: name.to_string(),
            man_num,
            options,
            subscribe_qos: 1,
            publish_qos: 1,
            publish_retain: false,
            reconnect_interval_ms: 10_000,
            redu_client_id: false,
            spb_host_id: String::new(),
            spb_redu_host_id: String::new(),
            state_ts: 0,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The library reconnects lost connections itself.
    pub fn auto_reconnect(&self) -> bool {
        self.options.automatic_reconnect.unwrap_or(false)
    }

    /// Timestamp of this client's STATE messages (set by `client_options`).
    pub fn state_ts(&self) -> i64 {
        self.state_ts
    }

    /// Sets one rsmqtt option (config file).
    pub fn set_option(&mut self, name: &str, value: &str) -> Result<(), String> {
        self.options.set(name, value).map_err(|e| e.message)
    }

    /// `sparkplug.hostId` (config file).
    pub fn set_spb_host_id(&mut self, id: &str) -> Result<(), String> {
        if !id.is_empty() && !sparkplug::valid_id(id) {
            return Err(SPB_HOST_ID_INVALID.into());
        }
        self.spb_host_id = id.to_string();
        Ok(())
    }

    /// A `redu.<option>` entry: on the second redundant host it sets
    /// `<option>`, on host 1 it is only checked.
    pub fn apply_redu_option(&mut self, option: &str, value: &str, host2: bool) -> Result<(), String> {
        if is_spb_host_id(option) {
            let id = value.trim();
            if !id.is_empty() && !sparkplug::valid_id(id) {
                return Err(format!("redu.{SPB_HOST_ID_INVALID}"));
            }
            self.spb_redu_host_id = id.to_string(); // used on host 2 only (spb_state_host)
            return Ok(());
        }
        if !is_known_option(option) {
            return Err(format!("unknown option 'redu.{option}'"));
        }
        if !host2 {
            return Ok(());
        }
        self.set_option(option, value)?;
        if normalize_name(option) == "clientid" {
            self.redu_client_id = true;
        }
        Ok(())
    }

    /// Builds the settings from a broker DP. On failure nothing changes.
    pub fn apply_broker_config(&mut self, cfg: &BrokerConfig, host2: bool, reveal: &mut Reveal) -> Result<(), String> {
        if cfg.server_uri.is_empty() && cfg.server_uris.is_empty() {
            return Err("Config.ServerURI is empty".into());
        }
        let mut n = Settings::new(&self.name, self.man_num);
        n.reconnect_interval_ms = self.reconnect_interval_ms;
        n.state_ts = self.state_ts;

        let uri = if cfg.server_uri.is_empty() { &cfg.server_uris[0] } else { &cfg.server_uri };
        n.set_option("serverURI", uri)?;
        if !cfg.server_uris.is_empty() {
            n.set_option("serverURIs", &cfg.server_uris.join(" "))?;
        }
        if !cfg.client_id.is_empty() {
            n.set_option("clientId", &cfg.client_id)?;
        }
        if cfg.mqtt_version != 0 {
            n.set_option("MQTTVersion", &cfg.mqtt_version.to_string())?;
        }
        if cfg.keep_alive > 0 {
            n.set_option("keepAliveInterval", &cfg.keep_alive.to_string())?;
        }
        n.set_option("cleansession", bool_text(cfg.clean_session))?;
        if !cfg.username.is_empty() {
            n.set_option("username", &cfg.username)?;
        }
        if !cfg.password.is_empty() {
            let plain = reveal(&cfg.password).map_err(|e| format!("Config.Password: {e}"))?;
            n.set_option("password", &plain)?;
        }
        if !cfg.tls_trust_store.is_empty() {
            n.set_option("ssl.trustStore", &cfg.tls_trust_store)?;
        }
        if !cfg.tls_key_store.is_empty() {
            n.set_option("ssl.keyStore", &cfg.tls_key_store)?;
        }
        if !cfg.tls_private_key.is_empty() {
            n.set_option("ssl.privateKey", &cfg.tls_private_key)?;
        }
        // Config.TlsVerify decides the host-name check both ways (the chain is
        // checked either way). Only for TLS URIs: ssl.* options attach TLS
        // settings to the connection. Config.Options "ssl.verify=..." still wins.
        if n.options.has_secure_uri() {
            n.set_option("ssl.verify", bool_text(cfg.tls_verify))?;
        }
        if !cfg.will_topic.is_empty() {
            n.set_option("will.topic", &cfg.will_topic)?;
            n.set_option("will.payload", &cfg.will_payload)?;
            n.set_option("will.qos", &cfg.will_qos.to_string())?;
            n.set_option("will.retained", bool_text(cfg.will_retain))?;
        }

        // redu.<option> entries last: on the second redundant host they win
        let mut redu = Vec::new();
        for opt in &cfg.options {
            if opt.is_empty() {
                continue;
            }
            let Some((name, value)) = opt.split_once('=') else {
                return Err(format!("Config.Options entry '{opt}' is not 'name=value'"));
            };
            if is_spb_host_id(name) {
                n.set_spb_host_id(value.trim())?;
                continue;
            }
            if let Some(option) = redu::split_redu_option(name.trim()) {
                redu.push((option.trim(), value));
                continue;
            }
            if is_secret_option(name) {
                let plain = reveal(value).map_err(|e| format!("Config.Options '{name}': {e}"))?;
                n.set_option(name, &plain)?;
            } else {
                n.set_option(name, value)?;
            }
        }
        for (option, value) in redu {
            if host2 && is_secret_option(option) {
                let plain = reveal(value).map_err(|e| format!("Config.Options 'redu.{option}': {e}"))?;
                n.apply_redu_option(option, &plain, host2)?;
            } else {
                n.apply_redu_option(option, value, host2)?;
            }
        }

        let qos_ok = |q: i32| (0..=2).contains(&q);
        if !qos_ok(cfg.subscribe_qos) || !qos_ok(cfg.publish_qos) {
            return Err("Config.SubscribeQoS / Config.PublishQoS must be 0, 1 or 2".into());
        }
        n.subscribe_qos = cfg.subscribe_qos;
        n.publish_qos = cfg.publish_qos;
        n.publish_retain = cfg.publish_retain;
        *self = n;
        Ok(())
    }

    /// The host id this driver publishes STATE for ("" for none). Two
    /// redundant drivers must not share one: each has its own will and STATE
    /// timestamp, and edge nodes ignore STATE older than the last one. So
    /// host 1 uses sparkplug.hostId and host 2 only redu.sparkplug.hostId
    /// (without it, host 2 reads and writes but is no primary host).
    pub fn spb_state_host(&self, host2: bool) -> &str {
        if host2 { &self.spb_redu_host_id } else { &self.spb_host_id }
    }

    /// The options the client is created with:
    /// - Sparkplug host application: the will is its STATE offline message
    ///   (timestamp `now_ms`, kept in [`Settings::state_ts`]);
    /// - the second redundant host connects with `<clientId>_2`;
    /// - file persistence goes to `<data_dir>/rsmqtt`, a relative
    ///   persistenceDir is taken below `data_dir` (the folder is created).
    ///
    /// Returns the options and warnings for the log.
    pub fn client_options(&mut self, host2: bool, data_dir: &str, now_ms: i64) -> (OptionsData, Vec<String>) {
        let mut o = self.options.clone();
        let mut warnings = Vec::new();
        let host = self.spb_state_host(host2).to_string();
        if !host.is_empty() {
            if o.will_topic.is_some() {
                warnings.push("sparkplug.hostId: the configured will is replaced by the STATE will of the host application".into());
            }
            self.state_ts = now_ms;
            for (n, v) in [
                ("will.topic", sparkplug::state_topic(&host)),
                ("will.payload", sparkplug::state_payload(false, now_ms)),
                ("will.qos", "1".into()),
                ("will.retained", "1".into()),
            ] {
                o.set(n, &v).expect("valid will option");
            }
        }

        let id = o.client_id.clone().unwrap_or_default();
        o.client_id = Some(redu::client_id_for_host(&id, host2, self.redu_client_id));

        if matches!(o.persistence, Persistence::File) {
            let dir = o.persistence_dir.take().unwrap_or_default();
            let dir = if dir.as_os_str().is_empty() {
                Path::new(data_dir).join("rsmqtt")
            } else if dir.is_relative() {
                Path::new(data_dir).join(dir)
            } else {
                dir
            };
            let _ = std::fs::create_dir_all(&dir); // paho reports a folder it cannot use
            o.persistence_dir = Some(PathBuf::from(normal_path(&dir)));
        }
        (o, warnings)
    }
}

fn bool_text(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

/// The path with `.` and `..` resolved and `/` as separator.
fn normal_path(p: &Path) -> String {
    let mut prefix = String::new();
    let mut root = false;
    let mut parts: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            Component::Prefix(x) => prefix = x.as_os_str().to_string_lossy().replace('\\', "/"),
            Component::RootDir => root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.last().is_some_and(|l| l != "..") {
                    parts.pop();
                } else if !root {
                    parts.push("..".into());
                }
            }
            Component::Normal(x) => parts.push(x.to_string_lossy().into_owned()),
        }
    }
    let mut s = prefix;
    if root {
        s.push('/');
    }
    s.push_str(&parts.join("/"));
    if s.is_empty() {
        s.push('.');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> Result<String, String> {
        Ok(s.to_string())
    }

    fn cfg(uri: &str) -> BrokerConfig {
        BrokerConfig { server_uri: uri.into(), ..Default::default() }
    }

    fn apply(s: &mut Settings, c: &BrokerConfig, host2: bool) -> Result<(), String> {
        s.apply_broker_config(c, host2, &mut plain)
    }

    #[test]
    fn defaults() {
        let s = Settings::new("plant", 3);
        assert_eq!(s.options.client_id.as_deref(), Some("WCCOArsmqtt_3_plant"));
        assert!(s.auto_reconnect());
        assert_eq!(s.options.max_buffered_messages, Some(10000));
        assert_eq!((s.subscribe_qos, s.publish_qos, s.publish_retain, s.reconnect_interval_ms), (1, 1, false, 10_000));
        assert!(is_secret_option("Password") && is_secret_option(" pass_word ") && !is_secret_option("username"));
    }

    #[test]
    fn broker_config() {
        let mut s = Settings::new("plant", 3);
        let mut c = cfg("");
        assert_eq!(apply(&mut s, &c, false).unwrap_err(), "Config.ServerURI is empty");
        c.server_uris = vec!["tcp://a:1883".into(), "tcp://b:1883".into()];
        c.client_id = "scada".into();
        c.mqtt_version = 5;
        c.keep_alive = 20;
        c.clean_session = false;
        c.will_topic = "lwt".into();
        c.will_payload = "gone".into();
        c.will_qos = 1;
        c.subscribe_qos = 2;
        c.publish_qos = 0;
        c.publish_retain = true;
        c.options = vec!["".into(), "maxInflight=5".into(), "automaticReconnect=0".into()];
        apply(&mut s, &c, false).unwrap();
        let o = &s.options;
        assert_eq!(o.server_uri.as_deref(), Some("tcp://a:1883"));
        assert_eq!(o.server_uris, ["tcp://a:1883", "tcp://b:1883"]);
        assert_eq!(o.client_id.as_deref(), Some("scada"));
        assert_eq!(o.keep_alive, Some(std::time::Duration::from_secs(20)));
        assert_eq!(o.clean, Some(false));
        assert_eq!(o.will_topic.as_deref(), Some("lwt"));
        assert_eq!(o.max_inflight, Some(5));
        assert!(!o.ssl.used && o.ssl.verify.is_none());
        assert!(!s.auto_reconnect());
        assert_eq!((s.subscribe_qos, s.publish_qos, s.publish_retain), (2, 0, true));

        // a new configuration starts from the defaults again
        apply(&mut s, &cfg("tcp://x"), false).unwrap();
        assert_eq!(s.options.client_id.as_deref(), Some("WCCOArsmqtt_3_plant"));
        assert!(s.auto_reconnect() && s.options.will_topic.is_none() && s.options.server_uris.is_empty());
        assert_eq!((s.subscribe_qos, s.publish_qos, s.publish_retain), (1, 1, false));
    }

    #[test]
    fn tls_verify_only_for_tls_uris() {
        let mut s = Settings::new("p", 1);
        let mut c = cfg("mqtts://broker:8883");
        c.tls_verify = true;
        apply(&mut s, &c, false).unwrap();
        assert_eq!(s.options.ssl.verify, Some(true));
        c.options = vec!["ssl.verify=0".into()];
        apply(&mut s, &c, false).unwrap();
        assert_eq!(s.options.ssl.verify, Some(false));
        let mut c = cfg("tcp://broker");
        c.server_uris = vec!["wss://broker/mqtt".into()];
        apply(&mut s, &c, false).unwrap();
        assert_eq!(s.options.ssl.verify, Some(false));
        apply(&mut s, &cfg("tcp://broker"), false).unwrap();
        assert!(s.options.ssl.verify.is_none() && !s.options.ssl.used);
    }

    #[test]
    fn errors_leave_the_settings_alone() {
        let mut s = Settings::new("p", 1);
        apply(&mut s, &cfg("tcp://good"), false).unwrap();
        let bad = |opts: &[&str]| BrokerConfig { options: opts.iter().map(|s| s.to_string()).collect(), ..cfg("tcp://bad") };
        let mut check = |c: BrokerConfig, host2: bool, msg: &str| {
            assert_eq!(apply(&mut s, &c, host2).unwrap_err(), msg);
            assert_eq!(s.options.server_uri.as_deref(), Some("tcp://good"));
        };
        check(bad(&["nonsense"]), false, "Config.Options entry 'nonsense' is not 'name=value'");
        check(bad(&["bogus=1"]), false, "unknown option 'bogus'");
        check(bad(&["will.qos=3"]), false, "option 'will.qos': QoS must be 0, 1 or 2");
        check(bad(&["redu.bogus=1"]), false, "unknown option 'redu.bogus'");
        check(bad(&["redu.will.qos=3"]), true, "option 'will.qos': QoS must be 0, 1 or 2");
        check(bad(&["sparkplug.hostId=a/b"]), false, "sparkplug.hostId must not contain / + #");
        check(bad(&["redu.sparkplug.hostId=a#"]), false, "redu.sparkplug.hostId must not contain / + #");
        check(BrokerConfig { publish_qos: 3, ..cfg("tcp://bad") }, false, "Config.SubscribeQoS / Config.PublishQoS must be 0, 1 or 2");
        check(BrokerConfig { subscribe_qos: -1, ..cfg("tcp://bad") }, false, "Config.SubscribeQoS / Config.PublishQoS must be 0, 1 or 2");
    }

    #[test]
    fn secrets_are_revealed() {
        let seen = std::cell::RefCell::new(Vec::new());
        let mut reveal = |s: &str| {
            seen.borrow_mut().push(s.to_string());
            if s == "broken" { Err("decryption failed".to_string()) } else { Ok(format!("plain-{s}")) }
        };
        let mut s = Settings::new("p", 1);
        let c = BrokerConfig {
            password: "a".into(),
            options: vec!["Pass_Word=b".into(), "redu.password=c".into(), "username=u".into()],
            ..cfg("tcp://x")
        };
        s.apply_broker_config(&c, false, &mut reveal).unwrap();
        assert_eq!(s.options.password.as_deref(), Some(&b"plain-b"[..]));
        // host 1 only checks redu.<option>: nothing to decrypt
        assert_eq!(*seen.borrow(), ["a", "b"]);
        s.apply_broker_config(&c, true, &mut reveal).unwrap();
        assert_eq!(s.options.password.as_deref(), Some(&b"plain-c"[..]));

        let fails = |c: BrokerConfig, host2: bool, msg: &str| {
            let mut s = Settings::new("p", 1);
            let mut reveal = |s: &str| if s == "broken" { Err("decryption failed".to_string()) } else { Ok(s.to_string()) };
            assert_eq!(s.apply_broker_config(&c, host2, &mut reveal).unwrap_err(), msg);
        };
        fails(BrokerConfig { password: "broken".into(), ..cfg("tcp://x") }, false, "Config.Password: decryption failed");
        fails(BrokerConfig { options: vec!["password=broken".into()], ..cfg("tcp://x") }, false, "Config.Options 'password': decryption failed");
        fails(BrokerConfig { options: vec!["redu.password=broken".into()], ..cfg("tcp://x") }, true, "Config.Options 'redu.password': decryption failed");
    }

    #[test]
    fn redundancy() {
        let mut s = Settings::new("plant", 3);
        let c = BrokerConfig {
            options: vec!["redu.serverURI=tcp://backup".into(), "sparkplug.hostId=scada".into(), "redu.sparkplug.hostId=scada2".into()],
            ..cfg("tcp://main")
        };
        apply(&mut s, &c, false).unwrap();
        assert_eq!(s.options.server_uri.as_deref(), Some("tcp://main"));
        assert_eq!((s.spb_state_host(false), s.spb_state_host(true)), ("scada", "scada2"));
        let (o, _) = s.client_options(false, "", 0);
        assert_eq!(o.client_id.as_deref(), Some("WCCOArsmqtt_3_plant"));

        apply(&mut s, &c, true).unwrap();
        assert_eq!(s.options.server_uri.as_deref(), Some("tcp://backup"));
        let (o, _) = s.client_options(true, "", 0);
        assert_eq!(o.client_id.as_deref(), Some("WCCOArsmqtt_3_plant_2"));

        let c = BrokerConfig { options: vec!["redu.clientId=other".into()], ..cfg("tcp://main") };
        apply(&mut s, &c, true).unwrap();
        let (o, _) = s.client_options(true, "", 0);
        assert_eq!(o.client_id.as_deref(), Some("other"));
        // the config file path
        let mut s = Settings::new("plant", 3);
        s.apply_redu_option("clientId", "x", false).unwrap();
        assert_eq!(s.client_options(true, "", 0).0.client_id.as_deref(), Some("WCCOArsmqtt_3_plant_2"));
        s.apply_redu_option("clientId", "x", true).unwrap();
        assert_eq!(s.client_options(true, "", 0).0.client_id.as_deref(), Some("x"));
        s.set_option("clientId", "").unwrap();
        s.redu_client_id = false;
        assert_eq!(s.client_options(true, "", 0).0.client_id.as_deref(), Some(""));
    }

    #[test]
    fn sparkplug_state_will() {
        let mut s = Settings::new("p", 1);
        s.set_option("will.topic", "lwt").unwrap();
        let (o, w) = s.client_options(false, "", 5);
        assert!(w.is_empty());
        assert_eq!(o.will_topic.as_deref(), Some("lwt"));
        s.set_spb_host_id("scada").unwrap();
        assert!(s.set_spb_host_id("a+b").is_err());
        let (o, w) = s.client_options(false, "", 1234);
        assert_eq!(w.len(), 1);
        assert_eq!(o.will_topic.as_deref(), Some("spBv1.0/STATE/scada"));
        assert_eq!(o.will_payload, br#"{"online":false,"timestamp":1234}"#);
        assert_eq!(format!("{:?}", o.will_qos), "Some(QoS1)");
        assert!(o.will_retained);
        assert_eq!(s.state_ts(), 1234);
        // the configured options keep the configured will
        assert_eq!(s.options.will_topic.as_deref(), Some("lwt"));
        // host 2 without redu.sparkplug.hostId is no primary host
        let (o, _) = s.client_options(true, "", 99);
        assert_eq!(o.will_topic.as_deref(), Some("lwt"));
        assert_eq!(s.state_ts(), 1234);
    }

    #[test]
    fn persistence_folder() {
        let data = std::env::temp_dir().join(format!("rsmqtt-wincc-settings-{}", std::process::id()));
        let data_dir = data.to_string_lossy().replace('\\', "/");
        let dir_of = |opts: &[(&str, &str)]| {
            let mut s = Settings::new("p", 1);
            for (n, v) in opts {
                s.set_option(n, v).unwrap();
            }
            // the folder of file persistence (None: no file persistence)
            let o = s.client_options(false, &format!("{data_dir}/"), 0).0;
            let file = matches!(o.persistence, Persistence::File);
            o.persistence_dir.filter(|_| file).map(|d| d.to_string_lossy().into_owned())
        };
        assert_eq!(dir_of(&[]), None);
        assert_eq!(dir_of(&[("persistence", "memory"), ("persistenceDir", "x")]), None);
        assert_eq!(dir_of(&[("persistence", "file")]).unwrap(), format!("{data_dir}/rsmqtt"));
        assert_eq!(dir_of(&[("persistenceDir", "")]).unwrap(), format!("{data_dir}/rsmqtt"));
        assert_eq!(dir_of(&[("persistenceDir", "sub/./a/../b")]).unwrap(), format!("{data_dir}/sub/b"));
        assert!(data.join("sub").join("b").is_dir() && data.join("rsmqtt").is_dir());
        let abs = data.join("abs");
        let abs_text = abs.to_string_lossy().replace('\\', "/");
        assert_eq!(dir_of(&[("persistenceDir", &abs_text)]).unwrap(), abs_text);
        assert!(abs.is_dir());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn normal_paths() {
        assert_eq!(normal_path(Path::new("a/./b/../c")), "a/c");
        assert_eq!(normal_path(Path::new("../a")), "../a");
        assert_eq!(normal_path(Path::new("/a/../../b")), "/b");
        assert_eq!(normal_path(Path::new("a/..")), ".");
        #[cfg(windows)]
        assert_eq!(normal_path(Path::new(r"C:\data\x\..\rsmqtt")), "C:/data/rsmqtt");
    }
}
