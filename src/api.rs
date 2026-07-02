use crate::probe::ProbeStatus;
use crate::sample::Sample;
use crate::snapshot::ClientView;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClientApi {
    pub mac: String,
    pub ip: String,
    pub host: String,
    pub rx_bps: u64, // bits/s
    pub tx_bps: u64, // bits/s
    pub rx_total: u64, // bytes
    pub tx_total: u64, // bytes
    pub conns_tcp: u32,
    pub conns_udp: u32,
}

pub fn client_api(v: &ClientView) -> ClientApi {
    ClientApi {
        mac: v.mac.clone(),
        ip: v.ip.clone(),
        host: v.host.clone(),
        rx_bps: v.rx_bps.saturating_mul(8),
        tx_bps: v.tx_bps.saturating_mul(8),
        rx_total: v.rx_total,
        tx_total: v.tx_total,
        conns_tcp: v.conns_tcp,
        conns_udp: v.conns_udp,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiPoint {
    pub ts: i64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_peak_bits: u64,
    pub tx_peak_bits: u64,
    pub conns: u32,
}

pub fn api_point(s: &Sample) -> ApiPoint {
    ApiPoint {
        ts: s.ts,
        rx_bytes: s.rx_bytes,
        tx_bytes: s.tx_bytes,
        rx_peak_bits: s.rx_peak.saturating_mul(8),
        tx_peak_bits: s.tx_peak.saturating_mul(8),
        conns: s.conns,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopTalker {
    pub mac: String,
    pub rx_total: u64,
    pub tx_total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub top: Vec<TopTalker>,
}

pub fn summary(clients: &[ClientView], top_n: usize) -> Summary {
    let total_rx_bytes = clients.iter().map(|c| c.rx_total).fold(0u64, u64::saturating_add);
    let total_tx_bytes = clients.iter().map(|c| c.tx_total).fold(0u64, u64::saturating_add);
    let mut talkers: Vec<TopTalker> = clients
        .iter()
        .map(|c| TopTalker { mac: c.mac.clone(), rx_total: c.rx_total, tx_total: c.tx_total })
        .collect();
    talkers.sort_by(|a, b| {
        b.rx_total.saturating_add(b.tx_total).cmp(&a.rx_total.saturating_add(a.tx_total))
    });
    talkers.truncate(top_n);
    Summary { total_rx_bytes, total_tx_bytes, top: talkers }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusApi {
    pub enabled: bool,
    pub conntrack_acct: bool,
    pub btf: bool,
    pub clsact: bool,
    pub hw_stats_advancing: Option<bool>,
    pub version: String,
}

pub fn status_api(enabled: bool, p: &ProbeStatus, version: &str) -> StatusApi {
    StatusApi {
        enabled,
        conntrack_acct: p.conntrack_acct,
        btf: p.btf,
        clsact: p.clsact,
        hw_stats_advancing: p.hw_stats_advancing,
        version: version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::ClientView;

    fn view(mac: &str, rx_bps: u64, tx_bps: u64, rx_total: u64, tx_total: u64) -> ClientView {
        ClientView {
            mac: mac.to_string(), ip: "192.168.1.5".to_string(), host: "h".to_string(),
            rx_bps, tx_bps, rx_total, tx_total, conns_tcp: 1, conns_udp: 2,
        }
    }

    #[test]
    fn client_api_converts_bps_to_bits() {
        let a = client_api(&view("aa", 1000, 500, 10, 20));
        assert_eq!(a.rx_bps, 8000); // bytes/s * 8
        assert_eq!(a.tx_bps, 4000);
        assert_eq!(a.rx_total, 10);  // totals stay bytes
    }

    #[test]
    fn summary_totals_and_top_n_descending() {
        let clients = vec![
            view("aa", 0, 0, 100, 10),  // sum 110
            view("bb", 0, 0, 50, 500),  // sum 550
            view("cc", 0, 0, 5, 5),     // sum 10
        ];
        let s = summary(&clients, 2);
        assert_eq!(s.total_rx_bytes, 155);
        assert_eq!(s.total_tx_bytes, 515);
        assert_eq!(s.top.len(), 2);
        assert_eq!(s.top[0].mac, "bb"); // largest sum first
        assert_eq!(s.top[1].mac, "aa");
    }

    #[test]
    fn api_point_peaks_to_bits() {
        let p = api_point(&crate::sample::Sample {
            ts: 60, rx_bytes: 1000, tx_bytes: 100, rx_peak: 200, tx_peak: 20, conns: 3,
        });
        assert_eq!(p.ts, 60);
        assert_eq!(p.rx_bytes, 1000);      // volume stays bytes
        assert_eq!(p.tx_bytes, 100);       // volume stays bytes
        assert_eq!(p.rx_peak_bits, 1600);  // 200 * 8
        assert_eq!(p.tx_peak_bits, 160);   // 20 * 8
        assert_eq!(p.conns, 3);
    }
}
