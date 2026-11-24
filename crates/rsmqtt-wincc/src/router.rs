//! The addresses of one broker connection and how messages reach them.
//!
//! The driver registers every address of a DPE ([`Router::add`] /
//! [`Router::remove`]); the router keeps the subscriptions (filter, QoS,
//! flags), the `$state` addresses and the Sparkplug B host application of
//! the connection. A message ([`Router::message_into`]) or a change of the
//! connection ([`Router::state_into`], [`Router::connected_into`],
//! [`Router::connection_lost_into`]) gives an [`Outcome`]: the values for the
//! addresses, messages to publish, filters to subscribe and warnings. The
//! driver turns the values into HWObjects; the router never calls the
//! driver.
//!
//! Subscriptions of the driver's MSA service (RsMqttService, by id) share
//! the filters with the addresses ([`Router::add_service`] /
//! [`Router::remove_service`]); a message lists the ids whose filter
//! matches it in [`Outcome::service`].
//!
//! A message costs no allocations once the buffers have grown: the caller
//! keeps one `Outcome` and passes it to every call, the values go into one
//! byte buffer in it, and the addresses are C strings made once when they
//! are registered and shared by the deliveries.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::sync::Arc;

use crate::address::{self, Address};
use crate::{json, sparkplug};

/// What a value does to the invalid bit of the DPE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    /// leave it alone
    Keep = 0,
    /// clear it
    Valid = 1,
    /// set it
    Invalid = 2,
}

/// A value for an address.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    /// the full address (shared, NUL-terminated for the C ABI)
    pub address: Arc<CStr>,
    /// (offset, length) of the value in [`Outcome::data`]; None: the
    /// payload of the message as it is (see [`Outcome::value`])
    pub value: Option<(usize, usize)>,
    /// source time in ms since 1970; None: now
    pub time_ms: Option<i64>,
    /// the time is the value's own (ts= or Sparkplug), not the receive time
    pub periph_time: bool,
    pub validity: Validity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Publish {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: i32,
    pub retain: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    /// from the Sparkplug host application (else: JSON of an address)
    pub sparkplug: bool,
    pub text: String,
}

/// What a message or a change of the connection leads to. Reuse one for
/// all calls: every call clears it first and keeps the capacity.
#[derive(Debug, Default)]
pub struct Outcome {
    pub deliveries: Vec<Delivery>,
    pub publish: Vec<Publish>,
    /// (filter, QoS) to subscribe, apart from the address subscriptions
    pub subscribe: Vec<(String, i32)>,
    pub warnings: Vec<Warning>,
    /// the values of the deliveries
    pub data: Vec<u8>,
    /// the service subscriptions (ids) whose filter matches the message;
    /// empty for a copy of the previous message (see [`Copies`])
    pub service: Vec<u64>,
    /// the payload is UTF-8 text without NUL bytes (set when `service` is not empty)
    pub service_text: bool,
}

impl Outcome {
    pub fn clear(&mut self) {
        self.deliveries.clear();
        self.publish.clear();
        self.subscribe.clear();
        self.warnings.clear();
        self.data.clear();
        self.service.clear();
        self.service_text = false;
    }

    /// The value of a delivery (None: the payload of the message).
    pub fn value(&self, d: &Delivery) -> Option<&[u8]> {
        d.value.map(|(at, n)| &self.data[at..at + n])
    }

    /// The message payload as it is.
    fn payload(&mut self, address: &Arc<CStr>) {
        self.deliveries.push(Delivery { address: address.clone(), value: None, time_ms: None, periph_time: false, validity: Validity::Keep });
    }

    /// A value written into `data` by `write`.
    fn value_with(&mut self, address: &Arc<CStr>, time_ms: Option<i64>, validity: Validity, write: impl FnOnce(&mut Vec<u8>)) {
        let at = self.data.len();
        write(&mut self.data);
        let value = Some((at, self.data.len() - at));
        self.deliveries.push(Delivery { address: address.clone(), value, time_ms, periph_time: time_ms.is_some(), validity });
    }

    fn text(&mut self, address: &Arc<CStr>, text: &[u8], time_ms: Option<i64>, validity: Validity) {
        self.value_with(address, time_ms, validity, |d| d.extend_from_slice(text));
    }
}

/// A subscription: filter, QoS, RSMQTT_SUB_* flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub filter: String,
    pub qos: i32,
    pub flags: i32,
}

/// What [`Router::add`] asks the driver to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    Nothing,
    /// a `$state` address: write the current state to it
    State,
    /// subscribe this filter (again: new, or with a higher QoS / more flags)
    Subscribe(String),
}

struct Entry {
    addr: Address,
    /// the full address as a C string, shared by the deliveries
    name: Arc<CStr>,
    dyn_: bool,
    refs: i32,
}

fn c_name(full: &str) -> Arc<CStr> {
    let c = CString::new(full).unwrap_or_else(|_| CString::new(full.replace('\0', " ")).unwrap_or_default());
    Arc::from(c.into_boxed_c_str())
}

/// How a message is handed to one address.
struct Plan {
    address: String,
    name: Arc<CStr>,
    json: Option<JsonPlan>,
}

struct JsonPlan {
    dyn_: bool,
    /// extractor ids of json=<path> and ts=<path> (None: invalid path)
    value: Option<usize>,
    ts: Option<Option<usize>>,
    /// for messages
    path: String,
    ts_path: String,
}

#[derive(Default)]
struct Subscription {
    qos: i32,
    flags: i32,
    /// wildcard or shared filter: needs topic matching
    matching: bool,
    /// Sparkplug addresses on this edge node's filter: messages go to the
    /// host application (other addresses on the filter get the payload)
    sparkplug_refs: i32,
    /// full address -> reference count
    addresses: BTreeMap<String, i32>,
    /// service subscriptions (ids) on this filter
    service: Vec<u64>,
    /// built from `addresses` when they change, so that a message needs no
    /// address lookups and one pass over its JSON for all json= / ts= paths
    plan: Vec<Plan>,
    extractor: json::Extractor,
    results: Vec<json::ExResult>,
    plan_dirty: bool,
}

