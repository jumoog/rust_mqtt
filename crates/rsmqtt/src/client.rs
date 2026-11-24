//! The client object behind the C API.
//!
//! Paho's `AsyncClient` does the MQTT work on its own runtime. This layer
//! turns its results into a single, thread-safe *event queue* that a host
//! drains whenever it likes — which is exactly the shape of a WinCC OA
//! driver's `workProc()`: no callbacks into the host from foreign threads.
//!
//! * Incoming messages and connection state changes come from Paho's event
//!   stream.
//! * Every operation gets a numeric token. Its Paho token (a future) is
//!   handed to a worker thread which awaits all outstanding tokens and posts a
//!   `Complete` event (and records the result for
//!   `rsmqtt_wait_for_completion`).
//! * An optional notify callback tells the host that the queue went from
//!   empty to non-empty, so event-loop hosts can wake up instead of polling.

use std::collections::{HashMap, HashSet, VecDeque};
use std::os::raw::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use paho_mqtt as mqtt;
use mqtt::{Properties, QoS};

use crate::error::*;
use crate::options::{CompletionEvents, OptionsData};
use crate::scram::Scram;

/// Kind of an event in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum EventKind {
    Connected = 1,
    ConnectionLost = 2,
    Disconnected = 3,
    Message = 4,
    Complete = 5,
}

/// The operation a token belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum OpKind {
    None = 0,
    Connect = 1,
    Publish = 2,
    Subscribe = 3,
    Unsubscribe = 4,
    Disconnect = 5,
}

/// One entry of the event queue.
#[derive(Debug, Clone)]
pub struct Event {
    pub kind: EventKind,
    pub op: OpKind,
    pub token: u64,
    pub rc: i32,
    pub reason_code: u8,
    pub reason_codes: Vec<u8>,
    pub session_present: bool,
    pub mqtt_version: i32,
    pub server_uri: String,
    pub text: String,
    pub message: Option<mqtt::Message>,
    pub properties: Properties,
}

impl Event {
    fn new(kind: EventKind) -> Self {
        Self {
            kind,
            op: OpKind::None,
            token: 0,
            rc: RSMQTT_SUCCESS,
            reason_code: 0,
            reason_codes: Vec::new(),
            session_present: false,
            mqtt_version: 0,
            server_uri: String::new(),
            text: String::new(),
            message: None,
            properties: Properties::new(),
        }
    }
}

/// The host's wake-up callback. Called from library threads.
pub type NotifyFn = extern "C" fn(ctx: *mut c_void);

#[derive(Clone, Copy)]
struct Notify {
    func: NotifyFn,
    ctx: usize,
}

/// Remembers the results of the last completed tokens for
/// `wait_for_completion`; old entries are dropped.
const MAX_REMEMBERED_RESULTS: usize = 4096;

#[derive(Default)]
struct Tokens {
    pending: HashSet<u64>,
    results: HashMap<u64, (i32, String)>,
    order: VecDeque<u64>,
}

struct Shared {
    queue: Mutex<VecDeque<Event>>,
    queue_cv: Condvar,
    max_queued: usize,
    dropped: AtomicU64,
    notify: Mutex<Option<Notify>>,
    tokens: Mutex<Tokens>,
    tokens_cv: Condvar,
    completion_events: CompletionEvents,
    /// The SCRAM exchange of the current connect options: every CONNACK,
    /// also of automatic reconnects, must carry its server signature.
    scram: Mutex<Option<Arc<Scram>>>,
    cli: mqtt::AsyncClient,
}

