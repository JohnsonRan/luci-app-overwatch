use crate::account::ClientStats;
use crate::names::NameTable;
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClientView {
    pub mac: String,
    pub ip: String,
    pub host: String,
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub conns_tcp: u32,
    pub conns_udp: u32,
}

pub fn build_snapshot(
    stats: &HashMap<IpAddr, ClientStats>,
    names: &NameTable,
) -> Vec<ClientView> {
    let mut views: Vec<ClientView> = stats
        .iter()
        .map(|(ip, s)| {
            let ip_str = ip.to_string();
            let mac = names.mac_for(*ip).map(|m| m.to_string());
            let host = mac
                .as_deref()
                .and_then(|m| names.host_for_mac(m))
                .unwrap_or("")
                .to_string();
            ClientView {
                mac: mac.unwrap_or_else(|| ip_str.clone()),
                ip: ip_str,
                host,
                rx_bps: s.rx_bps,
                tx_bps: s.tx_bps,
                rx_total: s.rx_total,
                tx_total: s.tx_total,
                conns_tcp: s.conns_tcp,
                conns_udp: s.conns_udp,
            }
        })
        .collect();

    views.sort_by(|a, b| (b.rx_bps + b.tx_bps).cmp(&(a.rx_bps + a.tx_bps)));
    views
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> NameTable {
        NameTable::from_text(
            "1719628800 aa:bb:cc:dd:ee:01 192.168.1.10 johns-laptop x\n",
            "IP address HW type Flags HW address Mask Device\n",
        )
    }

    #[test]
    fn builds_view_joined_with_names() {
        let mut stats: HashMap<IpAddr, ClientStats> = HashMap::new();
        stats.insert(
            "192.168.1.10".parse().unwrap(),
            ClientStats { rx_total: 5000, tx_total: 300, rx_bps: 2000, tx_bps: 100, conns_tcp: 2, conns_udp: 1 },
        );
        let views = build_snapshot(&stats, &names());
        assert_eq!(views.len(), 1);
        let v = &views[0];
        assert_eq!(v.mac, "aa:bb:cc:dd:ee:01");
        assert_eq!(v.ip, "192.168.1.10");
        assert_eq!(v.host, "johns-laptop");
        assert_eq!(v.rx_bps, 2000);
        assert_eq!(v.conns_tcp, 2);
    }

    #[test]
    fn sorted_by_total_speed_desc() {
        let mut stats: HashMap<IpAddr, ClientStats> = HashMap::new();
        stats.insert("192.168.1.10".parse().unwrap(),
            ClientStats { rx_bps: 100, tx_bps: 0, ..Default::default() });
        stats.insert("192.168.1.22".parse().unwrap(),
            ClientStats { rx_bps: 900, tx_bps: 50, ..Default::default() });
        let views = build_snapshot(&stats, &names());
        assert_eq!(views[0].ip, "192.168.1.22");
        assert_eq!(views[1].ip, "192.168.1.10");
    }

    #[test]
    fn unknown_mac_falls_back_to_ip_identity() {
        let mut stats: HashMap<IpAddr, ClientStats> = HashMap::new();
        stats.insert("192.168.1.99".parse().unwrap(), ClientStats::default());
        let views = build_snapshot(&stats, &names());
        assert_eq!(views[0].mac, "192.168.1.99");
        assert_eq!(views[0].host, "");
    }

    #[test]
    fn serializes_to_expected_json_keys() {
        let mut stats: HashMap<IpAddr, ClientStats> = HashMap::new();
        stats.insert("192.168.1.10".parse().unwrap(), ClientStats::default());
        let views = build_snapshot(&stats, &names());
        let json = serde_json::to_string(&views[0]).unwrap();
        assert!(json.contains("\"mac\":"));
        assert!(json.contains("\"rx_bps\":"));
        assert!(json.contains("\"conns_udp\":"));
    }
}
