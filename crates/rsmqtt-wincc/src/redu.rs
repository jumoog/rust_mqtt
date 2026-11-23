//! WinCC OA redundancy (two hosts, one active), as the WinCC OA drivers do it:
//!
//! - Every internal DP exists twice: host 1 uses `<dp>`, host 2 `<dp>_2`.
//! - Config, Command and the broker list are written to `<dp>`; the event
//!   manager copies them to `<dp>_2` (copyDpType in config.redu).
//! - Both hosts must not share an MQTT client id: host 2 appends `_2`.
//! - `redu.<option>=<value>` applies to host 2 only; with `redu.clientId`
//!   the `_2` is not appended.

pub const SUFFIX: &str = "_2";

/// The name a host uses: `<base>` on host 1, `<base>_2` on host 2.
pub fn for_host(base: &str, host2: bool) -> String {
    if host2 { format!("{base}{SUFFIX}") } else { base.to_string() }
}

/// TRUE if a name ends with `_2` (and has something before it).
pub fn ends_with_suffix(name: &str) -> bool {
    name.len() > 2 && name.ends_with(SUFFIX)
}

/// `redu.<option>` (any case): the option name.
pub fn split_redu_option(name: &str) -> Option<&str> {
    let b = name.as_bytes();
    (b.len() > 5 && b[..5].eq_ignore_ascii_case(b"redu.")).then(|| &name[5..])
}

/// The client id a host connects with. An empty id (assigned by the broker)
/// stays empty; an explicit redu.clientId is used as it is.
pub fn client_id_for_host(client_id: &str, host2: bool, redu_client_id_given: bool) -> String {
    if !host2 || redu_client_id_given || client_id.is_empty() {
        client_id.to_string()
    } else {
        format!("{client_id}{SUFFIX}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(for_host("_RsMqtt_plant", false), "_RsMqtt_plant");
        assert_eq!(for_host("_RsMqtt_plant", true), "_RsMqtt_plant_2");
        assert_eq!(for_host("_RsMqttDriver_2", true), "_RsMqttDriver_2_2");
        assert!(ends_with_suffix("line_2") && !ends_with_suffix("line_12x") && !ends_with_suffix("_2") && !ends_with_suffix("line2"));
        assert_eq!(split_redu_option("redu.serverURI"), Some("serverURI"));
        assert_eq!(split_redu_option("REDU.clientId"), Some("clientId"));
        assert!(split_redu_option("redu.").is_none() && split_redu_option("serverURI").is_none() && split_redu_option("reduce").is_none());
        assert_eq!(client_id_for_host("WCCOArsmqtt_3_plant", false, false), "WCCOArsmqtt_3_plant");
        assert_eq!(client_id_for_host("WCCOArsmqtt_3_plant", true, false), "WCCOArsmqtt_3_plant_2");
        assert_eq!(client_id_for_host("scada-b", true, true), "scada-b");
        assert_eq!(client_id_for_host("", true, false), "");
    }
}
