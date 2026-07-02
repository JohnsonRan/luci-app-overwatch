#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub ts: i64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_peak: u64,
    pub tx_peak: u64,
    pub conns: u32,
}

pub fn aggregate(samples: &[Sample], bucket_ts: i64) -> Sample {
    let mut out = Sample { ts: bucket_ts, rx_bytes: 0, tx_bytes: 0, rx_peak: 0, tx_peak: 0, conns: 0 };
    for s in samples {
        out.rx_bytes = out.rx_bytes.saturating_add(s.rx_bytes);
        out.tx_bytes = out.tx_bytes.saturating_add(s.tx_bytes);
        out.rx_peak = out.rx_peak.max(s.rx_peak);
        out.tx_peak = out.tx_peak.max(s.tx_peak);
        out.conns = out.conns.max(s.conns);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ts: i64, rx: u64, tx: u64, rp: u64, tp: u64, c: u32) -> Sample {
        Sample { ts, rx_bytes: rx, tx_bytes: tx, rx_peak: rp, tx_peak: tp, conns: c }
    }

    #[test]
    fn aggregate_sums_bytes_and_maxes_peaks() {
        let input = [
            s(10, 100, 10, 500, 50, 3),
            s(20, 200, 20, 400, 80, 5),
            s(30, 300, 30, 900, 40, 2),
        ];
        let out = aggregate(&input, 0);
        assert_eq!(out.ts, 0);
        assert_eq!(out.rx_bytes, 600);
        assert_eq!(out.tx_bytes, 60);
        assert_eq!(out.rx_peak, 900);
        assert_eq!(out.tx_peak, 80);
        assert_eq!(out.conns, 5);
    }

    #[test]
    fn aggregate_empty_is_zero_at_bucket_ts() {
        let out = aggregate(&[], 1234);
        assert_eq!(out, s(1234, 0, 0, 0, 0, 0));
    }
}