/// group, edge node, device ("" for the node), metric (None: `;online`)
type SpbKey = (String, String, String, Option<String>);

fn spb_key(a: &Address) -> SpbKey {
    let metric = if a.spb_online { None } else { Some(a.spb_metric.clone()) };
    (a.spb_group.clone(), a.spb_node.clone(), a.spb_device.clone(), metric)
}

impl Subscription {
    /// neither an address nor a service subscription uses the filter
    fn unused(&self) -> bool {
        self.addresses.is_empty() && self.service.is_empty()
    }
}

/// Overlapping filters (a/# and a/b) make brokers like mosquitto send a
/// message once per filter, back to back. The service subscriptions get it
/// once: identical messages (topic, payload, retained) that directly follow
/// within COPY_WINDOW_MS are skipped, up to the number of further filters
/// that matched. `$share/` filters are not counted (they may or may not get
/// a copy: a duplicate is better than a lost message). The time limit keeps
/// a value that is published again from being taken for a copy when the
/// broker sent fewer copies (one per client, a refused or no-local filter).
/// Addresses get every copy.
#[derive(Default)]
struct Copies {
    topic: String,
    payload: Vec<u8>,
    retained: bool,
    at_ms: i64,
    left: usize,
}

/// Copies of one publish arrive back to back; a later identical message is new.
const COPY_WINDOW_MS: i64 = 100;

impl Copies {
    /// A message that matched `filters` filters: true if it is a copy.
    fn check(&mut self, topic: &str, payload: &[u8], retained: bool, now_ms: i64, filters: usize, for_service: bool) -> bool {
        if self.left > 0
            && (0..=COPY_WINDOW_MS).contains(&(now_ms - self.at_ms))
            && self.retained == retained
            && self.topic == topic
            && self.payload == payload
        {
            self.left -= 1;
            return true;
        }
        self.left = if for_service { filters.saturating_sub(1) } else { 0 };
        if self.left > 0 {
            self.topic.clear();
            self.topic.push_str(topic);
            self.payload.clear();
            self.payload.extend_from_slice(payload);
            self.retained = retained;
            self.at_ms = now_ms;
        }
        false
    }
}

#[derive(Default)]
pub struct Router {
    addresses: BTreeMap<String, Entry>,
    subs: BTreeMap<String, Subscription>,
    matching: BTreeSet<String>,
    /// `$state` addresses -> reference count
    state: BTreeMap<String, i32>,
    spb: sparkplug::Host,
    /// Sparkplug input addresses -> reference count
    spb_index: BTreeMap<SpbKey, BTreeMap<String, i32>>,
    /// address -> last Sparkplug value, sent again as invalid when stale
    spb_last: HashMap<String, Vec<u8>>,
    /// addresses whose last JSON payload did not fit (warned once)
    json_warned: HashSet<String>,
    /// service subscription id -> filter
    service: HashMap<u64, String>,
    copies: Copies,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an address of a DPE (`input`: the DPE receives values).
    /// `dyn_` is set for dyn DPEs; `default_qos` is the connection's
    /// subscribe QoS for addresses without qos=.
    pub fn add(&mut self, full: &str, dyn_: bool, input: bool, default_qos: i32) -> Result<Added, String> {
        let addr = address::parse(full)?;
        let entry = self
            .addresses
            .entry(full.to_string())
            .or_insert_with(|| Entry { addr: Address::default(), name: c_name(full), dyn_, refs: 0 });
        entry.refs += 1;
        entry.addr = addr.clone();
        entry.dyn_ = dyn_;

        if addr.is_state {
            *self.state.entry(full.to_string()).or_default() += 1;
            return Ok(Added::State);
        }
        // Sparkplug outputs subscribe too: commands need the data types of the births
        if !input && !addr.sparkplug {
            return Ok(Added::Nothing);
        }

        let qos = if addr.qos >= 0 { addr.qos } else { default_qos };
        let is_new = !self.subs.contains_key(&addr.topic);
        let sub = self.subs.entry(addr.topic.clone()).or_default();
        let changed = is_new || qos > sub.qos || (addr.sub_flags & !sub.flags) != 0;
        if is_new || qos > sub.qos {
            sub.qos = qos;
        }
        sub.flags |= addr.sub_flags;
        sub.matching = address::is_wildcard_filter(&addr.topic);
        *sub.addresses.entry(full.to_string()).or_default() += 1;
        sub.plan_dirty = true;
        if addr.sparkplug {
            sub.sparkplug_refs += 1;
            if input {
                *self.spb_index.entry(spb_key(&addr)).or_default().entry(full.to_string()).or_default() += 1;
            }
        }
        if sub.matching {
            self.matching.insert(addr.topic.clone());
        }
        Ok(if changed { Added::Subscribe(addr.topic) } else { Added::Nothing })
    }

    /// Unregisters an address; returns the filter to unsubscribe when its
    /// last address is gone.
    pub fn remove(&mut self, full: &str, input: bool) -> Option<String> {
        let entry = self.addresses.get_mut(full)?;
        entry.refs -= 1;
        let addr = entry.addr.clone();
        if entry.refs <= 0 {
            self.addresses.remove(full);
        }

        if addr.is_state {
            if let Some(n) = self.state.get_mut(full) {
                *n -= 1;
                if *n <= 0 {
                    self.state.remove(full);
                }
            }
            return None;
        }
        if !input && !addr.sparkplug {
            return None;
        }
        if addr.sparkplug && input {
            let key = spb_key(&addr);
            if let Some(k) = self.spb_index.get_mut(&key) {
                if let Some(n) = k.get_mut(full) {
                    *n -= 1;
                    if *n <= 0 {
                        k.remove(full);
                        self.spb_last.remove(full);
                    }
                }
                if k.is_empty() {
                    self.spb_index.remove(&key);
                }
            }
        }

        let sub = self.subs.get_mut(&addr.topic)?;
        if let Some(n) = sub.addresses.get_mut(full) {
            if addr.sparkplug && sub.sparkplug_refs > 0 {
                sub.sparkplug_refs -= 1;
            }
            *n -= 1;
            if *n <= 0 {
                sub.addresses.remove(full);
            }
        }
        sub.plan_dirty = true;
        self.release(addr.topic)
    }

