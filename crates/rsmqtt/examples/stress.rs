//! MQTT load generator / counter built on rsmqtt.
//!
//!   cargo run --release --example stress -- pub --uri tcp://localhost:1883 \
//!       --topic "stress/in/{i}" --topics 500 --rounds 200 --qos 1
//!   cargo run --release --example stress -- sub --uri tcp://localhost:1883 \
//!       --filter "stress/out/#" --expect 10000 --timeout 60
//!
//! pub: publishes `rounds` x `topics` messages; `{i}` in the topic is the
//!      topic number (1-based). The payload is the round number as text, so
//!      after the run every topic's last value is `rounds - 1`.
//!      --rate limits the messages per second (0 = as fast as possible).
//! sub: subscribes and counts until --expect messages arrived or nothing
//!      came for --timeout seconds; exit code 0 if the expectation was met.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use paho_mqtt::QoS;
use rsmqtt::client::{Client, EventKind};
use rsmqtt::options::OptionsData;

struct Args {
    mode: String,
    uri: String,
    topic: String,
    filter: String,
    topics: usize,
    rounds: usize,
    qos: u8,
    rate: f64,
    expect: usize,
    timeout: u64,
    retain: bool,
}

fn parse() -> Args {
    let mut a = Args {
        mode: String::new(),
        uri: "tcp://localhost:1883".into(),
        topic: "stress/{i}".into(),
        filter: "stress/#".into(),
        topics: 100,
        rounds: 100,
        qos: 1,
        rate: 0.0,
        expect: 0,
        timeout: 10,
        retain: false,
    };
    let mut it = std::env::args().skip(1);
    a.mode = it.next().unwrap_or_default();
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| usage(&format!("missing value for {k}")));
        match k.as_str() {
            "--uri" => a.uri = v(),
            "--topic" => a.topic = v(),
            "--filter" => a.filter = v(),
            "--topics" => a.topics = v().parse().unwrap_or_else(|_| usage("--topics")),
            "--rounds" => a.rounds = v().parse().unwrap_or_else(|_| usage("--rounds")),
            "--qos" => a.qos = v().parse().unwrap_or_else(|_| usage("--qos")),
            "--rate" => a.rate = v().parse().unwrap_or_else(|_| usage("--rate")),
            "--expect" => a.expect = v().parse().unwrap_or_else(|_| usage("--expect")),
            "--timeout" => a.timeout = v().parse().unwrap_or_else(|_| usage("--timeout")),
            "--retain" => a.retain = true,
            _ => usage(&format!("unknown argument {k}")),
        }
    }
    if a.mode != "pub" && a.mode != "sub" {
        usage("mode must be pub or sub");
    }
    a
}

fn usage(err: &str) -> ! {
    eprintln!("error: {err}\nusage: stress pub|sub [--uri U] [--topic T] [--topics N] [--rounds N] [--qos Q] [--rate R] [--retain] [--filter F] [--expect N] [--timeout S]");
    std::process::exit(2)
}

fn connect(uri: &str, extra: &[(&str, &str)]) -> Client {
    let mut o = OptionsData::default();
    o.set("serverURI", uri).unwrap();
    let id = format!("rsmqtt-stress-{}", std::process::id());
    o.set("clientId", &id).unwrap();
    for (k, v) in extra {
        o.set(k, v).unwrap();
    }
    let c = Client::new(o).unwrap_or_else(|e| fail(&e.message));
    let tok = c.connect().unwrap_or_else(|e| fail(&e.message));
    c.wait_for_completion(tok, Duration::from_secs(10)).unwrap_or_else(|e| fail(&e.message));
    c
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}

fn qos(q: u8) -> QoS {
    QoS::try_from(q).unwrap_or_else(|_| usage("--qos must be 0, 1 or 2"))
}

fn main() {
    let a = parse();
    if a.mode == "pub" {
        run_pub(&a)
    } else {
        run_sub(&a)
    }
}

fn run_pub(a: &Args) {
    let c = connect(&a.uri, &[("completionEvents", "errors")]);
    let topics: Vec<String> = (1..=a.topics).map(|i| a.topic.replace("{i}", &i.to_string())).collect();
    let total = a.topics * a.rounds;
    let q = qos(a.qos);
    let start = Instant::now();
    let mut sent = 0usize;
    for round in 0..a.rounds {
        let payload = round.to_string();
        for t in &topics {
            c.publish(t, payload.as_bytes(), q, a.retain, None).unwrap_or_else(|e| fail(&e.message));
            sent += 1;
            if a.rate > 0.0 {
                let due = start + Duration::from_secs_f64(sent as f64 / a.rate);
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
            }
        }
    }
    let queued = start.elapsed();
    while c.pending_count() > 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    let done = start.elapsed();
    let mut errors = 0;
    while let Some(ev) = c.poll(None) {
        if ev.kind == EventKind::Complete && ev.rc != 0 {
            errors += 1;
        }
    }
    println!(
        "published {total} msgs ({} topics x {} rounds, QoS {}) queued in {:.2}s, completed in {:.2}s = {:.0} msg/s, {errors} errors",
        a.topics,
        a.rounds,
        a.qos,
        queued.as_secs_f64(),
        done.as_secs_f64(),
        total as f64 / done.as_secs_f64()
    );
    if errors > 0 {
        std::process::exit(1);
    }
}

fn run_sub(a: &Args) {
    let c = connect(&a.uri, &[]);
    let tok = c
        .subscribe_many(std::slice::from_ref(&a.filter), &[qos(a.qos)], &[paho_mqtt::SubscribeOptions::default()], None)
        .unwrap_or_else(|e| fail(&e.message));
    c.wait_for_completion(tok, Duration::from_secs(10)).unwrap_or_else(|e| fail(&e.message));
    println!("subscribed to {} - waiting for messages", a.filter);

    let mut count = 0usize;
    let mut first: Option<Instant> = None;
    let mut last_msg = Instant::now();
    let mut last: HashMap<String, String> = HashMap::new();
    let idle = Duration::from_secs(a.timeout);
    while (a.expect == 0 || count < a.expect) && last_msg.elapsed() < idle {
        if let Some(ev) = c.poll(Some(Duration::from_millis(100))) {
            if let (EventKind::Message, Some(m)) = (ev.kind, ev.message) {
                first.get_or_insert_with(Instant::now);
                count += 1;
                last_msg = Instant::now();
                last.insert(m.topic().to_string(), m.payload_str().into_owned());
            }
        }
    }
    let secs = first.map(|f| f.elapsed().as_secs_f64()).unwrap_or(0.0).max(1e-9);
    let mut values: HashMap<&str, usize> = HashMap::new();
    for v in last.values() {
        *values.entry(v.as_str()).or_default() += 1;
    }
    let summary = if values.len() <= 5 {
        format!("{values:?}")
    } else {
        let mut some: Vec<&str> = values.keys().copied().collect();
        some.sort_unstable();
        format!("{} distinct, e.g. {:?}", values.len(), &some[..5])
    };
    println!(
        "received {count} msgs on {} topics in {secs:.2}s = {:.0} msg/s; last values: {summary}",
        last.len(),
        count as f64 / secs
    );
    if a.expect > 0 && count < a.expect {
        eprintln!("expected {} messages, got {count}", a.expect);
        std::process::exit(1);
    }
}
