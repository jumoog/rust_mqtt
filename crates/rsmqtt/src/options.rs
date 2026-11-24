//! String based option handling.
//!
//! All client settings are given as `name = value` pairs. The names follow
//! the field names of the Paho C structures (`MQTTAsync_createOptions`,
//! `MQTTAsync_connectOptions`, `MQTTAsync_willOptions`,
//! `MQTTAsync_SSLOptions`), so they can be taken over 1:1 from existing Paho
//! configurations. Names are case-insensitive; `ssl.` and `tls.` prefixes are
//! synonyms. This keeps the C ABI stable (no versioned structs) and maps
//! directly onto key/value config files such as the WinCC OA config file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use paho_mqtt as mqtt;
use mqtt::{MqttVersion, Properties, Property, QoS};

use crate::error::*;

/// How completed operations are reported through the event queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompletionEvents {
    /// Every completed operation produces an event (default).
    #[default]
    All,
    /// Only failed operations produce an event.
    Errors,
    /// No completion events; use `rsmqtt_wait_for_completion` instead.
    None,
}

#[derive(Debug, Clone, Default)]
pub enum Persistence {
    #[default]
    None,
    Memory,
    /// File persistence, in `OptionsData::persistence_dir` (or the current
    /// working directory).
    File,
}

#[derive(Debug, Clone, Default)]
pub struct SslData {
    pub used: bool,
    pub trust_store: Option<String>,
    pub key_store: Option<String>,
    pub private_key: Option<String>,
    pub private_key_password: Option<String>,
    pub enabled_cipher_suites: Option<String>,
    pub enable_server_cert_auth: Option<bool>,
    pub verify: Option<bool>,
    pub ca_path: Option<String>,
    pub disable_default_trust_store: Option<bool>,
    pub version: Option<mqtt::TlsVersion>,
    pub alpn: Vec<String>,
}

/// Which property list a properties handle is attached to.
pub const PROPS_CONNECT: i32 = 1;
pub const PROPS_WILL: i32 = 2;

/// The collected settings of an `rsmqtt_options` handle.
#[derive(Debug, Clone, Default)]
pub struct OptionsData {
    // ----- create options -----
    pub server_uri: Option<String>,
    pub client_id: Option<String>,
    pub mqtt_version: Option<MqttVersion>,
    pub persistence: Persistence,
    /// Set by an explicit `persistence=`; `persistenceDir` alone implies file
    /// persistence but never overrides an explicit choice.
    pub persistence_explicit: bool,
    pub persistence_dir: Option<PathBuf>,
    pub send_while_disconnected: Option<bool>,
    pub max_buffered_messages: Option<usize>,
    pub allow_disconnected_send_at_any_time: Option<bool>,
    pub delete_oldest_messages: Option<bool>,
    pub restore_messages: Option<bool>,
    pub persist_qos0: Option<bool>,
    pub runtime_threads: Option<usize>,
    pub completion_events: CompletionEvents,
    pub max_queued_events: usize,

    // ----- connect options -----
    pub keep_alive: Option<Duration>,
    pub clean: Option<bool>,
    pub max_inflight: Option<u16>,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    pub connect_timeout: Option<Duration>,
    pub server_uris: Vec<String>,
    pub automatic_reconnect: Option<bool>,
    pub min_retry_interval: Option<Duration>,
    pub max_retry_interval: Option<Duration>,
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub http_headers: Vec<(String, String)>,
    pub connect_props: Properties,

    // ----- will -----
    pub will_topic: Option<String>,
    pub will_payload: Vec<u8>,
    pub will_qos: Option<QoS>,
    pub will_retained: bool,
    pub will_props: Properties,

    // ----- tls -----
    pub ssl: SslData,
}

