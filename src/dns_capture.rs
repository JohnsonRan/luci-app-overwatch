#[cfg(target_os = "linux")]
use crate::control::Shared;
#[cfg(target_os = "linux")]
use crate::db::Store;
#[cfg(target_os = "linux")]
use crate::dns_parse::parse_dns_query;
#[cfg(target_os = "linux")]
use crate::names::NameTable;
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};

// CaptureState is unconditionally compiled and tested (see spec's testing strategy),
// but only ever *constructed* by the Linux-only `run()` below — on non-Linux builds
// nothing outside #[cfg(test)] touches it, hence #[allow(dead_code)] here rather than
// gating the type itself behind target_os.
#[allow(dead_code)]
const CARDINALITY_CAP: usize = 5000;

/// Pure in-memory accumulator for one flush window. No I/O — testable without root.
#[derive(Default)]
#[allow(dead_code)]
struct CaptureState {
    counts: HashMap<(String, String), u64>, // (mac, domain) -> query count
    seen_domains: HashSet<String>,
}

#[allow(dead_code)]
impl CaptureState {
    fn record(&mut self, mac: String, domain: String) {
        let domain = if self.seen_domains.contains(&domain) {
            domain
        } else if self.seen_domains.len() >= CARDINALITY_CAP {
            "__other__".to_string()
        } else {
            self.seen_domains.insert(domain.clone());
            domain
        };
        *self.counts.entry((mac, domain)).or_insert(0) += 1;
    }

    fn drain(&mut self) -> Vec<(String, String, u64)> {
        let out = self.counts.drain().map(|((mac, domain), count)| (mac, domain, count)).collect();
        self.seen_domains.clear();
        out
    }
}

#[cfg(target_os = "linux")]
fn open_capture_socket(iface: &str, eth_proto: u16) -> std::io::Result<i32> {
    use std::ffi::CString;
    use std::io::Error;
    use std::mem;

    let proto_be = eth_proto.to_be() as i32;
    let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM, proto_be) };
    if fd < 0 {
        return Err(Error::last_os_error());
    }

    let ifname = CString::new(iface).map_err(|e| Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let ifindex = unsafe { libc::if_nametoindex(ifname.as_ptr()) };
    if ifindex == 0 {
        let e = Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }

    let mut addr: libc::sockaddr_ll = unsafe { mem::zeroed() };
    addr.sll_family = libc::AF_PACKET as u16;
    addr.sll_protocol = eth_proto.to_be();
    addr.sll_ifindex = ifindex as i32;

    let ret = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_ll as *const libc::sockaddr,
            mem::size_of::<libc::sockaddr_ll>() as u32,
        )
    };
    if ret < 0 {
        let e = Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }

    // Non-blocking: run()'s poll() loop is what provides the periodic wakeup
    // (see below) — a blocking recv per-socket would starve whichever address
    // family is quieter, since the two sockets are drained round-robin.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        let e = Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }

    Ok(fd)
}

#[cfg(target_os = "linux")]
fn recv_packet(fd: i32, buf: &mut [u8]) -> Option<usize> {
    let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
    if n > 0 { Some(n as usize) } else { None } // EAGAIN/error/empty: caller just loops again
}

