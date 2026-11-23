//! Stress tests: many messages, many topics, many threads, big payloads.
//!
//! They run only when `RSMQTT_TEST_URI` is set (see tests/mosquitto.conf in
//! the repository root, which raises the broker's queue limits), and are
//! meant to be run in release mode, one at a time:
//!
//!   RSMQTT_TEST_URI=tcp://localhost:18830 cargo test --release --test stress -- --test-threads=1 --nocapture
//!
//! `RSMQTT_STRESS_SCALE` (default 1.0) multiplies all message counts.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use paho_mqtt::QoS;
use rsmqtt::client::{Client, EventKind};
use rsmqtt::options::OptionsData;

fn uri() -> Option<String> {
    std::env::var("RSMQTT_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn scaled(n: usize) -> usize {
    let scale: f64 = std::env::var("RSMQTT_STRESS_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    ((n as f64 * scale) as usize).max(10)
}

fn unique(prefix: &str) -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{n}")
}

fn client(uri: &str, extra: &[(&str, &str)]) -> Client {
    let mut o = OptionsData::default();
    o.set("serverURI", uri).unwrap();
    o.set("clientId", &unique("stress")).unwrap();
    for (k, v) in extra {
        o.set(k, v).unwrap();
    }
    let c = Client::new(o).expect("create client");
    let tok = c.connect().expect("connect");
    c.wait_for_completion(tok, Duration::from_secs(10)).expect("connect completes");
    c
}

fn subscribe(c: &Client, filter: &str, qos: QoS) {
    let tok = c
        .subscribe_many(&[filter.to_string()], &[qos], &[paho_mqtt::SubscribeOptions::default()], None)
        .expect("subscribe");
    c.wait_for_completion(tok, Duration::from_secs(10)).expect("suback");
}

/// Waits until the client has no outstanding operations.
fn drain_pending(c: &Client, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while c.pending_count() > 0 {
        assert!(
            Instant::now() < deadline,
            "{} operations still pending after {timeout:?}",
            c.pending_count()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Receives messages until `n` arrived or nothing came for `idle`.
fn collect(c: &Client, n: usize, idle: Duration) -> Vec<paho_mqtt::Message> {
    let mut out = Vec::with_capacity(n);
    let mut last = Instant::now();
    while out.len() < n && last.elapsed() < idle {
        if let Some(ev) = c.poll(Some(Duration::from_millis(50))) {
            if ev.kind == EventKind::Message {
                out.push(ev.message.unwrap());
                last = Instant::now();
            }
        }
    }
    out
}

fn seq_of(m: &paho_mqtt::Message) -> u64 {
    u64::from_le_bytes(m.payload()[..8].try_into().unwrap())
}

fn rate(n: usize, d: Duration) -> String {
    format!("{n} msgs in {:.2}s = {:.0} msg/s", d.as_secs_f64(), n as f64 / d.as_secs_f64())
}

/// One topic, many messages per QoS level: no loss for QoS 1/2, no
/// duplicates for QoS 2, and in-order delivery.
#[test]
fn stress_throughput_per_qos() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };

    for (qos, count) in [(QoS::AtMostOnce, scaled(50_000)), (QoS::AtLeastOnce, scaled(30_000)), (QoS::ExactlyOnce, scaled(15_000))] {
        let topic = unique(&format!("rsmqtt/stress/qos{}", qos as u8));
        let sub = client(&u, &[]);
        subscribe(&sub, &topic, qos);
        // Completion events are not needed; pending tokens tell when done.
        let publ = client(&u, &[("completionEvents", "none")]);

        let start = Instant::now();
        let receiver = std::thread::spawn({
            let sub = Arc::new(sub);
            let s = Arc::clone(&sub);
            move || (collect(&s, count, Duration::from_secs(15)), start.elapsed())
        });

        let mut payload = vec![0u8; 64];
        for i in 0..count as u64 {
            payload[..8].copy_from_slice(&i.to_le_bytes());
            publ.publish(&topic, &payload, qos, false, None).expect("publish");
        }
        let queued = start.elapsed();
        drain_pending(&publ, Duration::from_secs(120));
        let published = start.elapsed();
        let (msgs, received) = receiver.join().unwrap();

        eprintln!(
            "QoS {}: queued {} | published {} | received {}",
            qos as u8,
            rate(count, queued),
            rate(count, published),
            rate(msgs.len(), received)
        );

        let seqs: Vec<u64> = msgs.iter().map(seq_of).collect();
        match qos {
            QoS::AtMostOnce => {
                // A local broker loses nothing, but QoS 0 allows it.
                assert!(msgs.len() * 10 >= count * 9, "QoS 0 lost more than 10%: {} of {count}", msgs.len());
            }
            _ => {
                let distinct: HashSet<u64> = seqs.iter().copied().collect();
                assert_eq!(distinct.len(), count, "QoS {} lost messages", qos as u8);
                if qos == QoS::ExactlyOnce {
                    assert_eq!(seqs.len(), count, "QoS 2 delivered duplicates");
                }
            }
        }
        // Per-topic ordering (MQTT 4.6): first deliveries must be increasing.
        let mut seen = HashSet::new();
        let firsts: Vec<u64> = seqs.into_iter().filter(|s| seen.insert(*s)).collect();
        assert!(firsts.windows(2).all(|w| w[0] < w[1]), "QoS {} messages out of order", qos as u8);
    }
}

/// Thousands of distinct topics under one wildcard subscription.
#[test]
fn stress_many_topics() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    let topics = scaled(10_000);
    let base = unique("rsmqtt/stress/topics");
    let sub = client(&u, &[]);
    subscribe(&sub, &format!("{base}/#"), QoS::AtLeastOnce);
    let publ = client(&u, &[("completionEvents", "none")]);

    let start = Instant::now();
    for i in 0..topics {
        // three levels, so the wildcard match is exercised
        let t = format!("{base}/{}/{}/{}", i % 10, (i / 10) % 100, i);
        publ.publish(&t, i.to_string().as_bytes(), QoS::AtLeastOnce, false, None).unwrap();
    }
    drain_pending(&publ, Duration::from_secs(60));
    let msgs = collect(&sub, topics, Duration::from_secs(15));
    eprintln!("many topics: {}", rate(msgs.len(), start.elapsed()));

    let distinct: HashSet<&str> = msgs.iter().map(|m| m.topic()).collect();
    assert_eq!(distinct.len(), topics, "not every topic arrived");
    for m in &msgs {
        assert!(m.topic().ends_with(&format!("/{}", m.payload_str())), "payload/topic mismatch on {}", m.topic());
    }
}

/// Many threads publish through the same client at once.
#[test]
fn stress_concurrent_publishers() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    let threads = 8u64;
    let per_thread = scaled(5_000) as u64;
    let topic = unique("rsmqtt/stress/threads");
    let sub = client(&u, &[]);
    subscribe(&sub, &topic, QoS::AtLeastOnce);
    let publ = Arc::new(client(&u, &[("completionEvents", "none")]));

    let start = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let p = Arc::clone(&publ);
            let topic = topic.clone();
            std::thread::spawn(move || {
                let mut payload = [0u8; 16];
                payload[8..].copy_from_slice(&t.to_le_bytes());
                for i in 0..per_thread {
                    payload[..8].copy_from_slice(&i.to_le_bytes());
                    p.publish(&topic, &payload, QoS::AtLeastOnce, false, None).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    drain_pending(&publ, Duration::from_secs(60));
    let total = (threads * per_thread) as usize;
    let msgs = collect(&sub, total, Duration::from_secs(15));
    eprintln!("{threads} threads: {}", rate(msgs.len(), start.elapsed()));

    // every (thread, seq) arrived, and each thread's sequence is in order
    let mut per: HashMap<u64, Vec<u64>> = HashMap::new();
    for m in &msgs {
        let p = m.payload();
        let i = u64::from_le_bytes(p[..8].try_into().unwrap());
        let t = u64::from_le_bytes(p[8..16].try_into().unwrap());
        per.entry(t).or_default().push(i);
    }
    assert_eq!(per.len() as u64, threads);
    for (t, seqs) in per {
        let mut seen = HashSet::new();
        let firsts: Vec<u64> = seqs.into_iter().filter(|s| seen.insert(*s)).collect();
        assert_eq!(firsts.len() as u64, per_thread, "thread {t} lost messages");
        assert!(firsts.windows(2).all(|w| w[0] < w[1]), "thread {t} out of order");
    }
}

/// Big payloads keep their content.
#[test]
fn stress_large_payloads() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    let count = scaled(200);
    let size = 256 * 1024;
    let topic = unique("rsmqtt/stress/large");
    let sub = client(&u, &[]);
    subscribe(&sub, &topic, QoS::AtLeastOnce);
    let publ = client(&u, &[("completionEvents", "none")]);

    let start = Instant::now();
    for i in 0..count {
        let payload: Vec<u8> = (0..size).map(|k| ((k * 31 + i * 7) % 251) as u8).collect();
        publ.publish(&topic, &payload, QoS::AtLeastOnce, false, None).unwrap();
    }
    drain_pending(&publ, Duration::from_secs(120));
    let msgs = collect(&sub, count, Duration::from_secs(20));
    let mb = (count * size) as f64 / (1024.0 * 1024.0);
    eprintln!("large payloads: {} ({mb:.0} MiB, {:.0} MiB/s)", rate(msgs.len(), start.elapsed()), mb / start.elapsed().as_secs_f64());

    assert_eq!(msgs.len(), count);
    for (i, m) in msgs.iter().enumerate() {
        let p = m.payload();
        assert_eq!(p.len(), size);
        assert!(p.iter().enumerate().all(|(k, b)| *b == ((k * 31 + i * 7) % 251) as u8), "payload {i} corrupted");
    }
}

/// A host that stops polling must not run out of memory: with
/// maxQueuedEvents the oldest messages are dropped and counted.
#[test]
fn stress_event_queue_limit() {
    let Some(u) = uri() else { return eprintln!("RSMQTT_TEST_URI not set; skipped") };
    let count = scaled(20_000);
    let limit = 1_000usize;
    let topic = unique("rsmqtt/stress/limit");
    let sub = client(&u, &[("maxQueuedEvents", "1000")]);
    // QoS 1 so the broker cannot drop anything: every message is either
    // queued or counted as dropped by the client.
    subscribe(&sub, &topic, QoS::AtLeastOnce);
    // the Complete event of the subscribe is in the queue; take it out
    while sub.poll(None).is_some() {}

    let publ = client(&u, &[("completionEvents", "none")]);
    let mut payload = vec![0u8; 32];
    for i in 0..count as u64 {
        payload[..8].copy_from_slice(&i.to_le_bytes());
        publ.publish(&topic, &payload, QoS::AtLeastOnce, false, None).unwrap();
    }
    drain_pending(&publ, Duration::from_secs(60));

    // wait until the subscriber's worker has queued everything that came
    let deadline = Instant::now() + Duration::from_secs(20);
    while (sub.queued_events() as u64 + sub.dropped_events()) < count as u64 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let queued = sub.queued_events();
    let dropped = sub.dropped_events();
    eprintln!("queue limit: queued {queued}, dropped {dropped} of {count}");
    assert!(queued <= limit, "queue grew beyond the limit: {queued}");
    assert_eq!(queued as u64 + dropped, count as u64, "messages neither queued nor counted as dropped");

    // what is left are the newest messages, in order
    let msgs = collect(&sub, limit, Duration::from_secs(2));
    let seqs: Vec<u64> = msgs.iter().map(seq_of).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(*seqs.last().unwrap(), count as u64 - 1, "the newest message must be kept");
}