/// All option names that `set` understands (lower case, canonical prefix).
pub const OPTION_NAMES: &[&str] = &[
    // create
    "serveruri",
    "clientid",
    "mqttversion",
    "persistence",
    "persistencedir",
    "sendwhiledisconnected",
    "maxbufferedmessages",
    "allowdisconnectedsendatanytime",
    "deleteoldestmessages",
    "restoremessages",
    "persistqos0",
    "runtimethreads",
    "completionevents",
    "maxqueuedevents",
    // connect
    "keepaliveinterval",
    "cleansession",
    "cleanstart",
    "maxinflight",
    "username",
    "password",
    "connecttimeout",
    "serveruris",
    "automaticreconnect",
    "minretryinterval",
    "maxretryinterval",
    "httpproxy",
    "httpsproxy",
    "httpheader",
    // v5 connect properties
    "sessionexpiryinterval",
    "receivemaximum",
    "maximumpacketsize",
    "topicaliasmaximum",
    "requestresponseinformation",
    "requestprobleminformation",
    "userproperty",
    // will
    "will.topic",
    "will.payload",
    "will.message",
    "will.qos",
    "will.retained",
    "will.delayinterval",
    "will.messageexpiryinterval",
    "will.contenttype",
    "will.responsetopic",
    "will.payloadformatindicator",
    "will.userproperty",
    // tls
    "ssl.truststore",
    "ssl.keystore",
    "ssl.privatekey",
    "ssl.privatekeypassword",
    "ssl.enabledciphersuites",
    "ssl.enableservercertauth",
    "ssl.verify",
    "ssl.capath",
    "ssl.disabledefaulttruststore",
    "ssl.sslversion",
    "ssl.alpn",
];

/// Normalises an option name: lower case, `tls.` → `ssl.`, `_`/`-` removed
/// (so `keep_alive_interval`, `keepAliveInterval` and `KEEPALIVEINTERVAL`
/// are the same option).
pub fn normalize_name(name: &str) -> String {
    // `ssl_trustStore`, `tls-verify`, `will_topic` → dotted form.
    let trimmed = name.trim();
    let mut dotted = String::with_capacity(trimmed.len());
    for prefix in ["ssl", "tls", "will"] {
        if trimmed.len() > prefix.len() + 1
            && trimmed[..prefix.len()].eq_ignore_ascii_case(prefix)
            && matches!(trimmed.as_bytes()[prefix.len()], b'_' | b'-')
        {
            dotted = format!("{prefix}.{}", &trimmed[prefix.len() + 1..]);
            break;
        }
    }
    let src = if dotted.is_empty() { trimmed } else { dotted.as_str() };
    let mut n: String = src
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(|c| c.to_lowercase())
        .collect();
    if let Some(rest) = n.strip_prefix("tls.") {
        n = format!("ssl.{rest}");
    }
    n
}

pub fn is_known_option(name: &str) -> bool {
    let n = normalize_name(name);
    OPTION_NAMES.contains(&n.as_str())
}

fn bad(name: &str, value: &str, what: &str) -> FfiError {
    FfiError::new(
        RSMQTT_BAD_MQTT_OPTION,
        format!("option '{name}': invalid value '{value}' ({what})"),
    )
}

pub fn parse_bool(name: &str, v: &str) -> FfiResult<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(bad(name, v, "expected a boolean")),
    }
}

fn parse_num<T: std::str::FromStr>(name: &str, v: &str) -> FfiResult<T> {
    v.trim()
        .parse::<T>()
        .map_err(|_| bad(name, v, "expected a number in range"))
}

/// Durations are given in seconds (like Paho C); fractions and the suffixes
/// `ms`, `s`, `min` and `h` are accepted, too.
pub fn parse_duration(name: &str, v: &str) -> FfiResult<Duration> {
    let t = v.trim().to_ascii_lowercase();
    let (num, mul) = if let Some(n) = t.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = t.strip_suffix("min") {
        (n, 60.0)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3600.0)
    } else {
        (t.as_str(), 1.0)
    };
    let secs: f64 = num
        .trim()
        .parse()
        .map_err(|_| bad(name, v, "expected a duration in seconds"))?;
    if !secs.is_finite() || secs < 0.0 {
        return Err(bad(name, v, "expected a non-negative duration"));
    }
    Ok(Duration::from_secs_f64(secs * mul))
}

pub fn parse_qos(name: &str, v: &str) -> FfiResult<QoS> {
    let n: u8 = parse_num(name, v)?;
    QoS::try_from(n).map_err(|_| FfiError::new(RSMQTT_BAD_QOS, format!("option '{name}': QoS must be 0, 1 or 2")))
}

