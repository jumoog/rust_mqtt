//! The addresses of one broker connection and how messages reach them.
//!
//! The driver registers every address of a DPE ([`Router::add`] /
//! [`Router::remove`]); the router keeps the subscriptions (filter, QoS,
//! flags), the `$state` addresses and the Sparkplug B host application of
//! the connection. A message ([`Router::message`]) or a change of the
//! connection ([`Router::state`], [`Router::connected`],
//! [`Router::connection_lost`]) gives an [`Outcome`]: the values for the
//! addresses, messages to publish, filters to subscribe and warnings. The
//! driver turns the values into HWObjects; the router never calls the
//! driver.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
    pub address: String,
    /// None: the payload of the message as it is
    pub value: Option<Vec<u8>>,
    /// source time in ms since 1970; None: now
    pub time_ms: Option<i64>,
    /// the time is the value's own (ts= or Sparkplug), not the receive time
    pub periph_time: bool,
    pub validity: Validity,
}

impl Delivery {
    fn text(address: &str, value: &[u8], validity: Validity) -> Self {
        Delivery { address: address.to_string(), value: Some(value.to_vec()), time_ms: None, periph_time: false, validity }
    }
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

/// What a message or a change of the connection leads to.
#[derive(Debug, Default)]
pub struct Outcome {
    pub deliveries: Vec<Delivery>,
    pub publish: Vec<Publish>,
    /// (filter, QoS) to subscribe, apart from the address subscriptions
    pub subscribe: Vec<(String, i32)>,
    pub warnings: Vec<Warning>,
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
    dyn_: bool,
    refs: i32,
}

/// How a message is handed to one address.
struct Plan {
    address: String,
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
        let entry = self.addresses.entry(full.to_string()).or_insert(Entry { addr: Address::default(), dyn_, refs: 0 });
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
        if !sub.addresses.is_empty() {
            return None;
        }
        self.matching.remove(&addr.topic);
        self.subs.remove(&addr.topic);
        Some(addr.topic)
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
    pub fn state(&self, connected: bool) -> Outcome {
        let v: &[u8] = if connected { b"1" } else { b"0" };
        Outcome { deliveries: self.state.keys().map(|a| Delivery::text(a, v, Validity::Keep)).collect(), ..Default::default() }
    }

    /// One message. `state_host` / `state_ts`: the Sparkplug host id this
    /// client publishes STATE for ("" for none) and its timestamp.
    pub fn message(&mut self, topic: &str, payload: &[u8], now_ms: i64, state_host: &str, state_ts: i64) -> Outcome {
        let mut out = Outcome::default();

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

        // exact subscriptions
        if self.subs.get(topic).is_some_and(|s| !s.matching) {
            self.deliver_to(topic, payload, &mut out);
        }

        // wildcard and shared subscriptions
        let matched: Vec<String> = self.matching.iter().filter(|f| address::topic_matches(f, topic)).cloned().collect();
        let mut sparkplug_done = false;
        for f in matched {
            if !sparkplug_done && self.subs.get(&f).is_some_and(|s| s.sparkplug_refs > 0) {
                sparkplug_done = true; // once per message
                let o = self.spb.on_message(topic, payload, now_ms);
                out.warnings.extend(o.warnings.into_iter().map(|text| Warning { sparkplug: true, text }));
                // rebirth requests: QoS 0
                out.publish.extend(o.publish.into_iter().map(|m| Publish { topic: m.topic, payload: m.payload, qos: 0, retain: false }));
                self.apply(o.updates, &mut out);
            }
            // other addresses on the filter (the plan skips Sparkplug ones)
            self.deliver_to(&f, payload, &mut out);
        }
        out
    }

