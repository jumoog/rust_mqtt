//! Peripheral address syntax of the rsmqtt driver.
//!
//! `<connection>:<topic>[;option[;option...]]`
//!
//! - connection: name of a broker (config file or `_RsMqtt_<name>` DP)
//! - topic: MQTT topic; for input addresses also a filter with `+` / `#` or a
//!   shared subscription (`$share/group/...`); `$state` is the connection
//!   state (1 = connected, 0 = not)
//! - options: `qos=0|1|2`, `retain[=0|1]`, `nolocal`, `rap`, `rh=0|1|2`,
//!   `json[=path]`, `ts=path` (see [`crate::json`])
//!
//! Sparkplug B (host application, see [`crate::sparkplug`]):
//! - `<connection>:$spb/<group>/<edge node>[/<device>];metric=<name>`
//! - `<connection>:$spb/<group>/<edge node>[/<device>];online`

use crate::json;
use crate::sparkplug;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Address {
    pub connection: String,
    /// the topic; for Sparkplug the edge node's subscription filter
    pub topic: String,
    /// -1: connection default
    pub qos: i32,
    /// -1: connection default
    pub retain: i32,
    /// RSMQTT_SUB_* flags
    pub sub_flags: i32,
    /// "$state" pseudo topic
    pub is_state: bool,
    /// json[=path]; the path as written (re-parsed with json::parse_path)
    pub json: bool,
    pub json_path: String,
    /// ts=path (needs json)
    pub has_ts: bool,
    pub ts_path: String,
    pub sparkplug: bool,
    pub spb_group: String,
    pub spb_node: String,
    pub spb_device: String,
    pub spb_metric: String,
    pub spb_online: bool,
}

fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C'))
}

fn parse_bool(v: &str) -> Option<i32> {
    let l = v.to_ascii_lowercase();
    match l.as_str() {
        "1" | "true" | "yes" | "on" => Some(1),
        "0" | "false" | "no" | "off" => Some(0),
        _ => None,
    }
}

/// Removes "$share/<group>/" from a shared subscription filter.
fn strip_share(filter: &str) -> &str {
    match filter.strip_prefix("$share/") {
        None => filter,
        Some(rest) => match rest.find('/') {
            Some(p) => &rest[p + 1..],
            None => "",
        },
    }
}

pub fn has_wildcards(filter: &str) -> bool {
    filter.contains(['+', '#'])
}

/// Filter needs matching (wildcards or shared subscription) instead of an exact comparison.
pub fn is_wildcard_filter(filter: &str) -> bool {
    has_wildcards(filter) || filter.starts_with("$share/")
}

/// Checks a topic filter for SUBSCRIBE: not empty, no NUL, wildcards as
/// whole levels, `$share/<group>/<filter>` with a group without wildcards.
pub fn check_filter(filter: &str) -> Result<(), String> {
    if filter.is_empty() {
        return Err("empty topic filter".into());
    }
    if filter.contains('\0') {
        return Err("a topic filter must not contain NUL".into());
    }
    if let Some(rest) = filter.strip_prefix("$share/") {
        let (group, inner) = rest.split_once('/').unwrap_or((rest, ""));
        if group.is_empty() || has_wildcards(group) {
            return Err("$share/<group>/<filter>: the group must not be empty or contain + #".into());
        }
        if inner.is_empty() {
            return Err("$share/<group>/<filter>: empty filter".into());
        }
    }
    // '#' only as the last level, '+' only as a whole level
    let tb = filter.as_bytes();
    for (i, &c) in tb.iter().enumerate() {
        if c == b'#' && (i + 1 != tb.len() || (i > 0 && tb[i - 1] != b'/')) {
            return Err("'#' must be the last level of a topic filter".into());
        }
        if c == b'+' && ((i > 0 && tb[i - 1] != b'/') || (i + 1 < tb.len() && tb[i + 1] != b'/')) {
            return Err("'+' must occupy a whole topic level".into());
        }
    }
    Ok(())
}

