use crate::sample::Sample;

pub fn lttb(samples: &[Sample], threshold: usize) -> Vec<Sample> {
    let n = samples.len();
    if threshold < 3 || threshold >= n {
        return samples.to_vec();
    }

    let value = |s: &Sample| s.rx_bytes.saturating_add(s.tx_bytes) as f64;
    let x = |s: &Sample| s.ts as f64;

    let mut sampled: Vec<Sample> = Vec::with_capacity(threshold);
    sampled.push(samples[0]);

    // bucket size for the threshold-2 middle points
    let every = (n - 2) as f64 / (threshold - 2) as f64;
    let mut a = 0usize; // index of the last selected point

    for i in 0..(threshold - 2) {
        // average point of the NEXT bucket
        let mut avg_start = ((i + 1) as f64 * every).floor() as usize + 1;
        let mut avg_end = ((i + 2) as f64 * every).floor() as usize + 1;
        avg_end = avg_end.min(n);
        if avg_start >= n { avg_start = n - 1; }
        let mut avg_x = 0.0;
        let mut avg_y = 0.0;
        let avg_count = (avg_end - avg_start).max(1);
        for s in &samples[avg_start..avg_end.max(avg_start + 1).min(n)] {
            avg_x += x(s);
            avg_y += value(s);
        }
        avg_x /= avg_count as f64;
        avg_y /= avg_count as f64;

        // range of the CURRENT bucket to pick from
        let range_start = (i as f64 * every).floor() as usize + 1;
        let range_end = (((i + 1) as f64 * every).floor() as usize + 1).min(n);

        let point_a_x = x(&samples[a]);
        let point_a_y = value(&samples[a]);

        let mut max_area = -1.0;
        let mut next_a = range_start;
        for j in range_start..range_end {
            let area = ((point_a_x - avg_x) * (value(&samples[j]) - point_a_y)
                - (point_a_x - x(&samples[j])) * (avg_y - point_a_y))
                .abs()
                * 0.5;
            if area > max_area {
                max_area = area;
                next_a = j;
            }
        }
        sampled.push(samples[next_a]);
        a = next_a;
    }

    sampled.push(samples[n - 1]);
    sampled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ts: i64, v: u64) -> Sample {
        Sample { ts, rx_bytes: v, tx_bytes: 0, rx_peak: v, tx_peak: 0, conns: 0 }
    }

    #[test]
    fn returns_input_when_threshold_not_smaller() {
        let data: Vec<Sample> = (0..5).map(|i| s(i, i as u64)).collect();
        assert_eq!(lttb(&data, 10).len(), 5);
        assert_eq!(lttb(&data, 5).len(), 5);
        assert_eq!(lttb(&data, 2).len(), 5); // threshold < 3 -> unchanged
    }

    #[test]
    fn downsamples_to_threshold_keeping_endpoints() {
        let data: Vec<Sample> = (0..100).map(|i| s(i, (i % 7) as u64)).collect();
        let out = lttb(&data, 10);
        assert_eq!(out.len(), 10);
        assert_eq!(out[0], data[0]);            // first kept
        assert_eq!(out[9], data[99]);           // last kept
        // output timestamps are strictly increasing (monotonic selection)
        for w in out.windows(2) {
            assert!(w[0].ts < w[1].ts);
        }
    }

    #[test]
    fn preserves_a_spike() {
        let mut data: Vec<Sample> = (0..50).map(|i| s(i, 1)).collect();
        data[25] = s(25, 1000); // sharp spike
        let out = lttb(&data, 6);
        assert!(out.iter().any(|p| p.rx_bytes == 1000), "spike should survive downsampling");
    }
}