    /// The connection is established: the STATE of the host application
    /// (subscription first, then the message, as Sparkplug 3.0 wants it),
    /// and edge nodes and devices count as offline until their birth.
    pub fn connected(&self, state_host: &str, state_ts: i64) -> Outcome {
        let mut out = Outcome::default();
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
                out.deliveries.extend(addrs.keys().map(|a| Delivery::text(a, b"0", Validity::Valid)));
            }
        }
        out
    }

    /// The connection is gone: every edge node and device goes offline, its
    /// metrics stale.
    pub fn connection_lost(&mut self) -> Outcome {
        let mut out = Outcome::default();
        let updates = self.spb.on_connection_lost().updates;
        self.apply(updates, &mut out);
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

    fn apply(&mut self, updates: Vec<sparkplug::Update>, out: &mut Outcome) {
        use sparkplug::UpdateKind;
        for u in updates {
            let metric = if u.kind == UpdateKind::Online { None } else { Some(u.metric) };
            let key = (u.group, u.node, u.device, metric);
            let Some(addrs) = self.spb_index.get(&key) else { continue };
            for a in addrs.keys() {
                match u.kind {
                    UpdateKind::Value => {
                        // dyn: a string stays one item, even if it looks like an array
                        let dyn_ = self.addresses.get(a).is_some_and(|e| e.dyn_);
                        let text = if dyn_ && u.is_string { json::quote(&u.text) } else { u.text.clone() };
                        out.deliveries.push(Delivery {
                            address: a.clone(),
                            value: Some(text.clone()),
                            time_ms: u.time_ms,
                            periph_time: u.time_ms.is_some(),
                            validity: Validity::Valid,
                        });
                        self.spb_last.insert(a.clone(), text);
                    }
                    UpdateKind::Stale => {
                        // the last value again, invalid (nothing to send before the first one)
                        if let Some(last) = self.spb_last.get(a) {
                            out.deliveries.push(Delivery::text(a, last, Validity::Invalid));
                        }
                    }
                    UpdateKind::Online => {
                        out.deliveries.push(Delivery::text(a, if u.online { b"1" } else { b"0" }, Validity::Valid));
                    }
                }
            }
        }
    }

    /// The (non-Sparkplug) addresses of one subscription.
    fn deliver_to(&mut self, filter: &str, payload: &[u8], out: &mut Outcome) {
        let Router { subs, addresses, json_warned, .. } = self;
        let Some(sub) = subs.get_mut(filter) else { return };
        if sub.plan_dirty {
            build_plan(sub, addresses);
            // addresses changed: a new or changed configuration warns again
            json_warned.clear();
        }

        // one pass over the payload for all paths of this subscription
        let err = if sub.extractor.is_empty() { String::new() } else { sub.extractor.run(payload, &mut sub.results) };

        for d in &sub.plan {
            let Some(j) = &d.json else {
                out.deliveries.push(Delivery {
                    address: d.address.clone(),
                    value: None,
                    time_ms: None,
                    periph_time: false,
                    validity: Validity::Keep,
                });
                continue;
            };
            let Some(r) = j.value.and_then(|id| sub.results.get(id)) else {
                warn_once(json_warned, &d.address, format!("invalid JSON path {}", j.path), out);
                continue;
            };
            match r.found {
                json::Found::Missing => {
                    warn_once(json_warned, &d.address, format!("the JSON payload has no value at {}", j.path), out);
                    continue;
                }
                json::Found::NotJson => {
                    warn_once(json_warned, &d.address, format!("the payload is not JSON: {err}"), out);
                    continue;
                }
                json::Found::Yes => {}
            }
            if r.value.kind == json::Kind::Null {
                continue; // no value, nothing to update
            }
            // dyn: the transformation reads a JSON array or one value; a string
            // stays a JSON string so that its text is one item, even if it looks like an array
            let value = if j.dyn_ && r.value.kind == json::Kind::String { json::quote(&r.value.text) } else { r.value.text.clone() };

            let mut time_ms = None;
            if let Some(ts) = j.ts {
                let t = ts.and_then(|id| sub.results.get(id)).filter(|t| t.found == json::Found::Yes).and_then(|t| json::parse_time(&t.value));
                match t {
                    Some(ms) => time_ms = Some(ms),
                    None => {
                        warn_once(json_warned, &d.address, format!("no valid timestamp at {}, the current time is used", j.ts_path), out);
                        out.deliveries.push(Delivery {
                            address: d.address.clone(),
                            value: Some(value),
                            time_ms: None,
                            periph_time: false,
                            validity: Validity::Keep,
                        });
                        continue;
                    }
                }
            }
            json_warned.remove(&d.address);
            out.deliveries.push(Delivery {
                address: d.address.clone(),
                value: Some(value),
                time_ms,
                periph_time: time_ms.is_some(),
                validity: Validity::Keep,
            });
        }
    }
}

/// A JSON problem of an address, once until a payload fits again: a topic
/// may carry many messages.
fn warn_once(warned: &mut HashSet<String>, address: &str, text: String, out: &mut Outcome) {
    if warned.insert(address.to_string()) {
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
        let e = addresses.get(a);
        if e.is_some_and(|e| e.addr.sparkplug) {
            continue; // decoded by the Sparkplug host application
        }
        let json = e.filter(|e| e.addr.json).map(|e| {
            let (value, path) = add(&e.addr.json_path, &mut sub.extractor);
            let (ts, ts_path) = if e.addr.has_ts {
                let (id, p) = add(&e.addr.ts_path, &mut sub.extractor);
                (Some(id), p)
            } else {
                (None, String::new())
            };
            JsonPlan { dyn_: e.dyn_, value, ts, path, ts_path }
        });
        sub.plan.push(Plan { address: a.clone(), json });
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
            .map(|d| (d.address.clone(), d.value.as_ref().map_or("<payload>".into(), |v| String::from_utf8_lossy(v).into_owned())))
            .collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
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