    /// Drops a subscription nothing uses any more; returns its filter to unsubscribe.
    fn release(&mut self, filter: String) -> Option<String> {
        if !self.subs.get(&filter)?.unused() {
            return None;
        }
        self.matching.remove(&filter);
        self.subs.remove(&filter);
        Some(filter)
    }

    /// Registers a subscription of the MSA service; returns the filter to
    /// subscribe, always: also for a filter that is subscribed already, the
    /// SUBSCRIBE makes the broker send its retained messages for the new
    /// subscription (the other users of the filter see them again).
    pub fn add_service(&mut self, id: u64, filter: &str, qos: i32) -> Result<String, String> {
        address::check_filter(filter)?;
        let is_new = !self.subs.contains_key(filter);
        let sub = self.subs.entry(filter.to_string()).or_default();
        if is_new || qos > sub.qos {
            sub.qos = qos;
        }
        if is_new {
            sub.matching = address::is_wildcard_filter(filter);
            if sub.matching {
                self.matching.insert(filter.to_string());
            }
        }
        sub.service.push(id);
        self.service.insert(id, filter.to_string());
        Ok(filter.to_string())
    }

    /// Unregisters a subscription of the MSA service; returns the filter to
    /// unsubscribe when nothing else uses it.
    pub fn remove_service(&mut self, id: u64) -> Option<String> {
        let filter = self.service.remove(&id)?;
        self.subs.get_mut(&filter)?.service.retain(|&s| s != id);
        self.release(filter)
    }

    /// A subscription by its filter.
    pub fn filter(&self, filter: &str) -> Option<Filter> {
        self.subs.get(filter).map(|s| Filter { filter: filter.to_string(), qos: s.qos, flags: s.flags })
    }

    /// All subscription filters.
    pub fn filters(&self) -> Vec<String> {
        self.subs.keys().cloned().collect()
    }

    /// Addresses that need a broker (subscriptions or `$state`).
    pub fn has_addresses(&self) -> bool {
        !self.subs.is_empty() || !self.state.is_empty()
    }

    /// The values of the `$state` addresses.
    pub fn state_into(&self, connected: bool, out: &mut Outcome) {
        out.clear();
        let v: &[u8] = if connected { b"1" } else { b"0" };
        for a in self.state.keys() {
            if let Some(e) = self.addresses.get(a) {
                out.text(&e.name, v, None, Validity::Keep);
            }
        }
    }

    /// One message. `state_host` / `state_ts`: the Sparkplug host id this
    /// client publishes STATE for ("" for none) and its timestamp.
    #[allow(clippy::too_many_arguments)]
    pub fn message_into(
        &mut self,
        topic: &str,
        payload: &[u8],
        retained: bool,
        now_ms: i64,
        state_host: &str,
        state_ts: i64,
        out: &mut Outcome,
    ) {
        out.clear();

        // our own STATE: another client (an earlier session's will) set it
        // offline; addresses on STATE topics still get the message below
        if !state_host.is_empty() && topic == sparkplug::state_topic(state_host) {
            let (found, v, _) = json::get(payload, &vec![json::Segment::Key("online".into())]);
            if found == json::Found::Yes && v.text == b"false" {
                out.publish.push(Publish {
                    topic: topic.to_string(),
                    payload: sparkplug::state_payload(true, state_ts).into_bytes(),
                    qos: 1,
                    retain: true,
                });
            }
        }

        let Router { addresses, subs, matching, spb, spb_index, spb_last, json_warned, copies, .. } = self;

        // filters that may get a copy of the message (see Copies)
        let mut filters = 0;

        // exact subscriptions
        if let Some(sub) = subs.get_mut(topic).filter(|s| !s.matching) {
            filters += 1;
            out.service.extend_from_slice(&sub.service);
            deliver_to(sub, addresses, json_warned, payload, out);
        }

        // wildcard and shared subscriptions
        let mut sparkplug_done = false;
        for f in matching.iter() {
            if !address::topic_matches(f, topic) {
                continue;
            }
            let Some(sub) = subs.get_mut(f) else { continue };
            if !f.starts_with("$share/") {
                filters += 1;
            }
            out.service.extend_from_slice(&sub.service);
            if !sparkplug_done && sub.sparkplug_refs > 0 {
                sparkplug_done = true; // once per message
                let o = spb.on_message(topic, payload, now_ms);
                out.warnings.extend(o.warnings.into_iter().map(|text| Warning { sparkplug: true, text }));
                // rebirth requests: QoS 0
                out.publish.extend(o.publish.into_iter().map(|m| Publish { topic: m.topic, payload: m.payload, qos: 0, retain: false }));
                apply(o.updates, spb_index, spb_last, addresses, out);
            }
            // other addresses on the filter (the plan skips Sparkplug ones)
            deliver_to(sub, addresses, json_warned, payload, out);
        }

        // the service subscriptions get one copy of each message
        if copies.check(topic, payload, retained, now_ms, filters, !out.service.is_empty()) {
            out.service.clear();
        } else if !out.service.is_empty() {
            out.service_text = !payload.contains(&0) && std::str::from_utf8(payload).is_ok();
        }
    }

    /// The connection is established: the STATE of the host application
    /// (subscription first, then the message, as Sparkplug 3.0 wants it),
    /// and edge nodes and devices count as offline until their birth.
    pub fn connected_into(&self, state_host: &str, state_ts: i64, out: &mut Outcome) {
        out.clear();
        if !state_host.is_empty() {
            let topic = sparkplug::state_topic(state_host);
            out.subscribe.push((topic.clone(), 1));
            out.publish.push(Publish {
                topic,
                payload: sparkplug::state_payload(true, state_ts).into_bytes(),
                qos: 1,
                retain: true,
            });
        }
        for ((group, node, device, metric), addrs) in &self.spb_index {
            if metric.is_none() && !self.spb.is_online(group, node, device) {
                for a in addrs.keys() {
                    if let Some(e) = self.addresses.get(a) {
                        out.text(&e.name, b"0", None, Validity::Valid);
                    }
                }
            }
        }
    }

