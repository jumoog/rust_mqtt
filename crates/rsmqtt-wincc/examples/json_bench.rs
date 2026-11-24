//! Benchmark of the JSON path extraction: rsmqtt-wincc's one-pass
//! `json::Extractor` against sonic-rs (SIMD) `get_many` with a `PointerTree`.
//!
//!   cargo run --release -p rsmqtt-wincc --example json_bench --features bench-sonic
//!
//! Each case extracts the values at some paths (strings unescaped, other
//! values as JSON text, copied into a buffer like the extractor does), as the
//! driver does for json= / ts= addresses. A second part checks that both give
//! the same results and how they treat invalid JSON.

use std::hint::black_box;
use std::time::{Duration, Instant};

use rsmqtt_wincc::json::{self, Extractor, Found, Segment};
use sonic_rs::{JsonValueTrait, PointerNode, PointerTree};

struct Case {
    name: &'static str,
    doc: Vec<u8>,
    paths: Vec<&'static str>,
}

fn cases() -> Vec<Case> {
    let small = br#"{"ts":1700000000000,"data":{"temp":21.5,"hum":40.2,"state":"ok"},"device":"plc-7"}"#.to_vec();

    let mut medium = String::from(r#"{"meta":{"site":"Vienna","line":3,"tags":["a","b","c"]},"values":{"#);
    for i in 0..60 {
        medium += &format!(r#"{}"v{i}":{{"value":{}.25,"unit":"degC","quality":"good"}}"#, if i > 0 { "," } else { "" }, i * 3);
    }
    medium += r#"},"status":"ok"}"#;

    let mut large = String::from(r#"{"records":["#);
    for i in 0..150 {
        large += &format!(
            r#"{}{{"id":{i},"name":"device-{i} \"quoted\" é","desc":"{}","vals":[1.5,2.5,3.5,{i}]}}"#,
            if i > 0 { "," } else { "" },
            "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor ".repeat(2)
        );
    }
    large += r#"],"summary":{"count":150}}"#;

    let blob = "The quick brown fox jumps over the lazy dog, \\\"again\\\" and again. ".repeat(1000);
    let bigstring = format!(r#"{{"blob":"{blob}","value":42}}"#);

    vec![
        Case { name: "small (83 B), 2 paths", doc: small, paths: vec!["data.temp", "ts"] },
        Case { name: "medium, 2 paths at the end", doc: medium.clone().into_bytes(), paths: vec!["values.v59.value", "status"] },
        Case {
            name: "medium, 10 paths",
            doc: medium.into_bytes(),
            paths: vec![
                "meta.site", "meta.tags[2]", "values.v0.value", "values.v10.unit", "values.v20.value", "values.v30.quality",
                "values.v40.value", "values.v50.value", "values.v59.value", "status",
            ],
        },
        Case { name: "large, 2 paths at the end", doc: large.into_bytes(), paths: vec!["records[149].name", "summary.count"] },
        Case { name: "one 66 KB string, then the value", doc: bigstring.into_bytes(), paths: vec!["value"] },
    ]
}

fn pointer(path: &str) -> Vec<PointerNode> {
    json::parse_path(path)
        .unwrap()
        .into_iter()
        .map(|s| match s {
            Segment::Key(k) => PointerNode::Key(k.into()),
            Segment::Index(i) => PointerNode::Index(i),
        })
        .collect()
}

struct Ours {
    x: Extractor,
    results: Vec<json::ExResult>,
}

impl Ours {
    fn new(paths: &[&str]) -> Self {
        let mut x = Extractor::new();
        for p in paths {
            x.add(&json::parse_path(p).unwrap());
        }
        Ours { x, results: Vec::new() }
    }

    /// (found, text) per path, or None for an invalid document
    fn run(&mut self, doc: &[u8]) -> Option<Vec<Option<Vec<u8>>>> {
        let err = self.x.run(doc, &mut self.results);
        if !err.is_empty() && self.results.iter().any(|r| r.found == Found::NotJson) {
            return None;
        }
        Some(self.results.iter().map(|r| (r.found == Found::Yes).then(|| r.value.text.clone())).collect())
    }

    fn bench(&mut self, doc: &[u8]) -> usize {
        self.x.run(doc, &mut self.results);
        self.results.iter().map(|r| r.value.text.len()).sum()
    }
}

struct Sonic {
    tree: PointerTree,
    buf: Vec<Vec<u8>>,
}

impl Sonic {
    fn new(paths: &[&str]) -> Self {
        let mut tree = PointerTree::new();
        for p in paths {
            tree.add_path(pointer(p).iter());
        }
        Sonic { tree, buf: vec![Vec::new(); paths.len()] }
    }

    fn values(&mut self, doc: &[u8], checked: bool) -> Option<Vec<Option<Vec<u8>>>> {
        let r = if checked { sonic_rs::get_many(doc, &self.tree) } else { unsafe { sonic_rs::get_many_unchecked(doc, &self.tree) } };
        let vals = r.ok()?;
        Some(
            vals.iter()
                .map(|v| v.as_ref().map(|v| if v.is_str() { v.as_str().unwrap_or("").as_bytes().to_vec() } else { v.as_raw_str().as_bytes().to_vec() }))
                .collect(),
        )
    }

    /// like Ours::bench: the texts go into reused buffers
    fn bench(&mut self, doc: &[u8], checked: bool) -> usize {
        let r = if checked { sonic_rs::get_many(doc, &self.tree) } else { unsafe { sonic_rs::get_many_unchecked(doc, &self.tree) } };
        let mut n = 0;
        if let Ok(vals) = r {
            for (k, v) in vals.iter().enumerate() {
                let b = &mut self.buf[k];
                b.clear();
                if let Some(v) = v {
                    if v.is_str() {
                        b.extend_from_slice(v.as_str().unwrap_or("").as_bytes());
                    } else {
                        b.extend_from_slice(v.as_raw_str().as_bytes());
                    }
                }
                n += b.len();
            }
        }
        n
    }
}

/// ns per call, measured over ~0.4 s after a warm-up
fn time(mut f: impl FnMut() -> usize) -> f64 {
    let warm = Instant::now();
    while warm.elapsed() < Duration::from_millis(100) {
        black_box(f());
    }
    let mut best = f64::MAX;
    for _ in 0..4 {
        let mut n = 0u64;
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(100) {
            for _ in 0..16 {
                black_box(f());
            }
            n += 16;
        }
        best = best.min(t.elapsed().as_nanos() as f64 / n as f64);
    }
    best
}

fn main() {
    println!(
        "{:<36} {:>8} | {:>12} {:>12} {:>14} | {:>7}",
        "case", "bytes", "extractor", "sonic get", "sonic unchecked", "speedup"
    );
    for c in cases() {
        let mut ours = Ours::new(&c.paths);
        let mut sonic = Sonic::new(&c.paths);
        // same results first
        let a = ours.run(&c.doc).expect("valid document");
        let b = sonic.values(&c.doc, true).expect("valid document");
        assert_eq!(a, b, "{}: results differ", c.name);

        let doc = c.doc.as_slice();
        let t_ours = time(|| ours.bench(black_box(doc)));
        let t_sonic = time(|| sonic.bench(black_box(doc), true));
        let t_unchecked = time(|| sonic.bench(black_box(doc), false));
        let mbs = |ns: f64| doc.len() as f64 / ns * 1000.0;
        println!(
            "{:<36} {:>8} | {:>7.0} ns {:>7.0} ns {:>9.0} ns | {:>6.1}x   ({:.0} / {:.0} MB/s)",
            c.name,
            doc.len(),
            t_ours,
            t_sonic,
            t_unchecked,
            t_ours / t_sonic,
            mbs(t_ours),
            mbs(t_sonic)
        );
    }

    // ---- semantics: invalid or odd documents
    println!("\ninvalid / odd documents (path \"a\"): extractor | sonic get_many");
    let docs: [&[u8]; 9] = [
        br#"{"a":1,"b":tru}"#,               // invalid after the value
        br#"{"b":tru,"a":1}"#,               // invalid before the value
        br#"{"a":1} trailing"#,              // data after the document
        br#"{"a":"x\q"}"#,                   // bad escape in the value
        b"{\"a\":\"ctrl\x01char\"}",         // control character in a string
        b"{\"a\":\"\xff\xfe\"}",             // invalid UTF-8
        br#"{"a":"\ud800"}"#,                // lone surrogate
        br#"{"a":01}"#,                      // leading zero
        br#"{"a":1,"a":2}"#,                 // duplicate key
    ];
    for d in docs {
        let mut ours = Ours::new(&["a"]);
        let mut sonic = Sonic::new(&["a"]);
        let show = |r: Option<Vec<Option<Vec<u8>>>>| match r {
            None => "invalid".to_string(),
            Some(v) => match &v[0] {
                None => "missing".to_string(),
                Some(t) => format!("{:?}", String::from_utf8_lossy(t)),
            },
        };
        println!("  {:<24} {:<14} | {}", String::from_utf8_lossy(d), show(ours.run(d)), show(sonic.values(d, true)));
    }
}