pub fn parse_mqtt_version(name: &str, v: &str) -> FfiResult<Option<MqttVersion>> {
    match v.trim().to_ascii_lowercase().as_str() {
        "0" | "default" | "" => Ok(None),
        "3" | "3.1" | "v3.1" => Ok(Some(MqttVersion::V3_1)),
        "4" | "3.1.1" | "v3.1.1" | "v3" => Ok(Some(MqttVersion::V3_1_1)),
        "5" | "5.0" | "v5" => Ok(Some(MqttVersion::V5)),
        _ => Err(bad(name, v, "expected 0, 3, 4, 5, 3.1, 3.1.1 or 5")),
    }
}

fn parse_tls_version(name: &str, v: &str) -> FfiResult<mqtt::TlsVersion> {
    match v.trim().to_ascii_lowercase().as_str() {
        "0" | "default" => Ok(mqtt::TlsVersion::Default),
        "1" | "1.0" | "tls1.0" | "tlsv1" => Ok(mqtt::TlsVersion::Tls1_0),
        "2" | "1.1" | "tls1.1" | "tlsv1.1" => Ok(mqtt::TlsVersion::Tls1_1),
        "3" | "1.2" | "tls1.2" | "tlsv1.2" => Ok(mqtt::TlsVersion::Tls1_2),
        _ => Err(bad(name, v, "expected 0..3 or 1.0, 1.1, 1.2")),
    }
}

/// `key=value` or `key: value` → (key, value)
fn parse_pair(name: &str, v: &str) -> FfiResult<(String, String)> {
    let idx = v
        .find(['=', ':'])
        .ok_or_else(|| bad(name, v, "expected 'name=value'"))?;
    let (k, rest) = v.split_at(idx);
    Ok((k.trim().to_string(), rest[1..].trim().to_string()))
}

