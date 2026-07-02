use std::net::IpAddr;

// The actual termination guarantee: a pointer chain can revisit an earlier
// offset (e.g. a label followed by a pointer back into that same label),
// so the per-jump "ptr must be < current pos" check alone can still cycle.
// This cap is what bounds the loop.
const MAX_LABEL_JUMPS: usize = 5;

/// Parses a raw IPv4 or IPv6 packet and, if it is a UDP query addressed to
/// port 53 (client -> resolver direction only — response packets, which have
/// dst port != 53, are never touched, so QR-bit/answer parsing is unneeded),
/// returns the source IP and the first question's domain name.
pub fn parse_dns_query(ip_packet: &[u8]) -> Option<(IpAddr, String)> {
    let (src_ip, proto, payload) = parse_ip_header(ip_packet)?;
    if proto != 17 {
        return None; // UDP only
    }
    let dns_msg = parse_udp_header(payload)?;
    let domain = parse_dns_qname(dns_msg)?;
    Some((src_ip, domain))
}

fn parse_ip_header(pkt: &[u8]) -> Option<(IpAddr, u8, &[u8])> {
    let version = pkt.first()? >> 4;
    match version {
        4 => parse_ipv4(pkt),
        6 => parse_ipv6(pkt),
        _ => None,
    }
}

fn parse_ipv4(pkt: &[u8]) -> Option<(IpAddr, u8, &[u8])> {
    if pkt.len() < 20 {
        return None;
    }
    let ihl = (pkt[0] & 0x0f) as usize * 4;
    if ihl < 20 || pkt.len() < ihl {
        return None;
    }
    let proto = pkt[9];
    let src = IpAddr::from([pkt[12], pkt[13], pkt[14], pkt[15]]);
    Some((src, proto, &pkt[ihl..]))
}

fn parse_ipv6(pkt: &[u8]) -> Option<(IpAddr, u8, &[u8])> {
    if pkt.len() < 40 {
        return None;
    }
    let next_header = pkt[6];
    let mut src = [0u8; 16];
    src.copy_from_slice(&pkt[8..24]);
    Some((IpAddr::from(src), next_header, &pkt[40..]))
}

fn parse_udp_header(pkt: &[u8]) -> Option<&[u8]> {
    if pkt.len() < 8 {
        return None;
    }
    let dst_port = u16::from_be_bytes([pkt[2], pkt[3]]);
    if dst_port != 53 {
        return None;
    }
    Some(&pkt[8..])
}

