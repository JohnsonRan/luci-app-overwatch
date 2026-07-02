#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProbeStatus {
    pub conntrack_acct: bool,
    pub btf: bool,
    pub clsact: bool,
    pub hw_stats_advancing: Option<bool>,
}

pub fn parse_acct(sysctl_value: &str) -> bool {
    sysctl_value.trim() == "1"
}

pub fn modules_has(proc_modules: &str, name: &str) -> bool {
    proc_modules
        .lines()
        .any(|l| l.split_whitespace().next() == Some(name))
}

pub fn hw_stats_advancing(prev_total: u64, cur_total: u64) -> Option<bool> {
    if cur_total < prev_total {
        None
    } else {
        Some(cur_total > prev_total)
    }
}

pub fn gather(acct: &str, btf_present: bool, proc_modules: &str) -> ProbeStatus {
    ProbeStatus {
        conntrack_acct: parse_acct(acct),
        btf: btf_present,
        clsact: modules_has(proc_modules, "sch_ingress") || modules_has(proc_modules, "ingress"),
        hw_stats_advancing: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acct_parsing() {
        assert!(parse_acct("1\n"));
        assert!(!parse_acct("0"));
        assert!(!parse_acct(""));
    }

    #[test]
    fn module_presence() {
        let mods = "sch_ingress 16384 0 - Live 0x0\nnf_conntrack 180224 4 - Live 0x0\n";
        assert!(modules_has(mods, "sch_ingress"));
        assert!(modules_has(mods, "nf_conntrack"));
        assert!(!modules_has(mods, "cls_bpf"));
    }

    #[test]
    fn hw_stats_delta() {
        assert_eq!(hw_stats_advancing(100, 200), Some(true));
        assert_eq!(hw_stats_advancing(200, 200), Some(false));
        assert_eq!(hw_stats_advancing(200, 100), None); // reset
        assert_eq!(hw_stats_advancing(0, 0), Some(false)); // boundary: no data, no reset
    }

    #[test]
    fn gather_assembles_status() {
        let mods = "sch_ingress 1 0 - Live 0x0\n";
        let st = gather("1", true, mods);
        assert!(st.conntrack_acct);
        assert!(st.btf);
        assert!(st.clsact);
        assert_eq!(st.hw_stats_advancing, None);
    }

    #[test]
    fn gather_clsact_via_ingress_alias() {
        // The alternate module name "ingress" alone must also set clsact.
        let st = gather("0", false, "ingress 1 0 - Live 0x0\n");
        assert!(st.clsact);
        assert!(!st.conntrack_acct);
        assert!(!st.btf);
    }
}
