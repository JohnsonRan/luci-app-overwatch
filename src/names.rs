use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Default, Clone)]
pub struct NameTable {
    pub ip_to_mac: HashMap<IpAddr, String>,
    pub mac_to_host: HashMap<String, String>,
}

impl NameTable {
    pub fn from_text(leases: &str, arp: &str) -> NameTable {
        let mut t = NameTable::default();

        for line in leases.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 {
                continue;
            }
            let mac = f[1].to_lowercase();
            if let Ok(ip) = f[2].parse::<IpAddr>() {
                t.ip_to_mac.insert(ip, mac.clone());
            }
            let host = f[3];
            if host != "*" && !host.is_empty() {
                t.mac_to_host.insert(mac, host.to_string());
            }
        }

        for line in arp.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 {
                continue;
            }
            let mac = f[3].to_lowercase();
            if mac == "00:00:00:00:00:00" {
                continue;
            }
            if let Ok(ip) = f[0].parse::<IpAddr>() {
                t.ip_to_mac.entry(ip).or_insert(mac);
            }
        }

        t
    }

    pub fn mac_for(&self, ip: IpAddr) -> Option<&str> {
        self.ip_to_mac.get(&ip).map(|s| s.as_str())
    }

    pub fn host_for_mac(&self, mac: &str) -> Option<&str> {
        self.mac_to_host.get(mac).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEASES: &str = include_str!("../tests/fixtures/dhcp.leases");
    const ARP: &str = include_str!("../tests/fixtures/arp.txt");

    #[test]
    fn resolves_hostname_from_lease() {
        let nt = NameTable::from_text(LEASES, ARP);
        let ip: IpAddr = "192.168.1.10".parse().unwrap();
        assert_eq!(nt.mac_for(ip), Some("aa:bb:cc:dd:ee:01"));
        assert_eq!(nt.host_for_mac("aa:bb:cc:dd:ee:01"), Some("johns-laptop"));
    }

    #[test]
    fn star_hostname_is_treated_as_absent() {
        let nt = NameTable::from_text(LEASES, ARP);
        assert_eq!(nt.host_for_mac("aa:bb:cc:dd:ee:02"), None);
    }

    #[test]
    fn arp_supplements_ip_to_mac_for_non_dhcp_host() {
        let nt = NameTable::from_text(LEASES, ARP);
        let ip: IpAddr = "192.168.1.55".parse().unwrap();
        assert_eq!(nt.mac_for(ip), Some("aa:bb:cc:dd:ee:09"));
    }

    #[test]
    fn mac_is_normalized_lowercase() {
        let nt = NameTable::from_text(LEASES, ARP);
        let ip: IpAddr = "192.168.1.22".parse().unwrap();
        assert_eq!(nt.mac_for(ip), Some("aa:bb:cc:dd:ee:02"));
    }
}