fn split_list(v: &str) -> Vec<String> {
    v.split([',', ' ', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn opt_string(v: &str) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

impl OptionsData {
    /// Sets a single option from its textual value.
    pub fn set(&mut self, name: &str, value: &str) -> FfiResult<()> {
        let key = normalize_name(name);
        let v = value;
        match key.as_str() {
            // ----- create -----
            "serveruri" => self.server_uri = opt_string(v.trim()),
            "clientid" => self.client_id = Some(v.to_string()),
            "mqttversion" => self.mqtt_version = parse_mqtt_version(name, v)?,
            "persistence" => {
                self.persistence = match v.trim().to_ascii_lowercase().as_str() {
                    "none" | "0" | "" => Persistence::None,
                    "memory" | "mem" => Persistence::Memory,
                    "file" | "default" | "1" => Persistence::File,
                    _ => return Err(bad(name, v, "expected none, memory or file")),
                };
                self.persistence_explicit = true;
            }
            "persistencedir" => {
                self.persistence_dir = opt_string(v.trim()).map(PathBuf::from);
                if !self.persistence_explicit {
                    self.persistence = Persistence::File;
                }
            }
            "sendwhiledisconnected" => self.send_while_disconnected = Some(parse_bool(name, v)?),
            "maxbufferedmessages" => self.max_buffered_messages = Some(parse_num(name, v)?),
            "allowdisconnectedsendatanytime" => {
                self.allow_disconnected_send_at_any_time = Some(parse_bool(name, v)?)
            }
            "deleteoldestmessages" => self.delete_oldest_messages = Some(parse_bool(name, v)?),
            "restoremessages" => self.restore_messages = Some(parse_bool(name, v)?),
            "persistqos0" => self.persist_qos0 = Some(parse_bool(name, v)?),
            "runtimethreads" => {
                let n: usize = parse_num(name, v)?;
                self.runtime_threads = if n == 0 { None } else { Some(n) };
            }
            "completionevents" => {
                self.completion_events = match v.trim().to_ascii_lowercase().as_str() {
                    "all" | "1" | "true" => CompletionEvents::All,
                    "errors" | "error" | "failures" => CompletionEvents::Errors,
                    "none" | "0" | "false" => CompletionEvents::None,
                    _ => return Err(bad(name, v, "expected all, errors or none")),
                }
            }
            "maxqueuedevents" => self.max_queued_events = parse_num(name, v)?,

            // ----- connect -----
            "keepaliveinterval" => self.keep_alive = Some(parse_duration(name, v)?),
            "cleansession" | "cleanstart" => self.clean = Some(parse_bool(name, v)?),
            "maxinflight" => self.max_inflight = Some(parse_num(name, v)?),
            "username" => self.username = opt_string(v),
            "password" => {
                self.password = if v.is_empty() {
                    None
                } else {
                    Some(v.as_bytes().to_vec())
                }
            }
            "connecttimeout" => self.connect_timeout = Some(parse_duration(name, v)?),
            "serveruris" => self.server_uris = split_list(v),
            "automaticreconnect" => self.automatic_reconnect = Some(parse_bool(name, v)?),
            "minretryinterval" => self.min_retry_interval = Some(parse_duration(name, v)?),
            "maxretryinterval" => self.max_retry_interval = Some(parse_duration(name, v)?),
            "httpproxy" => self.http_proxy = opt_string(v.trim()),
            "httpsproxy" => self.https_proxy = opt_string(v.trim()),
            "httpheader" => {
                let (k, val) = parse_pair(name, v)?;
                self.http_headers.push((k, val));
            }

            // ----- v5 connect properties -----
            "sessionexpiryinterval" => {
                let n = parse_num(name, v)?;
                set_prop(&mut self.connect_props, Property::SessionExpiryInterval(n))
            }
            "receivemaximum" => {
                let n = parse_num(name, v)?;
                set_prop(&mut self.connect_props, Property::ReceiveMaximum(n))
            }
            "maximumpacketsize" => {
                let n = parse_num(name, v)?;
                set_prop(&mut self.connect_props, Property::MaximumPacketSize(n))
            }
            "topicaliasmaximum" => {
                let n = parse_num(name, v)?;
                set_prop(&mut self.connect_props, Property::TopicAliasMaximum(n))
            }
            "requestresponseinformation" => {
                let b = parse_bool(name, v)? as u8;
                set_prop(&mut self.connect_props, Property::RequestResponseInformation(b))
            }
            "requestprobleminformation" => {
                let b = parse_bool(name, v)? as u8;
                set_prop(&mut self.connect_props, Property::RequestProblemInformation(b))
            }
            "userproperty" => {
                let (k, val) = parse_pair(name, v)?;
                self.connect_props.push_user(k, val);
            }

            // ----- will -----
            "will.topic" => self.will_topic = opt_string(v),
            "will.payload" | "will.message" => self.will_payload = v.as_bytes().to_vec(),
            "will.qos" => self.will_qos = Some(parse_qos(name, v)?),
            "will.retained" => self.will_retained = parse_bool(name, v)?,
            "will.delayinterval" => {
                let n = parse_num(name, v)?;
                set_prop(&mut self.will_props, Property::WillDelayInterval(n))
            }
            "will.messageexpiryinterval" => {
                let n = parse_num(name, v)?;
                set_prop(&mut self.will_props, Property::MessageExpiryInterval(n))
            }
            "will.contenttype" => set_prop(&mut self.will_props, Property::ContentType(v.to_string())),
            "will.responsetopic" => {
                set_prop(&mut self.will_props, Property::ResponseTopic(v.to_string()))
            }
            "will.payloadformatindicator" => {
                let b = parse_bool(name, v)? as u8;
                set_prop(&mut self.will_props, Property::PayloadFormatIndicator(b))
            }
            "will.userproperty" => {
                let (k, val) = parse_pair(name, v)?;
                self.will_props.push_user(k, val);
            }

            // ----- tls -----
            k if k.starts_with("ssl.") => {
                self.ssl.used = true;
                match k {
                    "ssl.truststore" => self.ssl.trust_store = opt_string(v.trim()),
                    "ssl.keystore" => self.ssl.key_store = opt_string(v.trim()),
                    "ssl.privatekey" => self.ssl.private_key = opt_string(v.trim()),
                    "ssl.privatekeypassword" => self.ssl.private_key_password = opt_string(v),
                    "ssl.enabledciphersuites" => self.ssl.enabled_cipher_suites = opt_string(v.trim()),
                    "ssl.enableservercertauth" => {
                        self.ssl.enable_server_cert_auth = Some(parse_bool(name, v)?)
                    }
                    "ssl.verify" => self.ssl.verify = Some(parse_bool(name, v)?),
                    "ssl.capath" => self.ssl.ca_path = opt_string(v.trim()),
                    "ssl.disabledefaulttruststore" => {
                        self.ssl.disable_default_trust_store = Some(parse_bool(name, v)?)
                    }
                    "ssl.sslversion" => self.ssl.version = Some(parse_tls_version(name, v)?),
                    "ssl.alpn" => self.ssl.alpn = split_list(v),
                    _ => return Err(unknown(name)),
                }
            }
            _ => return Err(unknown(name)),
        }
        Ok(())
    }

    /// Sets a binary option (`password`, `will.payload`, `ssl.alpn` in ALPN
    /// wire format like Paho C's `protos`).
    pub fn set_binary(&mut self, name: &str, data: &[u8]) -> FfiResult<()> {
        match normalize_name(name).as_str() {
            "password" => self.password = Some(data.to_vec()),
            "will.payload" | "will.message" => self.will_payload = data.to_vec(),
            "ssl.alpn" => {
                self.ssl.used = true;
                self.ssl.alpn = parse_alpn_wire(data)
                    .ok_or_else(|| FfiError::new(RSMQTT_BAD_MQTT_OPTION, "ssl.alpn: malformed ALPN protocol list"))?;
            }
            _ => {
                // Everything else must be text.
                let s = std::str::from_utf8(data).map_err(|_| {
                    FfiError::new(RSMQTT_BAD_UTF8_STRING, format!("option '{name}' is not valid UTF-8"))
                })?;
                return self.set(name, s);
            }
        }
        Ok(())
    }

    /// TRUE if `serverURI` or one of `serverURIs` is a TLS transport
    /// (ssl://, tls://, mqtts://, wss://): the connection gets TLS options.
    pub fn has_secure_uri(&self) -> bool {
        self.server_uri
            .iter()
            .chain(self.server_uris.iter())
            .any(|u| mqtt::is_secure_uri(u))
    }

    /// The effective MQTT version, or `None` for the library default (v5).
    pub fn version(&self) -> MqttVersion {
        self.mqtt_version.unwrap_or_default()
    }

    /// Builds the Paho create options.
    pub fn create_options(&self) -> FfiResult<mqtt::CreateOptions> {
        let uri = self
            .server_uri
            .clone()
            .or_else(|| self.server_uris.first().cloned())
            .ok_or_else(|| FfiError::new(RSMQTT_NULL_PARAMETER, "option 'serverURI' is required"))?;

        let mut b = mqtt::CreateOptionsBuilder::new()
            .server_uri(uri)
            .client_id(self.client_id.clone().unwrap_or_default())
            .mqtt_version(self.version());

        b = match &self.persistence {
            Persistence::None => b.persistence(mqtt::PersistenceType::None),
            Persistence::Memory => b.persistence(mqtt::MemPersistence::new()),
            Persistence::File => match &self.persistence_dir {
                None => b.persistence(mqtt::PersistenceType::File),
                Some(dir) => b.persistence(mqtt::PersistenceType::FilePath(dir.clone())),
            },
        };
        if let Some(v) = self.send_while_disconnected {
            b = b.send_while_disconnected(v);
        }
        if let Some(n) = self.max_buffered_messages {
            b = b.max_buffered_messages(n);
        }
        if let Some(v) = self.allow_disconnected_send_at_any_time {
            b = b.allow_disconnected_send_at_anytime(v);
        }
        if let Some(v) = self.delete_oldest_messages {
            b = b.delete_oldest_messages(v);
        }
        if let Some(v) = self.restore_messages {
            b = b.restore_messages(v);
        }
        if let Some(v) = self.persist_qos0 {
            b = b.persist_qos0(v);
        }
        // A C/C++ host has no async runtime of its own, so the library always
        // runs on its own shared runtime.
        b = b.shared_runtime(self.runtime_threads.unwrap_or(1));
        Ok(b.finalize())
    }

    /// Builds the Paho connect options.
    pub fn connect_options(&self) -> FfiResult<mqtt::ConnectOptions> {
        let ver = self.version();
        let ws = self
            .server_uri
            .as_deref()
            .map(|u| u.starts_with("ws://") || u.starts_with("wss://"))
            .unwrap_or(false);

        let mut b = mqtt::ConnectOptionsBuilder::with_mqtt_version(ver);
        if ws && self.keep_alive.is_none() {
            // Same longer default keep alive as ConnectOptions::new_ws().
            let ka = mqtt::ConnectOptions::new_ws().keep_alive_interval;
            b.keep_alive_interval(ka);
        }
        if let Some(ka) = self.keep_alive {
            b.keep_alive_interval(ka);
        }
        if let Some(clean) = self.clean {
            if ver == MqttVersion::V5 {
                b.clean_start(clean);
            } else {
                b.clean_session(clean);
            }
        }
        if let Some(n) = self.max_inflight {
            b.max_inflight(n);
        }
        if let Some(u) = &self.username {
            b.user_name(u.clone());
        }
        if let Some(p) = &self.password {
            b.password(p.clone());
        }
        if let Some(t) = self.connect_timeout {
            b.connect_timeout(t);
        }
        if !self.server_uris.is_empty() {
            b.server_uris(&self.server_uris);
        }
        if self.automatic_reconnect.unwrap_or(false) {
            b.automatic_reconnect(
                self.min_retry_interval.unwrap_or(Duration::from_secs(1)),
                self.max_retry_interval.unwrap_or(Duration::from_secs(60)),
            );
        }
        if let Some(p) = &self.http_proxy {
            b.http_proxy(p.clone());
        }
        if let Some(p) = &self.https_proxy {
            b.https_proxy(p.clone());
        }
        if !self.http_headers.is_empty() {
            let map: HashMap<String, String> = self.http_headers.iter().cloned().collect();
            b.http_headers(map);
        }
        if ver == MqttVersion::V5 && !self.connect_props.is_empty() {
            b.properties(self.connect_props.clone());
        }

        if let Some(topic) = &self.will_topic {
            let mut mb = mqtt::MessageBuilder::new()
                .topic(topic.clone())
                .payload(self.will_payload.clone())
                .qos(self.will_qos.unwrap_or(QoS::AtMostOnce))
                .retain(self.will_retained);
            if ver == MqttVersion::V5 && !self.will_props.is_empty() {
                mb = mb.properties(self.will_props.clone());
            }
            b.will_message(mb.finalize());
        } else if !self.will_payload.is_empty() {
            return Err(FfiError::new(
                RSMQTT_ZERO_LEN_WILL_TOPIC,
                "a will payload was given without 'will.topic'",
            ));
        }

        if self.ssl.used || self.has_secure_uri() {
            b.tls_options(self.tls_options());
        }

        let opts = b.finalize();
        opts.check().map_err(FfiError::from)?;
        Ok(opts)
    }

    fn tls_options(&self) -> mqtt::TlsOptions {
        let s = &self.ssl;
        let mut b = mqtt::TlsOptionsBuilder::new();
        if let Some(v) = &s.trust_store {
            b.trust_store(v);
        }
        if let Some(v) = &s.key_store {
            b.key_store(v);
        }
        if let Some(v) = &s.private_key {
            b.private_key(v);
        }
        if let Some(v) = &s.private_key_password {
            b.private_key_password(v.clone());
        }
        if let Some(v) = &s.enabled_cipher_suites {
            b.enabled_cipher_suites(v.clone());
        }
        if let Some(v) = s.enable_server_cert_auth {
            b.enable_server_cert_auth(v);
        }
        if let Some(v) = s.verify {
            b.verify(v);
        }
        if let Some(v) = &s.ca_path {
            b.ca_path(v);
        }
        if let Some(v) = s.disable_default_trust_store {
            b.disable_default_trust_store(v);
        }
        if let Some(v) = s.version {
            b.version(v);
        }
        if !s.alpn.is_empty() {
            let protos: Vec<&str> = s.alpn.iter().map(String::as_str).collect();
            b.alpn_protos(&protos);
        }
        b.finalize()
    }
}

fn unknown(name: &str) -> FfiError {
    FfiError::new(RSMQTT_UNKNOWN_OPTION, format!("unknown option '{name}'"))
}

/// Replaces a single-valued property (or appends it).
fn set_prop(props: &mut Properties, prop: Property) {
    props.remove(prop.property_id());
    props.push(prop);
}

/// ALPN wire format: a sequence of length-prefixed protocol names.
fn parse_alpn_wire(mut data: &[u8]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let n = data[0] as usize;
        if n == 0 || data.len() < 1 + n {
            return None;
        }
        out.push(String::from_utf8(data[1..1 + n].to_vec()).ok()?);
        data = &data[1 + n..];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized() {
        assert_eq!(normalize_name("keepAliveInterval"), "keepaliveinterval");
        assert_eq!(normalize_name("keep_alive_interval"), "keepaliveinterval");
        assert_eq!(normalize_name("TLS.trustStore"), "ssl.truststore");
        assert_eq!(normalize_name("ssl_trustStore"), "ssl.truststore");
        assert_eq!(normalize_name("will_topic"), "will.topic");
        assert_eq!(normalize_name("will-delay_interval"), "will.delayinterval");
        assert!(is_known_option("SSL.enableServerCertAuth"));
        assert!(!is_known_option("nonsense"));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("x", "30").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("x", "250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration("x", "1.5").unwrap(), Duration::from_millis(1500));
        assert_eq!(parse_duration("x", "2min").unwrap(), Duration::from_secs(120));
        assert!(parse_duration("x", "-1").is_err());
    }

    #[test]
    fn builds_options() {
        let mut o = OptionsData::default();
        o.set("serverURI", "tcp://localhost:1883").unwrap();
        o.set("clientId", "abc").unwrap();
        o.set("MQTTVersion", "4").unwrap();
        o.set("keepAliveInterval", "20").unwrap();
        o.set("cleansession", "0").unwrap();
        o.set("automaticReconnect", "1").unwrap();
        o.set("will.topic", "lwt").unwrap();
        o.set("will.payload", "gone").unwrap();
        o.set("will.qos", "1").unwrap();
        o.create_options().unwrap();
        let c = o.connect_options().unwrap();
        assert_eq!(c.keep_alive_interval, Duration::from_secs(20));
        assert!(!c.clean_session());
    }

    #[test]
    fn persistence_dir_never_overrides_an_explicit_type() {
        let kind = |opts: &[(&str, &str)]| {
            let mut o = OptionsData::default();
            for (n, v) in opts {
                o.set(n, v).unwrap();
            }
            (o.persistence, o.persistence_dir)
        };
        let dir = Some(PathBuf::from("d"));
        assert!(matches!(kind(&[("persistenceDir", "d")]), (Persistence::File, ref d) if *d == dir));
        assert!(matches!(kind(&[("persistence", "memory"), ("persistenceDir", "d")]), (Persistence::Memory, _)));
        assert!(matches!(kind(&[("persistenceDir", "d"), ("persistence", "memory")]), (Persistence::Memory, _)));
        assert!(matches!(kind(&[("persistence", "none"), ("persistenceDir", "d")]), (Persistence::None, _)));
        assert!(matches!(kind(&[("persistenceDir", "d"), ("persistence", "file")]), (Persistence::File, ref d) if *d == dir));
        assert!(matches!(kind(&[("persistence", "file")]), (Persistence::File, None)));
    }

    #[test]
    fn rejects_bad_values() {
        let mut o = OptionsData::default();
        assert_eq!(o.set("will.qos", "3").unwrap_err().rc, RSMQTT_BAD_QOS);
        assert_eq!(o.set("cleansession", "maybe").unwrap_err().rc, RSMQTT_BAD_MQTT_OPTION);
        assert_eq!(o.set("bogus", "1").unwrap_err().rc, RSMQTT_UNKNOWN_OPTION);
    }

    #[test]
    fn alpn_wire() {
        assert_eq!(
            parse_alpn_wire(b"\x04mqtt\x02h2").unwrap(),
            vec!["mqtt".to_string(), "h2".to_string()]
        );
        assert!(parse_alpn_wire(b"\x05mqtt").is_none());
    }
}