/// MQTT topic matching (including $share/<group>/ filters and the rule that
/// wildcards at the first level do not match $-topics).
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    let filter = strip_share(filter);
    if filter.is_empty() {
        return false;
    }
    if topic.starts_with('$') && (filter.starts_with('+') || filter.starts_with('#')) {
        return false;
    }
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true, // this level and below, incl. the parent ("a/#" matches "a")
            (Some(fl), Some(tl)) => {
                if fl != "+" && fl != tl {
                    return false;
                }
            }
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// Parses an address.
pub fn parse(text: &str) -> Result<Address, String> {
    let mut out = Address { qos: -1, retain: -1, ..Default::default() };
    let text = trim(text);

    let colon = match text.find(':') {
        Some(c) if c > 0 => c,
        _ => return Err("address must be '<connection>:<topic>'".into()),
    };
    out.connection = trim(&text[..colon]).to_string();
    let rest = &text[colon + 1..];
    let mut parts = rest.split(';');
    let topic = parts.next().unwrap_or("");
    if topic.is_empty() {
        return Err("empty topic".into());
    }
    out.topic = topic.to_string();
    out.is_state = topic == "$state";

    // Sparkplug B: $spb/<group>/<edge node>[/<device>]
    if let Some(ids) = topic.strip_prefix("$spb/") {
        let p: Vec<&str> = ids.split('/').collect();
        if p.len() < 2 || p.len() > 3 {
            return Err("Sparkplug address must be $spb/<group>/<edge node>[/<device>]".into());
        }
        if !p.iter().all(|id| sparkplug::valid_id(id)) {
            return Err("Sparkplug group, edge node and device must not be empty or contain / + #".into());
        }
        out.sparkplug = true;
        out.spb_group = p[0].to_string();
        out.spb_node = p[1].to_string();
        if p.len() == 3 {
            out.spb_device = p[2].to_string();
        }
        out.topic = sparkplug::node_filter(&out.spb_group, &out.spb_node);
    }

    check_filter(topic)?;

    for opt in parts {
        let opt = trim(opt);
        if opt.is_empty() {
            continue;
        }
        let (name, val) = match opt.find('=') {
            Some(e) => (trim(&opt[..e]), trim(&opt[e + 1..])),
            None => (trim(opt), ""),
        };
        let is = |n: &str| name.eq_ignore_ascii_case(n);
        let level = |v: &str| -> Option<i32> {
            let b = v.as_bytes();
            (b.len() == 1 && (b'0'..=b'2').contains(&b[0])).then(|| (b[0] - b'0') as i32)
        };
        if is("qos") {
            out.qos = level(val).ok_or("qos must be 0, 1 or 2")?;
        } else if is("retain") {
            out.retain = if val.is_empty() { 1 } else { parse_bool(val).ok_or("retain must be 0 or 1")? };
        } else if is("nolocal") || is("nl") {
            out.sub_flags |= 0x04;
        } else if is("rap") {
            out.sub_flags |= 0x08;
        } else if is("rh") {
            let rh = level(val).ok_or("rh must be 0, 1 or 2")?;
            out.sub_flags = (out.sub_flags & !0x30) | (rh << 4);
        } else if out.sparkplug && is("metric") {
            if val.is_empty() {
                return Err("metric= needs the metric name".into());
            }
            out.spb_metric = val.to_string();
        } else if out.sparkplug && is("online") && val.is_empty() {
            out.spb_online = true;
        } else if is("json") {
            json::parse_path(val)?;
            out.json = true;
            out.json_path = val.to_string();
        } else if is("ts") {
            if val.is_empty() {
                return Err("ts needs a JSON path, e.g. ts=time".into());
            }
            let p = json::parse_path(val).map_err(|e| format!("ts: {}", e))?;
            if p.is_empty() {
                return Err("ts needs a JSON path below the top level, e.g. ts=time".into());
            }
            out.has_ts = true;
            out.ts_path = val.to_string();
        } else {
            return Err(format!("unknown address option '{}'", name));
        }
    }

    if out.sparkplug {
        if out.json {
            return Err("json does not apply to Sparkplug addresses".into());
        }
        if out.spb_online == !out.spb_metric.is_empty() {
            return Err("a Sparkplug address needs either ;metric=<name> or ;online".into());
        }
    }
    if out.has_ts && !out.json {
        return Err("ts needs the json option".into());
    }
    if out.json && out.is_state {
        return Err("$state has no JSON payload".into());
    }
    if out.has_ts {
        // the published object must hold both: neither path may contain the other
        let (jp, tp) = (json::parse_path(&out.json_path)?, json::parse_path(&out.ts_path)?);
        if json::build(&[(&jp, b"0"), (&tp, b"0")]).is_err() {
            return Err("json and ts paths collide".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        for f in ["a", "a/b", "#", "a/#", "+", "a/+/c", "+/+", "$share/g/a/#", "$SYS/#", "/"] {
            assert!(check_filter(f).is_ok(), "{f}");
        }
        for f in ["", "a/#/b", "a#", "a/b#", "a+", "a/+b", "$share/g", "$share/g/", "$share//a", "$share/g+/a", "a\0b"] {
            assert!(check_filter(f).is_err(), "{f:?}");
        }
    }

    fn ok(t: &str) -> Address {
        parse(t).unwrap_or_else(|e| panic!("{t}: {e}"))
    }

    #[test]
    fn syntax() {
        let a = ok("plant:line1/temp");
        assert!(a.connection == "plant" && a.topic == "line1/temp" && a.qos == -1 && a.retain == -1);
        let a = ok(" plant : a/b;qos=2;retain;nolocal;rh=1 ");
        assert!(a.connection == "plant" && a.topic == " a/b"); // the topic is taken verbatim
        assert!(a.qos == 2 && a.retain == 1 && a.sub_flags == (0x04 | 0x10));
        let a = ok("plant:a;retain=0;rap");
        assert!(a.retain == 0 && a.sub_flags == 0x08);
        assert!(ok("plant:$state").is_state);
        assert!(is_wildcard_filter(&ok("plant:a/+/c").topic) && is_wildcard_filter(&ok("plant:$share/g/a/b").topic));
        for bad in ["no-colon", ":topic", "plant:", "plant:a;qos=3", "plant:a;bogus", "plant:a/#/b", "plant:a/b+"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn json_and_ts() {
        let a = ok("plant:s/1;json=data.temp;ts=data.time");
        assert!(a.json && a.has_ts && a.json_path == "data.temp" && a.ts_path == "data.time");
        assert!(ok("plant:s/1;json").json_path.is_empty());
        for bad in ["plant:s/1;ts=time", "plant:s/1;json=a;ts=a.b", "plant:s/1;json;ts=t", "plant:$state;json", "plant:s;json=a..b"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sparkplug() {
        let a = ok("plant:$spb/Factory/Line1/PLC7;metric=Motor/speed");
        assert!(a.sparkplug && a.spb_group == "Factory" && a.spb_node == "Line1" && a.spb_device == "PLC7");
        assert!(a.spb_metric == "Motor/speed" && !a.spb_online && a.topic == "spBv1.0/Factory/+/Line1/#");
        assert!(ok("plant:$spb/Factory/Line1;metric=temp;qos=1").spb_device.is_empty());
        assert!(ok("plant:$spb/Factory/Line1;online").spb_online);
        for bad in ["plant:$spb/Factory/Line1", "plant:$spb/Factory/Line1;metric=x;online", "plant:$spb/Factory;metric=x",
                    "plant:$spb/a/b/c/d;metric=x", "plant:$spb/a//c;metric=x", "plant:$spb/a/b;metric=x;json=v",
                    "plant:$spb/a/b;metric=", "plant:a/b;metric=x"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn matching() {
        let m = topic_matches;
        assert!(m("a/b", "a/b") && !m("a/b", "a/c"));
        assert!(m("a/+/c", "a/b/c") && !m("a/+/c", "a/b/d") && !m("a/+", "a/b/c"));
        assert!(m("a/#", "a") && m("a/#", "a/b/c") && m("#", "a/b"));
        assert!(!m("#", "$SYS/broker") && !m("+/broker", "$SYS/broker") && m("$SYS/#", "$SYS/broker"));
        assert!(m("$share/grp/a/+", "a/b") && !m("$share/grp", "a") && !m("a/b/c", "a/b"));
        assert!(m("+", "") && m("/+", "/x") && m("a//b", "a//b"));
    }
}
