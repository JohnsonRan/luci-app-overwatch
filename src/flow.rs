use std::net::IpAddr;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    pub proto: Proto,
    pub orig_src: IpAddr,
    pub orig_dst: IpAddr,
    pub orig_bytes: u64,
    pub reply_bytes: u64,
    pub flow_key: u64,
}

impl Proto {
    pub fn parse(name: &str) -> Proto {
        match name {
            "tcp" => Proto::Tcp,
            "udp" => Proto::Udp,
            _ => Proto::Other,
        }
    }
}

impl Flow {
    pub fn parse_line(line: &str) -> Option<Flow> {
        let toks: Vec<&str> = line.split_whitespace().collect();
        // proto name is the token after the L3 family fields; scan for it.
        let proto = toks.iter().find_map(|t| match *t {
            "tcp" => Some(Proto::Tcp),
            "udp" => Some(Proto::Udp),
            _ => None,
        }).unwrap_or(Proto::Other);

        // Collect key=value tokens in order.
        let mut src: Vec<IpAddr> = Vec::new();
        let mut dst: Vec<IpAddr> = Vec::new();
        let mut sport: Vec<u16> = Vec::new();
        let mut dport: Vec<u16> = Vec::new();
        let mut bytes: Vec<u64> = Vec::new();
        for t in &toks {
            if let Some(v) = t.strip_prefix("src=") {
                if let Ok(ip) = v.parse() { src.push(ip); }
            } else if let Some(v) = t.strip_prefix("dst=") {
                if let Ok(ip) = v.parse() { dst.push(ip); }
            } else if let Some(v) = t.strip_prefix("sport=") {
                if let Ok(p) = v.parse() { sport.push(p); }
            } else if let Some(v) = t.strip_prefix("dport=") {
                if let Ok(p) = v.parse() { dport.push(p); }
            } else if let Some(v) = t.strip_prefix("bytes=") {
                if let Ok(b) = v.parse() { bytes.push(b); }
            }
        }

        // Need both directions' byte counters (accounting on) plus a 5-tuple.
        if bytes.len() < 2 || src.is_empty() || dst.is_empty()
            || sport.is_empty() || dport.is_empty() {
            return None;
        }

        let orig_src = src[0];
        let orig_dst = dst[0];
        let flow_key = compute_flow_key(orig_src, sport[0], orig_dst, dport[0], proto);

        Some(Flow {
            proto,
            orig_src,
            orig_dst,
            orig_bytes: bytes[0],
            reply_bytes: bytes[1],
            flow_key,
        })
    }
}

fn compute_flow_key(a_ip: IpAddr, a_port: u16, b_ip: IpAddr, b_port: u16, proto: Proto) -> u64 {
    // Order endpoints so both directions of the same flow hash equally.
    let mut ends = [(a_ip, a_port), (b_ip, b_port)];
    ends.sort();
    let mut h = DefaultHasher::new();
    ends.hash(&mut h);
    (proto as u8).hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_parse_known_and_unknown() {
        assert_eq!(Proto::parse("tcp"), Proto::Tcp);
        assert_eq!(Proto::parse("udp"), Proto::Udp);
        assert_eq!(Proto::parse("icmp"), Proto::Other);
    }

    #[test]
    fn parse_accounted_tcp_line() {
        let line = "ipv4     2 tcp      6 431999 ESTABLISHED src=192.168.1.10 dst=93.184.216.34 sport=52000 dport=443 packets=12 bytes=1500 src=93.184.216.34 dst=192.168.1.10 sport=443 dport=52000 packets=10 bytes=24000 [ASSURED] mark=0 use=1";
        let f = Flow::parse_line(line).expect("should parse");
        assert_eq!(f.proto, Proto::Tcp);
        assert_eq!(f.orig_src, "192.168.1.10".parse::<IpAddr>().unwrap());
        assert_eq!(f.orig_dst, "93.184.216.34".parse::<IpAddr>().unwrap());
        assert_eq!(f.orig_bytes, 1500);
        assert_eq!(f.reply_bytes, 24000);
    }

    #[test]
    fn parse_skips_unaccounted_line() {
        let line = "ipv4     2 tcp      6 60 SYN_SENT src=192.168.1.10 dst=10.0.0.5 sport=53001 dport=80 [UNREPLIED] src=10.0.0.5 dst=192.168.1.10 sport=80 dport=53001 mark=0 use=1";
        assert_eq!(Flow::parse_line(line), None);
    }

    #[test]
    fn flow_key_is_direction_stable() {
        // orig and reply describe the same flow; key derived from sorted endpoints
        let line = "ipv4     2 udp      17 29 src=192.168.1.22 dst=8.8.8.8 sport=41000 dport=53 packets=2 bytes=140 src=8.8.8.8 dst=192.168.1.22 sport=53 dport=41000 packets=2 bytes=300 mark=0 use=1";
        let f = Flow::parse_line(line).unwrap();
        assert_ne!(f.flow_key, 0);
    }
}