    /// The connection is gone: every edge node and device goes offline, its
    /// metrics stale.
    pub fn connection_lost_into(&mut self, out: &mut Outcome) {
        out.clear();
        let updates = self.spb.on_connection_lost().updates;
        apply(updates, &self.spb_index, &mut self.spb_last, &self.addresses, out);
    }

    /// [`Router::state_into`] with a new Outcome.
    pub fn state(&self, connected: bool) -> Outcome {
        let mut out = Outcome::default();
        self.state_into(connected, &mut out);
        out
    }

    /// [`Router::message_into`] with a new Outcome.
    pub fn message(&mut self, topic: &str, payload: &[u8], now_ms: i64, state_host: &str, state_ts: i64) -> Outcome {
        let mut out = Outcome::default();
        self.message_into(topic, payload, false, now_ms, state_host, state_ts, &mut out);
        out
    }

    /// [`Router::connected_into`] with a new Outcome.
    pub fn connected(&self, state_host: &str, state_ts: i64) -> Outcome {
        let mut out = Outcome::default();
        self.connected_into(state_host, state_ts, &mut out);
        out
    }

    /// [`Router::connection_lost_into`] with a new Outcome.
    pub fn connection_lost(&mut self) -> Outcome {
        let mut out = Outcome::default();
        self.connection_lost_into(&mut out);
        out
    }

    /// The message that writes a value to an output address.
    /// - `value`: the payload text from the transformation, of data type
    ///   `value_type` (values::types); with json= numbers, bool and dyn
    ///   (a JSON array) are JSON as they are, text and blobs become strings;
    /// - `time_ms`: the value's time, for ts=;
    /// - `publish_qos` / `publish_retain`: the connection's defaults.
    ///
    /// Sparkplug metrics are written as NCMD / DCMD with the data type of
    /// their birth (QoS 0, not retained).
    #[allow(clippy::too_many_arguments)] // mirrors the C ABI call
    pub fn write(
        &self,
        full: &str,
        value: &[u8],
        value_type: i32,
        time_ms: i64,
        now_ms: i64,
        publish_qos: i32,
        publish_retain: bool,
    ) -> Result<Publish, String> {
        let e = self.addresses.get(full).ok_or_else(|| format!("no active MQTT connection for address '{full}'"))?;
        let a = &e.addr;
        if a.sparkplug && a.spb_online {
            return Err(format!("address '{full}': cannot be written"));
        }
        // (a Sparkplug metric's topic is its edge node's filter; it is written as NCMD / DCMD)
        if a.is_state || (address::is_wildcard_filter(&a.topic) && !(a.sparkplug && !a.spb_online)) {
            return Err(format!("address '{full}' cannot be written"));
        }
        if a.sparkplug {
            let m = self
                .spb
                .command(&a.spb_group, &a.spb_node, &a.spb_device, &a.spb_metric, value, now_ms)
                .map_err(|err| format!("address '{full}': {err}"))?;
            return Ok(Publish { topic: m.topic, payload: m.payload, qos: 0, retain: false });
        }

        let payload = if a.json {
            use crate::values::types::{BOOL, FLOAT, INT32};
            let plain = e.dyn_ || (INT32..=FLOAT).contains(&value_type) || value_type == BOOL;
            let v = if plain { value.to_vec() } else { json::quote(value) };
            let path = json::parse_path(&a.json_path).map_err(|err| format!("address '{full}': {err}"))?;
            let mut members = vec![(path, v)];
            if a.has_ts {
                let ts = json::parse_path(&a.ts_path).map_err(|err| format!("address '{full}': {err}"))?;
                members.push((ts, json::quote(json::iso_time(time_ms).as_bytes())));
            }
            let refs: Vec<(&json::Path, &[u8])> = members.iter().map(|(p, v)| (p, v.as_slice())).collect();
            json::build(&refs).map_err(|err| format!("address '{full}': {err}"))?
        } else {
            value.to_vec()
        };
        Ok(Publish {
            topic: a.topic.clone(),
            payload,
            qos: if a.qos >= 0 { a.qos } else { publish_qos },
            retain: if a.retain >= 0 { a.retain != 0 } else { publish_retain },
        })
    }
}

/// Sparkplug updates to the input addresses of their metrics / online state.
fn apply(
    updates: Vec<sparkplug::Update>,
    spb_index: &BTreeMap<SpbKey, BTreeMap<String, i32>>,
    spb_last: &mut HashMap<String, Vec<u8>>,
    addresses: &BTreeMap<String, Entry>,
    out: &mut Outcome,
) {
    use sparkplug::UpdateKind;
    for u in updates {
        let metric = if u.kind == UpdateKind::Online { None } else { Some(u.metric) };
        let key = (u.group, u.node, u.device, metric);
        let Some(addrs) = spb_index.get(&key) else { continue };
        for a in addrs.keys() {
            let Some(e) = addresses.get(a) else { continue };
            match u.kind {
                UpdateKind::Value => {
                    // dyn: a string stays one item, even if it looks like an array
                    let text = if e.dyn_ && u.is_string { json::quote(&u.text) } else { u.text.clone() };
                    out.text(&e.name, &text, u.time_ms, Validity::Valid);
                    spb_last.insert(a.clone(), text);
                }
                UpdateKind::Stale => {
                    // the last value again, invalid (nothing to send before the first one)
                    if let Some(last) = spb_last.get(a) {
                        out.text(&e.name, last, None, Validity::Invalid);
                    }
                }
                UpdateKind::Online => out.text(&e.name, if u.online { b"1" } else { b"0" }, None, Validity::Valid),
            }
        }
    }
}

