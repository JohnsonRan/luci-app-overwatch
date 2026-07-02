use crate::classify::IpNet;
use crate::db::{default_retention, Retention};

#[derive(Debug, Clone)]
pub struct Config {
    pub enabled: bool,
    pub sample_interval: u64,
    pub lan_prefixes: Vec<IpNet>,
    pub retention: Retention,
    pub db_path: String,
    pub dns_stats_enabled: bool,
    pub dns_iface: String,
    pub dns_flush_secs: u64,
    pub dns_top_n: u64,
}

pub fn default_config() -> Config {
    Config {
        enabled: true,
        sample_interval: 2,
        lan_prefixes: vec![IpNet { addr: "192.168.1.0".parse().unwrap(), prefix_len: 24 }],
        retention: default_retention(),
        // /etc is persistent flash (unlike /var, tmpfs on OpenWrt) so history survives reboot.
        db_path: "/etc/overwatch/overwatch.db".to_string(),
        dns_stats_enabled: false,
        dns_iface: "br-lan".to_string(),
        dns_flush_secs: 60,
        dns_top_n: 50,
    }
}

// A single parsed `config <type>` section with its option/list lines.
struct Section {
    kind: String,
    options: Vec<(String, String)>, // (key, value), lists repeat the key
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    s.trim_matches(|c| c == '\'' || c == '"').to_string()
}

fn split_sections(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("config ") {
            let kind = rest.split_whitespace().next().unwrap_or("").to_string();
            out.push(Section { kind, options: Vec::new() });
        } else if let Some(rest) = l.strip_prefix("option ").or_else(|| l.strip_prefix("list ")) {
            let mut it = rest.splitn(2, char::is_whitespace);
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                if let Some(sec) = out.last_mut() {
                    sec.options.push((k.to_string(), unquote(v)));
                }
            }
        }
    }
    out
}

fn get<'a>(sec: &'a Section, key: &str) -> Option<&'a str> {
    sec.options.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn get_all<'a>(sec: &'a Section, key: &str) -> Vec<&'a str> {
    sec.options.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
}

fn parse_prefix(s: &str) -> Option<IpNet> {
    let (addr, len) = s.split_once('/')?;
    Some(IpNet { addr: addr.trim().parse().ok()?, prefix_len: len.trim().parse().ok()? })
}

pub fn parse_uci(text: &str) -> Config {
    let mut cfg = default_config();
    let sections = split_sections(text);

    if let Some(g) = sections.iter().find(|s| s.kind == "overwatch") {
        if let Some(v) = get(g, "enabled") {
            cfg.enabled = v == "1";
        }
        if let Some(v) = get(g, "sample_interval").and_then(|v| v.parse().ok()) {
            cfg.sample_interval = v;
        }
        // Reject negative retention (would make prune delete nothing/everything); invalid keeps default.
        if let Some(v) = get(g, "retention_hours_min").and_then(|v| v.parse::<i64>().ok()).filter(|v| *v >= 0) {
            cfg.retention.min_secs = v * 3600;
        }
        if let Some(v) = get(g, "retention_days_hour").and_then(|v| v.parse::<i64>().ok()).filter(|v| *v >= 0) {
            cfg.retention.hour_secs = v * 86400;
        }
        if let Some(v) = get(g, "retention_days_day").and_then(|v| v.parse::<i64>().ok()).filter(|v| *v >= 0) {
            cfg.retention.day_secs = v * 86400;
        }
        let prefixes: Vec<IpNet> = get_all(g, "lan_prefix").iter().filter_map(|p| parse_prefix(p)).collect();
        if !prefixes.is_empty() {
            cfg.lan_prefixes = prefixes;
        }
        if let Some(v) = get(g, "db_path") {
            cfg.db_path = v.to_string();
        }
        if let Some(v) = get(g, "dns_stats_enabled") {
            cfg.dns_stats_enabled = v == "1";
        }
        if let Some(v) = get(g, "dns_iface") {
            cfg.dns_iface = v.to_string();
        }
        if let Some(v) = get(g, "dns_flush_secs").and_then(|v| v.parse().ok()) {
            cfg.dns_flush_secs = v;
        }
        if let Some(v) = get(g, "dns_top_n").and_then(|v| v.parse().ok()) {
            cfg.dns_top_n = v;
        }
    }

    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "
config overwatch 'global'
    option enabled '1'
    option sample_interval '5'
    option retention_hours_min '24'
    option retention_days_hour '20'
    option retention_days_day '200'
    list lan_prefix '192.168.1.0/24'
    list lan_prefix '10.0.0.0/8'
";

    #[test]
    fn parses_global_section() {
        let c = parse_uci(SAMPLE);
        assert!(c.enabled);
        assert_eq!(c.sample_interval, 5);
        assert_eq!(c.lan_prefixes.len(), 2);
        assert_eq!(c.retention.min_secs, 24 * 3600);
        assert_eq!(c.retention.hour_secs, 20 * 86400);
        assert_eq!(c.retention.day_secs, 200 * 86400);
    }

    #[test]
    fn empty_text_yields_defaults() {
        let c = parse_uci("");
        let d = default_config();
        assert_eq!(c.enabled, d.enabled);
        assert_eq!(c.sample_interval, d.sample_interval);
        assert_eq!(c.lan_prefixes.len(), 1); // default 192.168.1.0/24
    }

    #[test]
    fn disabled_flag_applies() {
        let txt = "
config overwatch 'global'
    option enabled '0'
";
        let c = parse_uci(txt);
        assert!(!c.enabled);
    }

    #[test]
    fn negative_retention_falls_back_to_default() {
        let txt = "
config overwatch 'global'
    option retention_hours_min '-5'
";
        let c = parse_uci(txt);
        // negative rejected -> keeps default_retention min_secs (48h)
        assert_eq!(c.retention.min_secs, default_config().retention.min_secs);
    }

    #[test]
    fn new_options_have_persistent_defaults() {
        let d = default_config();
        assert_eq!(d.db_path, "/etc/overwatch/overwatch.db");
    }

    #[test]
    fn parses_new_options_from_uci() {
        let txt = "
config overwatch 'global'
    option db_path '/etc/overwatch/custom.db'
";
        let c = parse_uci(txt);
        assert_eq!(c.db_path, "/etc/overwatch/custom.db");
    }

    #[test]
    fn dns_options_have_documented_defaults() {
        let d = default_config();
        assert!(!d.dns_stats_enabled);
        assert_eq!(d.dns_iface, "br-lan");
        assert_eq!(d.dns_flush_secs, 60);
        assert_eq!(d.dns_top_n, 50);
    }

    #[test]
    fn parses_dns_options_from_uci() {
        let txt = "
config overwatch 'global'
    option dns_stats_enabled '1'
    option dns_iface 'eth0'
    option dns_flush_secs '30'
    option dns_top_n '100'
";
        let c = parse_uci(txt);
        assert!(c.dns_stats_enabled);
        assert_eq!(c.dns_iface, "eth0");
        assert_eq!(c.dns_flush_secs, 30);
        assert_eq!(c.dns_top_n, 100);
    }
}
