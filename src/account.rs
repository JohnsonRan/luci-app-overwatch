use crate::classify::{ClientDelta, LanClassifier};
use crate::flow::{Flow, Proto};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientStats {
    pub rx_total: u64,
    pub tx_total: u64,
    // NOTE: `*_bps` are BYTES/s, not bits. The API must multiply by 8 for bits/s.
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub conns_tcp: u32,
    pub conns_udp: u32,
}

#[derive(Clone, Copy)]
struct FlowBytes {
    rx: u64,
    tx: u64,
}

pub struct Accountant {
    classifier: LanClassifier,
    last_flows: HashMap<u64, FlowBytes>,
    stats: HashMap<IpAddr, ClientStats>,
    last_ts: Option<f64>,
}

impl Accountant {
    pub fn new(classifier: LanClassifier) -> Self {
        Accountant {
            classifier,
            last_flows: HashMap::new(),
            stats: HashMap::new(),
            last_ts: None,
        }
    }

    pub fn stats(&self) -> &HashMap<IpAddr, ClientStats> {
        &self.stats
    }

    pub fn update(&mut self, flows: &[Flow], now_secs: f64) {
        let elapsed = self.last_ts.map(|t| now_secs - t).filter(|d| *d > 0.0);

        // Reset per-scan rate accumulators and connection counts.
        let mut interval_rx: HashMap<IpAddr, u64> = HashMap::new();
        let mut interval_tx: HashMap<IpAddr, u64> = HashMap::new();
        for s in self.stats.values_mut() {
            s.rx_bps = 0;
            s.tx_bps = 0;
            s.conns_tcp = 0;
            s.conns_udp = 0;
        }

        let mut seen: HashMap<u64, FlowBytes> = HashMap::with_capacity(flows.len());

        for flow in flows {
            let Some(d): Option<ClientDelta> = self.classifier.attribute(flow) else {
                continue;
            };
            let entry = self.stats.entry(d.client).or_default();
            match d.proto {
                Proto::Tcp => entry.conns_tcp += 1,
                Proto::Udp => entry.conns_udp += 1,
                Proto::Other => {}
            }

            let (drx, dtx) = match self.last_flows.get(&flow.flow_key) {
                Some(prev) => (
                    d.rx.saturating_sub(prev.rx),
                    d.tx.saturating_sub(prev.tx),
                ),
                None => (d.rx, d.tx),
            };
            entry.rx_total += drx;
            entry.tx_total += dtx;
            *interval_rx.entry(d.client).or_default() += drx;
            *interval_tx.entry(d.client).or_default() += dtx;

            seen.insert(flow.flow_key, FlowBytes { rx: d.rx, tx: d.tx });
        }

        if let Some(dt) = elapsed {
            for (client, bytes) in &interval_rx {
                if let Some(s) = self.stats.get_mut(client) {
                    s.rx_bps = (*bytes as f64 / dt) as u64;
                }
            }
            for (client, bytes) in &interval_tx {
                if let Some(s) = self.stats.get_mut(client) {
                    s.tx_bps = (*bytes as f64 / dt) as u64;
                }
            }
        }

        self.last_flows = seen;
        self.last_ts = Some(now_secs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::IpNet;

    fn flow(key: u64, src: &str, dst: &str, ob: u64, rb: u64, proto: Proto) -> Flow {
        Flow {
            proto,
            orig_src: src.parse().unwrap(),
            orig_dst: dst.parse().unwrap(),
            orig_bytes: ob,
            reply_bytes: rb,
            flow_key: key,
        }
    }

    fn accountant() -> Accountant {
        Accountant::new(LanClassifier::new(vec![IpNet {
            addr: "192.168.1.0".parse().unwrap(),
            prefix_len: 24,
        }]))
    }

    #[test]
    fn accumulates_deltas_across_scans() {
        let mut a = accountant();
        let client: IpAddr = "192.168.1.10".parse().unwrap();

        // Scan 1 at t=0: flow has tx=100 (orig), rx=1000 (reply).
        a.update(&[flow(1, "192.168.1.10", "9.9.9.9", 100, 1000, Proto::Tcp)], 0.0);
        assert_eq!(a.stats()[&client].tx_total, 100);
        assert_eq!(a.stats()[&client].rx_total, 1000);

        // Scan 2 at t=2: same flow grew to tx=300, rx=5000 → deltas 200 / 4000.
        a.update(&[flow(1, "192.168.1.10", "9.9.9.9", 300, 5000, Proto::Tcp)], 2.0);
        assert_eq!(a.stats()[&client].tx_total, 300);
        assert_eq!(a.stats()[&client].rx_total, 5000);
        // rate = delta / elapsed: rx 4000 / 2s = 2000 Bps, tx 200 / 2s = 100 Bps
        assert_eq!(a.stats()[&client].rx_bps, 2000);
        assert_eq!(a.stats()[&client].tx_bps, 100);
    }

    #[test]
    fn new_flow_after_old_closes_does_not_regress() {
        let mut a = accountant();
        let client: IpAddr = "192.168.1.10".parse().unwrap();

        a.update(&[flow(1, "192.168.1.10", "9.9.9.9", 0, 8000, Proto::Tcp)], 0.0);
        assert_eq!(a.stats()[&client].rx_total, 8000);

        // Old flow gone, brand-new flow (different key) starts at rx=500.
        a.update(&[flow(2, "192.168.1.10", "9.9.9.9", 0, 500, Proto::Tcp)], 2.0);
        // Total must grow by the new flow's bytes, never drop.
        assert_eq!(a.stats()[&client].rx_total, 8500);
    }

    #[test]
    fn counts_active_connections_by_proto() {
        let mut a = accountant();
        let client: IpAddr = "192.168.1.10".parse().unwrap();
        a.update(
            &[
                flow(1, "192.168.1.10", "9.9.9.9", 1, 1, Proto::Tcp),
                flow(2, "192.168.1.10", "8.8.8.8", 1, 1, Proto::Udp),
                flow(3, "192.168.1.10", "1.1.1.1", 1, 1, Proto::Tcp),
            ],
            0.0,
        );
        assert_eq!(a.stats()[&client].conns_tcp, 2);
        assert_eq!(a.stats()[&client].conns_udp, 1);
    }
}