fn parse_dns_qname(msg: &[u8]) -> Option<String> {
    if msg.len() < 12 {
        return None; // shorter than the fixed DNS header
    }
    let mut pos = 12usize;
    let mut labels: Vec<String> = Vec::new();
    let mut jumps = 0usize;

    loop {
        if pos >= msg.len() {
            return None;
        }
        let len = msg[pos];
        if len == 0 {
            break;
        } else if len & 0xC0 == 0xC0 {
            if pos + 1 >= msg.len() {
                return None;
            }
            jumps += 1;
            if jumps > MAX_LABEL_JUMPS {
                return None;
            }
            let ptr = (((len as usize) & 0x3F) << 8) | msg[pos + 1] as usize;
            if ptr >= pos {
                return None; // reject non-backward pointers outright
            }
            pos = ptr;
        } else if len & 0xC0 != 0 {
            return None; // reserved length-prefix bits set: malformed
        } else {
            let label_len = len as usize;
            let start = pos + 1;
            let end = start + label_len;
            if end > msg.len() {
                return None;
            }
            let label = std::str::from_utf8(&msg[start..end]).ok()?;
            labels.push(label.to_string());
            pos = end;
        }
    }

    Some(labels.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_qname(labels: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for l in labels {
            out.push(l.len() as u8);
            out.extend_from_slice(l.as_bytes());
        }
        out.push(0);
        out
    }

    fn build_ipv4_udp_dns(proto: u8, dst_port: u16, dns_body: &[u8]) -> Vec<u8> {
        let mut dns = vec![0u8; 12]; // 12-byte DNS header, zeroed (fields unused by parser)
        dns.extend_from_slice(dns_body);

        let mut l4 = Vec::new();
        if proto == 17 {
            l4.extend_from_slice(&1234u16.to_be_bytes()); // src port
            l4.extend_from_slice(&dst_port.to_be_bytes()); // dst port
            let udp_len = (8 + dns.len()) as u16;
            l4.extend_from_slice(&udp_len.to_be_bytes());
            l4.extend_from_slice(&0u16.to_be_bytes()); // checksum, unchecked
            l4.extend_from_slice(&dns);
        } else {
            // minimal TCP header stand-in (20 bytes) + payload; parser must reject on proto alone
            l4.extend_from_slice(&[0u8; 20]);
            l4.extend_from_slice(&dns);
        }

        let mut ip = vec![0u8; 20];
        ip[0] = 0x45; // version 4, IHL 5 (20 bytes, no options)
        let total_len = (20 + l4.len()) as u16;
        ip[2..4].copy_from_slice(&total_len.to_be_bytes());
        ip[9] = proto;
        ip[12..16].copy_from_slice(&[192, 168, 1, 50]); // src
        ip[16..20].copy_from_slice(&[192, 168, 1, 1]);  // dst
        ip.extend_from_slice(&l4);
        ip
    }

    fn build_ipv6_udp_dns(dst_port: u16, dns_body: &[u8]) -> Vec<u8> {
        let mut dns = vec![0u8; 12];
        dns.extend_from_slice(dns_body);

        let mut udp = Vec::new();
        udp.extend_from_slice(&1234u16.to_be_bytes());
        udp.extend_from_slice(&dst_port.to_be_bytes());
        let udp_len = (8 + dns.len()) as u16;
        udp.extend_from_slice(&udp_len.to_be_bytes());
        udp.extend_from_slice(&0u16.to_be_bytes());
        udp.extend_from_slice(&dns);

        let mut ip = vec![0u8; 40];
        ip[0] = 0x60; // version 6
        let payload_len = udp.len() as u16;
        ip[4..6].copy_from_slice(&payload_len.to_be_bytes());
        ip[6] = 17; // next header = UDP
        ip[8..24].copy_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]); // src
        ip[24..40].copy_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]); // dst
        ip.extend_from_slice(&udp);
        ip
    }

    #[test]
    fn parses_valid_ipv4_udp53_query() {
        let body = encode_qname(&["example", "com"]);
        let pkt = build_ipv4_udp_dns(17, 53, &body);
        let (ip, domain) = parse_dns_query(&pkt).unwrap();
        assert_eq!(ip, "192.168.1.50".parse::<std::net::IpAddr>().unwrap());
        assert_eq!(domain, "example.com");
    }

    #[test]
    fn parses_valid_ipv6_udp53_query() {
        let body = encode_qname(&["example", "org"]);
        let pkt = build_ipv6_udp_dns(53, &body);
        let (ip, domain) = parse_dns_query(&pkt).unwrap();
        assert_eq!(ip, "fe80::1".parse::<std::net::IpAddr>().unwrap());
        assert_eq!(domain, "example.org");
    }

    #[test]
    fn multi_label_domain_joins_with_dots() {
        let body = encode_qname(&["a", "b", "c", "example", "net"]);
        let pkt = build_ipv4_udp_dns(17, 53, &body);
        let (_, domain) = parse_dns_query(&pkt).unwrap();
        assert_eq!(domain, "a.b.c.example.net");
    }

    #[test]
    fn root_domain_yields_empty_string() {
        let body = encode_qname(&[]); // just the terminating 0
        let pkt = build_ipv4_udp_dns(17, 53, &body);
        let (_, domain) = parse_dns_query(&pkt).unwrap();
        assert_eq!(domain, "");
    }

    #[test]
    fn non_53_destination_port_is_ignored() {
        let body = encode_qname(&["example", "com"]);
        let pkt = build_ipv4_udp_dns(17, 80, &body);
        assert!(parse_dns_query(&pkt).is_none());
    }

    #[test]
    fn tcp_53_is_ignored() {
        let body = encode_qname(&["example", "com"]);
        let pkt = build_ipv4_udp_dns(6, 53, &body); // proto 6 = TCP
        assert!(parse_dns_query(&pkt).is_none());
    }

    #[test]
    fn truncated_packet_returns_none_not_panic() {
        let body = encode_qname(&["example", "com"]);
        let pkt = build_ipv4_udp_dns(17, 53, &body);
        for cut in [0, 1, 10, 20, 27, pkt.len() - 1] {
            assert!(parse_dns_query(&pkt[..cut]).is_none(), "cut at {cut} should not panic/parse");
        }
    }

    #[test]
    fn malformed_forward_pointing_compression_pointer_does_not_loop() {
        // First label byte is a compression pointer (0xC0 prefix) pointing at or
        // after its own position — must be rejected, not followed (would loop/OOB).
        let dns_body = vec![0xC0, 0x0C]; // points at offset 12 == this pointer's own position
        let pkt = build_ipv4_udp_dns(17, 53, &dns_body);
        assert!(parse_dns_query(&pkt).is_none());
    }
}
