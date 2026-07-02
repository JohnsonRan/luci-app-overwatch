use crate::flow::{Flow, Proto};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpNet {
    pub addr: IpAddr,
    pub prefix_len: u8,
}

impl IpNet {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(host)) => {
                let mask = prefix_mask_v4(self.prefix_len);
                (u32::from(net) & mask) == (u32::from(host) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(host)) => {
                let mask = prefix_mask_v6(self.prefix_len);
                (u128::from(net) & mask) == (u128::from(host) & mask)
            }
            _ => false,
        }
    }
}

fn prefix_mask_v4(len: u8) -> u32 {
    if len == 0 { 0 } else { u32::MAX << (32 - len.min(32)) }
}

fn prefix_mask_v6(len: u8) -> u128 {
    if len == 0 { 0 } else { u128::MAX << (128 - len.min(128)) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientDelta {
    pub client: IpAddr,
    pub rx: u64, // download, toward client
    pub tx: u64, // upload, from client
    pub proto: Proto,
}

pub struct LanClassifier {
    prefixes: Vec<IpNet>,
}

impl LanClassifier {
    pub fn new(prefixes: Vec<IpNet>) -> Self {
        LanClassifier { prefixes }
    }

    fn is_lan(&self, ip: IpAddr) -> bool {
        self.prefixes.iter().any(|n| n.contains(ip))
    }

    pub fn attribute(&self, flow: &Flow) -> Option<ClientDelta> {
        if self.is_lan(flow.orig_src) {
            // client initiated: orig bytes are upload, reply bytes are download
            Some(ClientDelta {
                client: flow.orig_src,
                rx: flow.reply_bytes,
                tx: flow.orig_bytes,
                proto: flow.proto,
            })
        } else if self.is_lan(flow.orig_dst) {
            // remote initiated: orig bytes go toward client = download
            Some(ClientDelta {
                client: flow.orig_dst,
                rx: flow.orig_bytes,
                tx: flow.reply_bytes,
                proto: flow.proto,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lan() -> LanClassifier {
        LanClassifier::new(vec![IpNet {
            addr: "192.168.1.0".parse().unwrap(),
            prefix_len: 24,
        }])
    }

    #[test]
    fn client_is_originator_when_orig_src_is_lan() {
        let f = Flow {
            proto: Proto::Tcp,
            orig_src: "192.168.1.10".parse().unwrap(),
            orig_dst: "93.184.216.34".parse().unwrap(),
            orig_bytes: 1500,   // from client = upload
            reply_bytes: 24000, // to client = download
            flow_key: 1,
        };
        let d = lan().attribute(&f).unwrap();
        assert_eq!(d.client, "192.168.1.10".parse::<IpAddr>().unwrap());
        assert_eq!(d.tx, 1500);
        assert_eq!(d.rx, 24000);
    }

    #[test]
    fn client_is_responder_when_orig_dst_is_lan() {
        let f = Flow {
            proto: Proto::Tcp,
            orig_src: "93.184.216.34".parse().unwrap(),
            orig_dst: "192.168.1.10".parse().unwrap(),
            orig_bytes: 1500,   // toward client = download
            reply_bytes: 24000, // from client = upload
            flow_key: 1,
        };
        let d = lan().attribute(&f).unwrap();
        assert_eq!(d.client, "192.168.1.10".parse::<IpAddr>().unwrap());
        assert_eq!(d.rx, 1500);
        assert_eq!(d.tx, 24000);
    }

    #[test]
    fn non_lan_flow_is_skipped() {
        let f = Flow {
            proto: Proto::Udp,
            orig_src: "8.8.8.8".parse().unwrap(),
            orig_dst: "1.1.1.1".parse().unwrap(),
            orig_bytes: 100,
            reply_bytes: 200,
            flow_key: 1,
        };
        assert!(lan().attribute(&f).is_none());
    }
}