#[cfg(target_os = "linux")]
fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Runs forever on its own thread (spawned from main.rs, same pattern as
/// server::serve). Independent of the conntrack scan loop: a bug here must
/// not affect traffic monitoring.
#[cfg(target_os = "linux")]
pub fn run(iface: &str, flush_secs: u64, shared: Arc<Mutex<Shared>>) {
    let sock_v4 = match open_capture_socket(iface, libc::ETH_P_IP as u16) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("dns capture: ipv4 socket open on {iface} failed: {e}");
            return;
        }
    };
    let sock_v6 = open_capture_socket(iface, libc::ETH_P_IPV6 as u16).unwrap_or_else(|e| {
        eprintln!("dns capture: ipv6 socket open on {iface} failed: {e} (continuing v4-only)");
        -1
    });

    let mut state = CaptureState::default();
    let mut names = NameTable::from_text(
        &std::fs::read_to_string("/tmp/dhcp.leases").unwrap_or_default(),
        &std::fs::read_to_string("/proc/net/arp").unwrap_or_default(),
    );
    let mut last_flush = now_secs();
    let mut buf = [0u8; 4096];

    let mut pollfds: Vec<libc::pollfd> = [sock_v4, sock_v6]
        .into_iter()
        .filter(|&fd| fd >= 0)
        .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 })
        .collect();

    loop {
        for pfd in &mut pollfds {
            pfd.revents = 0;
        }
        // 1s timeout so the loop wakes periodically to flush/refresh even with
        // no matching traffic on either socket, without a second timer thread.
        let n_ready = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 1000) };
        if n_ready > 0 {
            for pfd in &pollfds {
                if pfd.revents & libc::POLLIN == 0 {
                    continue;
                }
                // Non-blocking socket: drain until EAGAIN so a burst on one
                // family can't starve the other or delay the next poll() tick.
                while let Some(n) = recv_packet(pfd.fd, &mut buf) {
                    if let Some((ip, domain)) = parse_dns_query(&buf[..n]) {
                        if let Some(mac) = names.mac_for(ip) {
                            state.record(mac.to_string(), domain);
                        }
                    }
                }
            }
        }

        let now = now_secs();
        if now - last_flush >= flush_secs as i64 {
            let counts = state.drain();
            if !counts.is_empty() {
                let ts_day = now - (now % 86400);
                let g = shared.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(e) = g.store.add_dns_counts(ts_day, &counts) {
                    eprintln!("dns capture: add_dns_counts failed: {e}");
                }
            }
            names = NameTable::from_text(
                &std::fs::read_to_string("/tmp/dhcp.leases").unwrap_or_default(),
                &std::fs::read_to_string("/proc/net/arp").unwrap_or_default(),
            );
            last_flush = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_accumulates_counts_per_mac_domain() {
        let mut s = CaptureState::default();
        s.record("aa".to_string(), "example.com".to_string());
        s.record("aa".to_string(), "example.com".to_string());
        s.record("aa".to_string(), "other.com".to_string());
        s.record("bb".to_string(), "example.com".to_string());
        let mut out = s.drain();
        out.sort();
        assert_eq!(out, vec![
            ("aa".to_string(), "example.com".to_string(), 2),
            ("aa".to_string(), "other.com".to_string(), 1),
            ("bb".to_string(), "example.com".to_string(), 1),
        ]);
    }

    #[test]
    fn drain_clears_state_for_next_window() {
        let mut s = CaptureState::default();
        s.record("aa".to_string(), "example.com".to_string());
        assert_eq!(s.drain().len(), 1);
        assert_eq!(s.drain().len(), 0); // second window starts empty
    }

    #[test]
    fn cardinality_cap_folds_overflow_domains_into_other_bucket() {
        let mut s = CaptureState::default();
        for i in 0..CARDINALITY_CAP {
            s.record("aa".to_string(), format!("host{i}.example"));
        }
        // the (CARDINALITY_CAP+1)-th *new* distinct domain overflows the cap
        s.record("aa".to_string(), "overflow.example".to_string());
        let out = s.drain();
        assert_eq!(out.len(), CARDINALITY_CAP + 1); // CAP distinct + 1 "__other__" bucket
        let other = out.iter().find(|(mac, d, _)| mac == "aa" && d == "__other__");
        assert_eq!(other.map(|(_, _, c)| *c), Some(1));
    }

    #[test]
    fn already_seen_domain_stays_itself_even_after_cap_reached() {
        let mut s = CaptureState::default();
        s.record("aa".to_string(), "first.example".to_string());
        for i in 0..CARDINALITY_CAP {
            s.record("bb".to_string(), format!("filler{i}.example"));
        }
        // "first.example" was already tracked before the cap filled up -> stays itself
        s.record("aa".to_string(), "first.example".to_string());
        let out = s.drain();
        let first = out.iter().find(|(mac, d, _)| mac == "aa" && d == "first.example");
        assert_eq!(first.map(|(_, _, c)| *c), Some(2));
    }
}
