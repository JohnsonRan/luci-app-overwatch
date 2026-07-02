use crate::api::{client_api, api_point, status_api, summary};
use crate::config::Config;
use crate::db::{SqliteStore, Store};
use crate::history::{HistTier, HistoryService};
use crate::probe::ProbeStatus;
use crate::recorder::Recorder;
use crate::snapshot::ClientView;
use serde_json::{json, Value};

pub struct Shared {
    pub clients: Vec<ClientView>,
    pub recorder: Recorder,
    pub store: SqliteStore,
    pub config: Config,
    pub probes: ProbeStatus,
}

fn hist_tier(s: &str) -> Option<HistTier> {
    match s {
        "realtime" => Some(HistTier::Realtime),
        "day" => Some(HistTier::Day),
        "week" => Some(HistTier::Week),
        "month" => Some(HistTier::Month),
        _ => None,
    }
}

pub fn dispatch(shared: &mut Shared, method: &str, params: &Value) -> Value {
    match method {
        "clients" => {
            let list: Vec<_> = shared.clients.iter().map(client_api).collect();
            json!({ "clients": list })
        }
        "status" => {
            json!(status_api(shared.config.enabled, &shared.probes, env!("CARGO_PKG_VERSION")))
        }
        "summary" => {
            let top_n = params.get("top_n").and_then(|v| v.as_u64()).unwrap_or(5).min(10_000) as usize;
            json!(summary(&shared.clients, top_n))
        }
        "history" => {
            let tier = match params.get("tier").and_then(|v| v.as_str()).and_then(hist_tier) {
                Some(t) => t,
                None => return json!({ "error": "missing or invalid tier" }),
            };
            let client = params.get("client").and_then(|v| v.as_str());
            let from = params.get("from").and_then(|v| v.as_i64()).unwrap_or(0);
            let to = params.get("to").and_then(|v| v.as_i64()).unwrap_or(i64::MAX);
            let max_points = params.get("max_points").and_then(|v| v.as_u64()).unwrap_or(500).min(100_000) as usize;
            let svc = HistoryService::new(&shared.recorder, &shared.store);
            let series = svc.series(tier, client, from, to, max_points);
            let points: Vec<_> = series.iter().map(api_point).collect();
            json!({ "points": points })
        }
        "dns_top" => {
            let client = params.get("client").and_then(|v| v.as_str());
            let from = params.get("from").and_then(|v| v.as_i64()).unwrap_or(0);
            let to = params.get("to").and_then(|v| v.as_i64()).unwrap_or(i64::MAX);
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(shared.config.dns_top_n)
                .min(10_000);
            let top = match shared.store.dns_top(client, from, to, limit) {
                Ok(t) => t,
                Err(e) => return json!({ "error": e.to_string() }),
            };
            let domains: Vec<_> = top
                .into_iter()
                .map(|(domain, count)| json!({ "domain": domain, "count": count }))
                .collect();
            json!({ "domains": domains })
        }
        _ => json!({ "error": "unknown method" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config;
    use crate::db::{SqliteStore, Store};
    use crate::probe::ProbeStatus;
    use crate::recorder::Recorder;
    use crate::snapshot::ClientView;
    use serde_json::json;

    fn view(mac: &str, rx_bps: u64) -> ClientView {
        ClientView {
            mac: mac.to_string(), ip: "192.168.1.5".to_string(), host: String::new(),
            rx_bps, tx_bps: 0, rx_total: rx_bps, tx_total: 0,
            conns_tcp: 0, conns_udp: 0,
        }
    }

    fn shared() -> Shared {
        Shared {
            clients: vec![view("aa", 1000), view("bb", 50)],
            recorder: Recorder::new(300, 1440),
            store: SqliteStore::open_in_memory().unwrap(),
            config: default_config(),
            probes: ProbeStatus::default(),
        }
    }

    #[test]
    fn clients_method_returns_bits() {
        let mut s = shared();
        let r = dispatch(&mut s, "clients", &json!({}));
        let arr = r["clients"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // aa rx_bps 1000 bytes/s -> 8000 bits/s
        let aa = arr.iter().find(|c| c["mac"] == "aa").unwrap();
        assert_eq!(aa["rx_bps"], 8000);
    }

    #[test]
    fn history_defaults_apply() {
        let mut s = shared();
        // tier valid, all range params absent -> from 0 / to MAX / max_points 500.
        let r = dispatch(&mut s, "history", &json!({"tier":"realtime"}));
        assert!(r.get("points").is_some());
        // invalid tier -> error, not panic/default.
        let e = dispatch(&mut s, "history", &json!({"tier":"century"}));
        assert!(e.get("error").is_some());
    }

    #[test]
    fn unknown_method_errors() {
        let mut s = shared();
        let r = dispatch(&mut s, "bogus", &json!({}));
        assert!(r.get("error").is_some());
    }

    #[test]
    fn dns_top_returns_sorted_domains() {
        let mut s = shared();
        s.store.add_dns_counts(0, &[
            ("aa".to_string(), "example.com".to_string(), 5),
            ("aa".to_string(), "other.com".to_string(), 2),
        ]).unwrap();
        let r = dispatch(&mut s, "dns_top", &json!({}));
        let domains = r["domains"].as_array().unwrap();
        assert_eq!(domains.len(), 2);
        assert_eq!(domains[0]["domain"], "example.com");
        assert_eq!(domains[0]["count"], 5);
    }

    #[test]
    fn dns_top_filters_by_client_param() {
        let mut s = shared();
        s.store.add_dns_counts(0, &[
            ("aa".to_string(), "a.com".to_string(), 1),
            ("bb".to_string(), "b.com".to_string(), 1),
        ]).unwrap();
        let r = dispatch(&mut s, "dns_top", &json!({"client": "aa"}));
        let domains = r["domains"].as_array().unwrap();
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0]["domain"], "a.com");
    }

    #[test]
    fn dns_top_empty_result_is_empty_array_not_error() {
        let mut s = shared();
        let r = dispatch(&mut s, "dns_top", &json!({}));
        assert_eq!(r["domains"].as_array().unwrap().len(), 0);
    }
}