/// The (non-Sparkplug) addresses of one subscription.
fn deliver_to(
    sub: &mut Subscription,
    addresses: &BTreeMap<String, Entry>,
    json_warned: &mut HashSet<String>,
    payload: &[u8],
    out: &mut Outcome,
) {
    if sub.plan_dirty {
        build_plan(sub, addresses);
        // addresses changed: a new or changed configuration warns again
        json_warned.clear();
    }

    // one pass over the payload for all paths of this subscription
    let err = if sub.extractor.is_empty() { String::new() } else { sub.extractor.run(payload, &mut sub.results) };

    for d in &sub.plan {
        let Some(j) = &d.json else {
            out.payload(&d.name);
            continue;
        };
        let Some(r) = j.value.and_then(|id| sub.results.get(id)) else {
            warn_once(json_warned, &d.address, || format!("invalid JSON path {}", j.path), out);
            continue;
        };
        match r.found {
            json::Found::Missing => {
                warn_once(json_warned, &d.address, || format!("the JSON payload has no value at {}", j.path), out);
                continue;
            }
            json::Found::NotJson => {
                warn_once(json_warned, &d.address, || format!("the payload is not JSON: {err}"), out);
                continue;
            }
            json::Found::Yes => {}
        }
        if r.value.kind == json::Kind::Null {
            continue; // no value, nothing to update
        }

        let mut time_ms = None;
        if let Some(ts) = j.ts {
            let t = ts.and_then(|id| sub.results.get(id)).filter(|t| t.found == json::Found::Yes).and_then(|t| json::parse_time(&t.value));
            match t {
                Some(ms) => time_ms = Some(ms),
                None => warn_once(json_warned, &d.address, || format!("no valid timestamp at {}, the current time is used", j.ts_path), out),
            }
        }
        if time_ms.is_some() || j.ts.is_none() {
            // fits: a later problem warns again
            if !json_warned.is_empty() {
                json_warned.remove(&d.address);
            }
        }
        // dyn: the transformation reads a JSON array or one value; a string
        // stays a JSON string so that its text is one item, even if it looks like an array
        let quote = j.dyn_ && r.value.kind == json::Kind::String;
        out.value_with(&d.name, time_ms, Validity::Keep, |data| {
            if quote {
                json::quote_into(&r.value.text, data)
            } else {
                data.extend_from_slice(&r.value.text)
            }
        });
    }
}

/// A JSON problem of an address, once until a payload fits again: a topic
/// may carry many messages.
/// The text is only made when the warning is new.
fn warn_once(warned: &mut HashSet<String>, address: &str, text: impl FnOnce() -> String, out: &mut Outcome) {
    if !warned.contains(address) {
        warned.insert(address.to_string());
        let text = text();
        out.warnings.push(Warning { sparkplug: false, text: format!("address '{address}': {text} (logged once until it works again)") });
    }
}