impl Shared {
    /// Checks the server signature of a CONNACK (built-in SCRAM). On a
    /// mismatch the connection is closed: the broker does not know the
    /// password.
    fn verify_connack(&self, props: &Properties) -> Result<(), String> {
        let Some(s) = lock(&self.scram).clone() else { return Ok(()) };
        s.verify_connack(props.get_bytes(mqtt::PropertyId::AuthenticationData)).map_err(|e| {
            drop(self.cli.disconnect(None::<mqtt::DisconnectOptions>)); // runs without being awaited
            format!("{}: {e}", s.mechanism().name())
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding one of these locks cannot leave the data in an
    // invalid state (they are plain collections), so poisoning is ignored.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn push(&self, ev: Event) {
        let was_empty = {
            let mut q = lock(&self.queue);
            if self.max_queued > 0 && q.len() >= self.max_queued {
                // Drop the oldest incoming message to make room; state
                // changes and completions are never dropped.
                if let Some(pos) = q.iter().position(|e| e.kind == EventKind::Message) {
                    q.remove(pos);
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            let was_empty = q.is_empty();
            q.push_back(ev);
            was_empty
        };
        self.queue_cv.notify_all();
        if was_empty {
            let n = *lock(&self.notify);
            if let Some(n) = n {
                (n.func)(n.ctx as *mut c_void);
            }
        }
    }

    fn pop(&self, timeout: Option<Duration>) -> Option<Event> {
        let mut q = lock(&self.queue);
        if let Some(ev) = q.pop_front() {
            return Some(ev);
        }
        let timeout = timeout?;
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self
                .queue_cv
                .wait_timeout(q, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            q = guard;
            if let Some(ev) = q.pop_front() {
                return Some(ev);
            }
        }
    }

    fn complete(&self, mut ev: Event) {
        {
            let mut t = lock(&self.tokens);
            t.pending.remove(&ev.token);
            t.results.insert(ev.token, (ev.rc, ev.text.clone()));
            t.order.push_back(ev.token);
            while t.order.len() > MAX_REMEMBERED_RESULTS {
                if let Some(old) = t.order.pop_front() {
                    t.results.remove(&old);
                }
            }
        }
        self.tokens_cv.notify_all();

        let emit = match self.completion_events {
            CompletionEvents::All => true,
            CompletionEvents::Errors => ev.rc != RSMQTT_SUCCESS,
            CompletionEvents::None => false,
        };
        if emit {
            ev.kind = EventKind::Complete;
            self.push(ev);
        }
    }
}

/// An operation in flight: our token number and the future that resolves
/// to its completion event.
struct PendingOp {
    fut: BoxFuture<'static, Event>,
}

/// The client behind an `rsmqtt_client*`.
pub struct Client {
    cli: mqtt::AsyncClient,
    shared: Arc<Shared>,
    ops_tx: mqtt::async_channel::Sender<PendingOp>,
    stop_tx: mqtt::async_channel::Sender<()>,
    worker: Option<JoinHandle<()>>,
    next_token: AtomicU64,
    opts: Mutex<OptionsData>,
}

impl Client {
    pub fn new(opts: OptionsData) -> FfiResult<Self> {
        let create = opts.create_options()?;
        let cli = mqtt::AsyncClient::new(create)?;

        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            queue_cv: Condvar::new(),
            max_queued: opts.max_queued_events,
            dropped: AtomicU64::new(0),
            notify: Mutex::new(None),
            tokens: Mutex::new(Tokens::default()),
            tokens_cv: Condvar::new(),
            completion_events: opts.completion_events,
            scram: Mutex::new(None),
            cli: cli.clone(),
        });

        // The event stream must exist before connecting so nothing is lost.
        let events = cli.get_event_stream(None);
        let (ops_tx, ops_rx) = mqtt::async_channel::unbounded::<PendingOp>();
        let (stop_tx, stop_rx) = mqtt::async_channel::bounded::<()>(1);

        let sh = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("rsmqtt-events".into())
            .spawn(move || worker_main(sh, events, ops_rx, stop_rx))
            .map_err(|e| FfiError::new(RSMQTT_FAILURE, format!("cannot start worker thread: {e}")))?;

        Ok(Self {
            cli,
            shared,
            ops_tx,
            stop_tx,
            worker: Some(worker),
            next_token: AtomicU64::new(1),
            opts: Mutex::new(opts),
        })
    }

    pub fn set_options(&self, opts: OptionsData) {
        *lock(&self.opts) = opts;
    }

    pub fn set_notify(&self, func: Option<NotifyFn>, ctx: *mut c_void) {
        *lock(&self.shared.notify) = func.map(|func| Notify {
            func,
            ctx: ctx as usize,
        });
    }

    pub fn mqtt_version(&self) -> mqtt::MqttVersion {
        self.cli.mqtt_version()
    }

    pub fn is_connected(&self) -> bool {
        self.cli.is_connected()
    }

    pub fn client_id(&self) -> String {
        self.cli.client_id()
    }

    pub fn dropped_events(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    pub fn poll(&self, timeout: Option<Duration>) -> Option<Event> {
        self.shared.pop(timeout)
    }

    pub fn queued_events(&self) -> usize {
        lock(&self.shared.queue).len()
    }

    /// Registers a Paho token under a new token number.
    fn track<T, F>(&self, op: OpKind, tok: mqtt::Token<T>, on_ok: F) -> FfiResult<u64>
    where
        T: Clone + Send + 'static,
        F: FnOnce(T, &mut Event) + Send + 'static,
    {
        let id = self.next_token.fetch_add(1, Ordering::Relaxed);
        lock(&self.shared.tokens).pending.insert(id);

        let fut = async move {
            let mut ev = Event::new(EventKind::Complete);
            ev.op = op;
            ev.token = id;
            match tok.await {
                Ok(rsp) => on_ok(rsp, &mut ev),
                Err(err) => fill_error(&mut ev, err),
            }
            ev
        }
        .boxed();

        if self.ops_tx.try_send(PendingOp { fut }).is_err() {
            lock(&self.shared.tokens).pending.remove(&id);
            return Err(FfiError::new(RSMQTT_FAILURE, "client is being destroyed"));
        }
        Ok(id)
    }

    pub fn connect(&self) -> FfiResult<u64> {
        let conn = lock(&self.opts).connect()?;
        // in place before the CONNACK can come; a refused connect (one in
        // progress, already connected) keeps the exchange of the options in use
        let prev = std::mem::replace(&mut *lock(&self.shared.scram), conn.scram);
        let tok = match self.cli.try_connect(conn.opts) {
            Ok(tok) => tok,
            Err(e) => {
                *lock(&self.shared.scram) = prev;
                return Err(e.into());
            }
        };
        self.track_connect(tok)
    }

    pub fn reconnect(&self) -> FfiResult<u64> {
        let tok = self.cli.try_reconnect()?;
        self.track_connect(tok)
    }

    fn track_connect(&self, tok: mqtt::ConnectToken) -> FfiResult<u64> {
        let shared = Arc::clone(&self.shared);
        self.track(OpKind::Connect, tok, move |rsp, ev| {
            ev.session_present = rsp.session_present;
            ev.mqtt_version = rsp.mqtt_version as i32;
            ev.server_uri = rsp.server_uri;
            ev.reason_code = rsp.reason_code as u8;
            if let Err(e) = shared.verify_connack(&rsp.properties) {
                ev.rc = RSMQTT_FAILURE;
                ev.text = e;
            }
            ev.properties = rsp.properties;
        })
    }

    pub fn disconnect(
        &self,
        timeout: Duration,
        reason: Option<mqtt::ReasonCode>,
        props: Option<Properties>,
        publish_will: bool,
    ) -> FfiResult<u64> {
        let mut b = mqtt::DisconnectOptionsBuilder::new();
        b.timeout(timeout);
        if let Some(r) = reason {
            b.reason_code(r);
        }
        if publish_will {
            b.publish_will_message();
        }
        if let Some(p) = props {
            if self.mqtt_version() == mqtt::MqttVersion::V5 {
                b.properties(p);
            }
        }
        let tok = self.cli.try_disconnect(b.finalize())?;
        self.track(OpKind::Disconnect, tok, |_, _| {})
    }

    pub fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: QoS,
        retained: bool,
        props: Option<Properties>,
    ) -> FfiResult<u64> {
        let mut b = mqtt::MessageBuilder::new()
            .topic(topic)
            .payload(payload.to_vec())
            .qos(qos)
            .retain(retained);
        if let Some(p) = props {
            if self.mqtt_version() == mqtt::MqttVersion::V5 {
                b = b.properties(p);
            }
        }
        let tok = self.cli.try_publish(b.finalize())?;
        self.track(OpKind::Publish, tok, |rsp, ev| {
            ev.reason_code = rsp.reason_code as u8;
            ev.properties = rsp.properties;
        })
    }

    pub fn subscribe_many(
        &self,
        filters: &[String],
        qos: &[QoS],
        sub_opts: &[mqtt::SubscribeOptions],
        props: Option<Properties>,
    ) -> FfiResult<u64> {
        let v5 = self.mqtt_version() == mqtt::MqttVersion::V5;
        let props = if v5 { props } else { None };
        let tok = if v5 {
            self.cli
                .try_subscribe_many_with_options(filters, qos, sub_opts, props)?
        } else {
            self.cli.try_subscribe_many(filters, qos)?
        };
        self.track(OpKind::Subscribe, tok, |rsp, ev| {
            ev.reason_codes = rsp.reason_codes.iter().map(|r| *r as u8).collect();
            ev.reason_code = ev.reason_codes.first().copied().unwrap_or(0);
            ev.properties = rsp.properties;
            // A refused filter (reason >= 0x80) is reported as a failure of
            // the whole request only if *every* filter was refused, like
            // Paho C does for single subscriptions.
            if !ev.reason_codes.is_empty() && ev.reason_codes.iter().all(|r| *r >= 0x80) {
                ev.rc = RSMQTT_REASON_CODE;
                ev.text = format!("subscription refused by server (reason 0x{:02X})", ev.reason_code);
            }
        })
    }

    pub fn unsubscribe_many(&self, filters: &[String], props: Option<Properties>) -> FfiResult<u64> {
        let v5 = self.mqtt_version() == mqtt::MqttVersion::V5;
        let tok = match props {
            Some(p) if v5 => self.cli.try_unsubscribe_many_with_options(filters, p)?,
            _ => self.cli.try_unsubscribe_many(filters)?,
        };
        self.track(OpKind::Unsubscribe, tok, |rsp, ev| {
            ev.reason_codes = rsp.reason_codes.iter().map(|r| *r as u8).collect();
            ev.reason_code = ev.reason_codes.first().copied().unwrap_or(0);
            ev.properties = rsp.properties;
        })
    }

    /// Waits until the token completed and returns its result code.
    pub fn wait_for_completion(&self, token: u64, timeout: Duration) -> FfiResult<()> {
        let deadline = Instant::now() + timeout;
        let mut t = lock(&self.shared.tokens);
        loop {
            if let Some((rc, text)) = t.results.get(&token) {
                return if *rc == RSMQTT_SUCCESS {
                    Ok(())
                } else {
                    Err(FfiError::new(*rc, text.clone()))
                };
            }
            if !t.pending.contains(&token) {
                // Unknown, or completed so long ago its result was dropped.
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(FfiError::new(RSMQTT_TIMEOUT, "operation not completed in time"));
            }
            let (guard, _) = self
                .shared
                .tokens_cv
                .wait_timeout(t, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            t = guard;
        }
    }

    /// Number of operations not completed yet (cheap; no copy).
    pub fn pending_count(&self) -> usize {
        lock(&self.shared.tokens).pending.len()
    }

    pub fn pending_tokens(&self) -> Vec<u64> {
        let mut v: Vec<u64> = lock(&self.shared.tokens).pending.iter().copied().collect();
        v.sort_unstable();
        v
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.stop_tx.try_send(());
        self.ops_tx.close();
        self.cli.stop_stream();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn fill_error(ev: &mut Event, err: mqtt::Error) {
    if let mqtt::Error::ReasonCode(_, props) = &err {
        ev.properties = props.clone();
    }
    let fe = FfiError::from(err);
    ev.rc = fe.rc;
    ev.reason_code = fe.reason_code;
    ev.text = fe.message;
}

/// The worker: awaits Paho events and the tokens of all operations.
fn worker_main(
    shared: Arc<Shared>,
    events: mqtt::AsyncReceiver<mqtt::Event>,
    ops: mqtt::async_channel::Receiver<PendingOp>,
    stop: mqtt::async_channel::Receiver<()>,
) {
    futures::executor::block_on(async move {
        let mut pending: FuturesUnordered<BoxFuture<'static, Event>> = FuturesUnordered::new();
        let mut events = Box::pin(events.fuse());
        let mut ops = Box::pin(ops.fuse());
        let mut stop = Box::pin(stop.fuse());
        loop {
            futures::select! {
                _ = stop.next() => break,
                op = ops.next() => match op {
                    Some(op) => pending.push(op.fut),
                    None => {
                        // Client is being destroyed.
                        break;
                    }
                },
                ev = events.next() => if let Some(ev) = ev {
                    shared.push(convert_event(&shared, ev));
                },
                done = pending.select_next_some() => shared.complete(done),
                complete => break,
            }
        }
    });
}

fn convert_event(shared: &Shared, ev: mqtt::Event) -> Event {
    match ev {
        mqtt::Event::Connected(rsp) => {
            // a broker that fails the SCRAM check: closed again at once
            if let Err(text) = shared.verify_connack(&rsp.properties) {
                let mut e = Event::new(EventKind::ConnectionLost);
                e.rc = RSMQTT_FAILURE;
                e.text = text;
                return e;
            }
            let mut e = Event::new(EventKind::Connected);
            e.session_present = rsp.session_present;
            e.mqtt_version = rsp.mqtt_version as i32;
            e.server_uri = rsp.server_uri;
            e.reason_code = rsp.reason_code as u8;
            e.properties = rsp.properties;
            e
        }
        mqtt::Event::Message(msg) => {
            let mut e = Event::new(EventKind::Message);
            e.properties = msg.properties().clone();
            e.message = Some(msg);
            e
        }
        mqtt::Event::ConnectionLost => {
            let mut e = Event::new(EventKind::ConnectionLost);
            e.rc = RSMQTT_DISCONNECTED;
            e.text = "connection lost".into();
            e
        }
        mqtt::Event::Disconnected { reason_code, props } => {
            let mut e = Event::new(EventKind::Disconnected);
            e.reason_code = reason_code as u8;
            e.text = format!("disconnected by server: {reason_code}");
            e.properties = props;
            e
        }
    }
}
