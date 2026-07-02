use crate::flow::Flow;
use std::io;
use std::path::PathBuf;

pub fn parse_all(text: &str) -> Vec<Flow> {
    text.lines().filter_map(Flow::parse_line).collect()
}

pub trait ConntrackSource {
    fn read(&self) -> io::Result<String>;
}

pub struct ProcConntrack {
    pub path: PathBuf,
}

impl ProcConntrack {
    pub fn new() -> Self {
        ProcConntrack { path: PathBuf::from("/proc/net/nf_conntrack") }
    }
}

impl ConntrackSource for ProcConntrack {
    fn read(&self) -> io::Result<String> {
        std::fs::read_to_string(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/nf_conntrack.txt");

    #[test]
    fn parse_all_keeps_only_accounted_flows() {
        let flows = parse_all(SAMPLE);
        // fixture has 3 lines; the SYN_SENT/UNREPLIED line lacks byte counters.
        assert_eq!(flows.len(), 2);
    }
}