fn build_plan(sub: &mut Subscription, addresses: &BTreeMap<String, Entry>) {
    sub.plan.clear();
    sub.extractor.clear();
    let add = |path: &str, x: &mut json::Extractor| match json::parse_path(path) {
        Ok(p) => (Some(x.add(&p)), json::path_to_string(&p)),
        Err(_) => (None, path.to_string()),
    };
    for a in sub.addresses.keys() {
        let Some(e) = addresses.get(a) else { continue };
        if e.addr.sparkplug {
            continue; // decoded by the Sparkplug host application
        }
        let json = e.addr.json.then(|| {
            let (value, path) = add(&e.addr.json_path, &mut sub.extractor);
            let (ts, ts_path) = if e.addr.has_ts {
                let (id, p) = add(&e.addr.ts_path, &mut sub.extractor);
                (Some(id), p)
            } else {
                (None, String::new())
            };
            JsonPlan { dyn_: e.dyn_, value, ts, path, ts_path }
        });
        sub.plan.push(Plan { address: a.clone(), name: e.name.clone(), json });
    }
    sub.plan_dirty = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use srad_types::payload::{Message, Metric, Payload};

    fn values(o: &Outcome) -> Vec<(String, String)> {
        o.deliveries
            .iter()
            .map(|d| {
                let v = o.value(d).map_or("<payload>".into(), |v| String::from_utf8_lossy(v).into_owned());
                (d.address.to_str().unwrap().to_string(), v)
            })
            .collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn service_subscriptions() {
        let mut r = Router::new();
        assert!(r.add_service(1, "a/#/b", 1).is_err());
        assert_eq!(r.add_service(1, "a/#", 1).unwrap(), "a/#");
        // a filter in use is subscribed again too: the broker sends the retained messages
        assert_eq!(r.add_service(2, "a/#", 0).unwrap(), "a/#");
        assert_eq!(r.filter("a/#").unwrap().qos, 1);
        // shared with an address: removing one keeps the filter
        assert_eq!(r.add("p:a/b", false, true, 1).unwrap(), Added::Subscribe("a/b".into()));
        assert_eq!(r.add_service(3, "a/b", 2).unwrap(), "a/b");
        assert_eq!(r.remove("p:a/b", true), None);
        assert_eq!(r.filter("a/b").unwrap().qos, 2);

        let mut out = Outcome::default();
        let at = |r: &mut Router, topic: &str, payload: &[u8], retained: bool, now: i64, out: &mut Outcome| {
            r.message_into(topic, payload, retained, now, "", 0, out);
            let mut v = out.service.clone();
            v.sort();
            v
        };
        let ids = |r: &mut Router, topic: &str, payload: &[u8], out: &mut Outcome| {
            r.message_into(topic, payload, false, 0, "", 0, out);
            let mut v = out.service.clone();
            v.sort();
            v
        };
        // a/b matches a/# and a/b: the broker sends it twice, the service gets it once
        assert_eq!(ids(&mut r, "a/b", b"x", &mut out), vec![1, 2, 3]);
        assert!(out.service_text);
        assert_eq!(ids(&mut r, "a/b", b"x", &mut out), Vec::<u64>::new());
        // the same payload published again is a new message (and its copy)
        assert_eq!(ids(&mut r, "a/b", b"x", &mut out), vec![1, 2, 3]);
        assert_eq!(ids(&mut r, "a/b", b"x", &mut out), Vec::<u64>::new());
        // one filter: no copies; NUL / not UTF-8 is no text
        assert_eq!(ids(&mut r, "a/c", b"x\0y", &mut out), vec![1, 2]);
        assert!(!out.service_text);
        assert_eq!(ids(&mut r, "a/c", b"x\0y", &mut out), vec![1, 2]);
        assert_eq!(ids(&mut r, "a/c", &[0xff], &mut out), vec![1, 2]);
        assert!(!out.service_text);
        // another message ends the copies of the previous one
        assert_eq!(ids(&mut r, "a/b", b"y", &mut out), vec![1, 2, 3]);
        assert_eq!(ids(&mut r, "a/c", b"z", &mut out), vec![1, 2]);
        assert_eq!(ids(&mut r, "a/b", b"y", &mut out), vec![1, 2, 3]);
        assert_eq!(ids(&mut r, "b", b"y", &mut out), Vec::<u64>::new());
        // copies follow within COPY_WINDOW_MS; the same value later is a new
        // message (a broker that sends one copy per client)
        assert_eq!(at(&mut r, "a/b", b"v", false, 1000, &mut out), vec![1, 2, 3]);
        assert_eq!(at(&mut r, "a/b", b"v", false, 1000 + COPY_WINDOW_MS + 1, &mut out), vec![1, 2, 3]);
        // a retained message is no copy of a live one
        assert_eq!(at(&mut r, "a/b", b"w", false, 3000, &mut out), vec![1, 2, 3]);
        assert_eq!(at(&mut r, "a/b", b"w", true, 3001, &mut out), vec![1, 2, 3]);
        assert_eq!(at(&mut r, "a/b", b"w", true, 3002, &mut out), Vec::<u64>::new());

        assert_eq!(r.remove_service(1), None);
        assert_eq!(r.remove_service(2), Some("a/#".into()));
        assert_eq!(r.remove_service(3), Some("a/b".into()));
        assert_eq!(r.remove_service(3), None);
        assert!(!r.has_addresses());
    }

    #[test]
    fn subscriptions() {
        let mut r = Router::new();
        assert_eq!(r.add("p:a/b", false, true, 1).unwrap(), Added::Subscribe("a/b".into()));
        assert_eq!(r.filter("a/b").unwrap(), Filter { filter: "a/b".into(), qos: 1, flags: 0 });
        // same filter, same QoS: nothing to do; higher QoS or new flags: subscribe again
        assert_eq!(r.add("p:a/b;qos=0", false, true, 1).unwrap(), Added::Nothing);
        assert_eq!(r.add("p:a/b;qos=2", false, true, 1).unwrap(), Added::Subscribe("a/b".into()));
        assert_eq!(r.add("p:a/b;nolocal", false, true, 1).unwrap(), Added::Subscribe("a/b".into()));
        assert_eq!(r.filter("a/b").unwrap().qos, 2);
        // outputs do not subscribe, $state neither
        assert_eq!(r.add("p:out", false, false, 1).unwrap(), Added::Nothing);
        assert_eq!(r.add("p:$state", false, true, 1).unwrap(), Added::State);
        assert!(r.add("p:a;qos=7", false, true, 1).is_err());
        assert_eq!(r.filters(), ["a/b"]);

        // reference counts: the filter goes with its last address
        assert_eq!(r.add("p:a/b", false, true, 1).unwrap(), Added::Nothing);
        for full in ["p:a/b", "p:a/b;qos=0", "p:a/b;qos=2"] {
            assert_eq!(r.remove(full, true), None);
        }
        assert_eq!(r.remove("p:a/b;nolocal", true), None);
        assert_eq!(r.remove("p:a/b", true), Some("a/b".into()));
        assert_eq!(r.remove("p:a/b", true), None);
        assert!(r.has_addresses());
        assert_eq!(r.remove("p:$state", true), None);
        assert_eq!(r.remove("p:out", false), None);
        assert!(!r.has_addresses());
    }

    #[test]
    fn messages() {
        let mut r = Router::new();
        r.add("p:s/1", false, true, 1).unwrap();
        r.add("p:s/+", false, true, 1).unwrap();
        r.add("p:s/1;json=data.t;ts=data.time", false, true, 1).unwrap();
        r.add("p:s/1;json=list", true, true, 1).unwrap();
        r.add("p:s/1;json=name", true, true, 1).unwrap();
        r.add("p:$state", false, true, 1).unwrap();

        let o = r.message("s/1", br#"{"data":{"t":21.5,"time":1700000000000},"list":[1,2],"name":"[x]"}"#, 0, "", 0);
        assert_eq!(
            values(&o),
            pairs(&[
                ("p:s/1", "<payload>"),
                ("p:s/1;json=data.t;ts=data.time", "21.5"),
                ("p:s/1;json=list", "[1,2]"),
                ("p:s/1;json=name", r#""[x]""#),
                ("p:s/+", "<payload>"),
            ])
        );
        let ts = &o.deliveries[1];
        assert_eq!((ts.time_ms, ts.periph_time), (Some(1700000000000), true));
        assert!(o.warnings.is_empty());

        // missing values warn once per address until a payload fits again
        let o = r.message("s/1", br#"{"data":{"t":1}}"#, 0, "", 0);
        assert_eq!(values(&o), pairs(&[("p:s/1", "<payload>"), ("p:s/1;json=data.t;ts=data.time", "1"), ("p:s/+", "<payload>")]));
        assert_eq!((o.deliveries[1].time_ms, o.deliveries[1].periph_time), (None, false));
        let texts: Vec<_> = o.warnings.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "address 'p:s/1;json=data.t;ts=data.time': no valid timestamp at data.time, the current time is used (logged once until it works again)",
                "address 'p:s/1;json=list': the JSON payload has no value at list (logged once until it works again)",
                "address 'p:s/1;json=name': the JSON payload has no value at name (logged once until it works again)",
            ]
        );
        assert!(r.message("s/1", br#"{"data":{"t":1}}"#, 0, "", 0).warnings.is_empty());
        // all three are still warned; a fitting payload clears the warning
        assert!(r.message("s/1", b"not json", 0, "", 0).warnings.is_empty());
        r.message("s/1", br#"{"list":[]}"#, 0, "", 0);
        let o = r.message("s/1", b"not json", 0, "", 0);
        assert_eq!(o.warnings.len(), 1);
        assert!(o.warnings[0].text.starts_with("address 'p:s/1;json=list': the payload is not JSON: "));
        // null: nothing to update
        assert_eq!(values(&r.message("s/1", br#"{"data":{"t":null}}"#, 0, "", 0)).len(), 2);

        assert_eq!(values(&r.state(true)), pairs(&[("p:$state", "1")]));
        assert_eq!(values(&r.state(false)), pairs(&[("p:$state", "0")]));
        assert!(r.message("other", b"x", 0, "", 0).deliveries.is_empty());
    }

    #[test]
    fn own_state() {
        let mut r = Router::new();
        let o = r.message("spBv1.0/STATE/scada", br#"{"online":false,"timestamp":1}"#, 0, "scada", 42);
        assert_eq!(o.publish, [Publish { topic: "spBv1.0/STATE/scada".into(), payload: br#"{"online":true,"timestamp":42}"#.to_vec(), qos: 1, retain: true }]);
        assert!(r.message("spBv1.0/STATE/scada", br#"{"online":true}"#, 0, "scada", 42).publish.is_empty());
        assert!(r.message("spBv1.0/STATE/other", br#"{"online":false}"#, 0, "scada", 42).publish.is_empty());
        let o = r.connected("scada", 42);
        assert_eq!(o.subscribe, [("spBv1.0/STATE/scada".to_string(), 1)]);
        assert_eq!(o.publish.len(), 1);
        assert!(r.connected("", 42).publish.is_empty());
    }

    #[test]
    fn writes() {
        use crate::values::types::{BLOB, FLOAT, STRING};
        let mut r = Router::new();
        for a in ["p:out", "p:out;qos=2;retain=0", "p:j;json=data.v;ts=data.t", "p:j2;json", "p:$state", "p:in/#"] {
            r.add(a, false, false, 1).unwrap();
        }
        r.add("p:d;json=v", true, false, 1).unwrap();
        r.add("p:$spb/G/N;metric=m", false, false, 1).unwrap();

        let p = r.write("p:out", b"21.5", FLOAT, 0, 0, 1, true).unwrap();
        assert_eq!(p, Publish { topic: "out".into(), payload: b"21.5".to_vec(), qos: 1, retain: true });
        let p = r.write("p:out;qos=2;retain=0", b"x", STRING, 0, 0, 1, true).unwrap();
        assert_eq!((p.qos, p.retain), (2, false));

        let p = r.write("p:j;json=data.v;ts=data.t", b"21.5", FLOAT, 1700000000123, 0, 0, false).unwrap();
        assert_eq!(String::from_utf8(p.payload).unwrap(), r#"{"data":{"v":21.5,"t":"2023-11-14T22:13:20.123Z"}}"#);
        let p = r.write("p:j2;json", b"a\"b", STRING, 0, 0, 0, false).unwrap();
        assert_eq!(p.payload, br#""a\"b""#);
        assert_eq!(r.write("p:j2;json", b"\x01", BLOB, 0, 0, 0, false).unwrap().payload, br#""\u0001""#);
        // dyn: the transformation's JSON array as it is
        assert_eq!(r.write("p:d;json=v", b"[\"a\",1]", STRING, 0, 0, 0, false).unwrap().payload, br#"{"v":["a",1]}"#);

        for (a, msg) in [
            ("p:$state", "address 'p:$state' cannot be written"),
            ("p:in/#", "address 'p:in/#' cannot be written"),
            ("p:none", "no active MQTT connection for address 'p:none'"),
            ("p:$spb/G/N;metric=m", "address 'p:$spb/G/N;metric=m': G/N is offline (no birth received)"),
        ] {
            assert_eq!(r.write(a, b"1", FLOAT, 0, 0, 1, false).unwrap_err(), msg);
        }
    }

    // counts the allocations of the current thread (the whole test binary
    // uses it; other tests only pay a thread-local increment)
    mod counting {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local!(static COUNT: Cell<usize> = const { Cell::new(0) });

        pub struct Counting;

        unsafe impl GlobalAlloc for Counting {
            unsafe fn alloc(&self, l: Layout) -> *mut u8 {
                let _ = COUNT.try_with(|c| c.set(c.get() + 1));
                System.alloc(l)
            }
            unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
                System.dealloc(p, l)
            }
            unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
                let _ = COUNT.try_with(|c| c.set(c.get() + 1));
                System.realloc(p, l, n)
            }
        }

        #[global_allocator]
        static ALLOC: Counting = Counting;

        pub fn allocations() -> usize {
            COUNT.with(|c| c.get())
        }
    }

    // a message costs no allocations once the buffers have grown
    #[test]
    fn messages_do_not_allocate() {
        let mut r = Router::new();
        for a in ["p:s/+", "p:s/1", "p:s/1;json=data.v;ts=data.t", "p:s/1;json=data.n", "p:s/1;json=data.list"] {
            r.add(a, a.ends_with("list"), true, 1).unwrap();
        }
        // payloads that fit the addresses (alternating misfits log new warnings)
        let payloads: [&[u8]; 3] = [
            br#"{"data":{"v":21.5,"t":1700000000000,"n":"text \"quoted\" \u00e9","list":[1,2,3]},"x":{"y":[{"z":1}]}}"#,
            br#"{"data":{"v":-3,"t":"2023-11-14T22:13:20.123Z","n":"other","list":"one item"}}"#,
            br#"{"data":{"v":7,"t":1700000000001,"n":"","list":[]}}"#,
        ];
        let mut out = Outcome::default();
        let run = |r: &mut Router, out: &mut Outcome| {
            for p in payloads {
                r.message_into("s/1", p, false, 0, "", 0, out);
                assert!(out.deliveries.len() >= 2);
            }
        };
        for _ in 0..3 {
            run(&mut r, &mut out); // warm-up: plans, buffers
        }
        let before = counting::allocations();
        for _ in 0..100 {
            run(&mut r, &mut out);
        }
        assert_eq!(counting::allocations() - before, 0, "allocations per 300 messages");

        // a payload that keeps not fitting: warned once, then nothing either
        r.message_into("s/1", b"42", false, 0, "", 0, &mut out);
        assert!(!out.warnings.is_empty());
        let before = counting::allocations();
        for _ in 0..100 {
            r.message_into("s/1", b"42", false, 0, "", 0, &mut out);
            r.message_into("s/1", b"{\"data\":", false, 0, "", 0, &mut out);
        }
        assert!(out.warnings.is_empty());
        let not_json = counting::allocations() - before;
        // (the error text of a document that is not JSON is made per message)
        assert!(not_json <= 100 * 2, "allocations for 200 misfits: {not_json}");

        // the same through the C ABI the driver calls
        unsafe {
            use crate::ffi::*;
            let h = rsmqtt_wincc_router_new();
            for a in [c"p:s/+", c"p:s/1;json=data.v"] {
                let mut f = RsmqttWinccBuf { data: std::ptr::null_mut(), len: 0 };
                rsmqtt_wincc_router_add(h, a.as_ptr(), 0, 1, 1, &mut f);
                rsmqtt_wincc_buf_free(f);
            }
            let p = payloads[0];
            let mut d = std::mem::zeroed::<RsmqttWinccDelivery>();
            let mut call = || {
                rsmqtt_wincc_router_message(h, c"s/1".as_ptr(), p.as_ptr() as _, p.len(), 0, c"".as_ptr(), 0, 0);
                for i in 0..rsmqtt_wincc_router_delivery_count(h) {
                    rsmqtt_wincc_router_delivery_get(h, i, &mut d);
                }
            };
            for _ in 0..3 {
                call();
            }
            let before = counting::allocations();
            for _ in 0..100 {
                call();
            }
            assert_eq!(counting::allocations() - before, 0, "allocations per 100 messages through the C ABI");
            assert_eq!(rsmqtt_wincc_router_delivery_count(h), 2);
            rsmqtt_wincc_router_free(h);
        }
    }

    fn birth(metric: &str, t: u32, value: &str) -> Vec<u8> {
        let mut m = Metric::new();
        m.name = Some(metric.into());
        m.datatype = Some(t);
        sparkplug::set_value(&mut m, t, value.as_bytes()).unwrap();
        Payload { timestamp: Some(1700000000000), metrics: vec![m], seq: Some(0), uuid: None, body: None }.encode_to_vec()
    }

    #[test]
    fn sparkplug() {
        use sparkplug::types::{DOUBLE, STRING};
        let mut r = Router::new();
        let filter = "spBv1.0/G/+/N/#";
        assert_eq!(r.add("p:$spb/G/N;metric=temp", false, true, 1).unwrap(), Added::Subscribe(filter.into()));
        r.add("p:$spb/G/N;metric=name", true, true, 1).unwrap();
        r.add("p:$spb/G/N;online", false, true, 1).unwrap();
        assert_eq!(r.add("p:$spb/G/N;metric=temp;qos=1", false, false, 1).unwrap(), Added::Nothing); // command
        r.add(&format!("p:{filter}"), false, true, 1).unwrap(); // raw payloads on the same filter

        let o = r.connected("", 0);
        assert_eq!(values(&o), pairs(&[("p:$spb/G/N;online", "0")]));

        let o = r.message("spBv1.0/G/NBIRTH/N", &birth("temp", DOUBLE, "21.5"), 0, "", 0);
        assert_eq!(
            values(&o),
            pairs(&[("p:$spb/G/N;metric=temp", "21.5"), ("p:$spb/G/N;online", "1"), (&format!("p:{filter}"), "<payload>")])
        );
        assert_eq!((o.deliveries[0].time_ms, o.deliveries[0].periph_time, o.deliveries[0].validity), (Some(1700000000000), true, Validity::Valid));
        assert!(r.connected("", 0).deliveries.is_empty());
        let cmd = r.write("p:$spb/G/N;metric=temp;qos=1", b"3", 1006, 0, 5, 1, true).unwrap();
        assert_eq!((cmd.topic.as_str(), cmd.qos, cmd.retain), ("spBv1.0/G/NCMD/N", 0, false));
        assert_eq!(r.write("p:$spb/G/N;online", b"1", 1007, 0, 5, 1, true).unwrap_err(), "address 'p:$spb/G/N;online': cannot be written");

        // a string metric of a dyn DPE stays one item
        let mut b = birth("name", STRING, "[a]");
        let mut p = Payload::decode(&b[..]).unwrap();
        p.metrics.push(Metric { name: Some("temp".into()), datatype: Some(DOUBLE), ..p.metrics[0].clone() });
        sparkplug::set_value(&mut p.metrics[1], DOUBLE, b"1").unwrap();
        b = p.encode_to_vec();
        let o = r.message("spBv1.0/G/NBIRTH/N", &b, 0, "", 0);
        assert!(values(&o).contains(&("p:$spb/G/N;metric=name".to_string(), r#""[a]""#.to_string())));

        let o = r.connection_lost();
        assert_eq!(values(&o), pairs(&[("p:$spb/G/N;metric=name", r#""[a]""#), ("p:$spb/G/N;metric=temp", "1"), ("p:$spb/G/N;online", "0")]));
        assert_eq!(o.deliveries[0].validity, Validity::Invalid);

        // the last Sparkplug input goes, the filter stays for the command and the raw address
        assert_eq!(r.remove("p:$spb/G/N;metric=temp", true), None);
        assert!(!r.spb_last.contains_key("p:$spb/G/N;metric=temp"));
        r.remove("p:$spb/G/N;metric=name", true);
        r.remove("p:$spb/G/N;online", true);
        assert!(r.spb_index.is_empty());
        assert_eq!(r.remove(&format!("p:{filter}"), true), None);
        assert_eq!(r.remove("p:$spb/G/N;metric=temp;qos=1", false), Some(filter.into()));
    }
}
