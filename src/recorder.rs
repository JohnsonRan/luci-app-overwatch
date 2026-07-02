use crate::ring::RingBuffer;
use crate::sample::{aggregate, Sample};
use crate::snapshot::ClientView;
use std::collections::HashMap;

struct ClientSeries {
    realtime: RingBuffer<Sample>,
    minutes: RingBuffer<Sample>,
    cur_minute: i64,         // current minute being accumulated (now / 60); reset on each rollover
    cur_samples: Vec<Sample>,
}

pub struct Recorder {
    realtime_cap: usize,
    minute_cap: usize,
    series: HashMap<String, ClientSeries>,
    last_ts: Option<i64>,
}

impl Recorder {
    pub fn new(realtime_cap: usize, minute_cap: usize) -> Self {
        Recorder {
            realtime_cap,
            minute_cap,
            series: HashMap::new(),
            last_ts: None,
        }
    }

    pub fn record(&mut self, views: &[ClientView], now: i64) -> Vec<(String, Sample)> {
        let interval = self.last_ts.map(|t| (now - t).max(0) as u64).unwrap_or(0);
        let minute = now / 60;
        let mut finalized: Vec<(String, Sample)> = Vec::new();

        for v in views {
            let rx_bps = v.rx_bps;
            let tx_bps = v.tx_bps;
            let conns = v.conns_tcp + v.conns_udp;
            let sample = Sample {
                ts: now,
                rx_bytes: rx_bps.saturating_mul(interval),
                tx_bytes: tx_bps.saturating_mul(interval),
                rx_peak: rx_bps,
                tx_peak: tx_bps,
                conns,
            };

            let cap_rt = self.realtime_cap;
            let cap_min = self.minute_cap;
            let series = self.series.entry(v.mac.clone()).or_insert_with(|| ClientSeries {
                realtime: RingBuffer::new(cap_rt),
                minutes: RingBuffer::new(cap_min),
                cur_minute: minute,
                cur_samples: Vec::new(),
            });

            series.realtime.push(sample);

            if series.cur_minute != minute && !series.cur_samples.is_empty() {
                let bucket_ts = series.cur_minute * 60;
                let agg = aggregate(&series.cur_samples, bucket_ts);
                series.minutes.push(agg);
                finalized.push((v.mac.clone(), agg));
                series.cur_samples.clear();
            }
            series.cur_minute = minute;
            series.cur_samples.push(sample);
        }

        self.last_ts = Some(now);
        finalized
    }

    pub fn realtime(&self, mac: &str) -> Vec<Sample> {
        self.series.get(mac).map(|s| s.realtime.to_vec()).unwrap_or_default()
    }

    pub fn minutes(&self, mac: &str) -> Vec<Sample> {
        self.series.get(mac).map(|s| s.minutes.to_vec()).unwrap_or_default()
    }

    pub fn macs(&self) -> Vec<String> {
        self.series.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(mac: &str, rx_bps: u64, tx_bps: u64, conns_tcp: u32, conns_udp: u32) -> ClientView {
        ClientView {
            mac: mac.to_string(),
            ip: "192.168.1.10".to_string(),
            host: String::new(),
            rx_bps,
            tx_bps,
            rx_total: 0,
            tx_total: 0,
            conns_tcp,
            conns_udp,
        }
    }

    #[test]
    fn first_tick_contributes_no_bytes_but_records_speed() {
        let mut r = Recorder::new(300, 1440);
        let finalized = r.record(&[view("aa", 1000, 100, 1, 0)], 100);
        assert!(finalized.is_empty()); // no minute boundary crossed yet
        let rt = r.realtime("aa");
        assert_eq!(rt.len(), 1);
        assert_eq!(rt[0].rx_bytes, 0);   // first tick: unknown interval -> 0 bytes
        assert_eq!(rt[0].rx_peak, 1000); // but speed/peak recorded
        assert_eq!(rt[0].conns, 1);
    }

    #[test]
    fn second_tick_uses_elapsed_interval_for_bytes() {
        let mut r = Recorder::new(300, 1440);
        r.record(&[view("aa", 1000, 100, 0, 0)], 100);
        r.record(&[view("aa", 1000, 100, 0, 0)], 102); // 2s later
        let rt = r.realtime("aa");
        assert_eq!(rt.len(), 2);
        assert_eq!(rt[1].rx_bytes, 2000); // 1000 Bps * 2s
        assert_eq!(rt[1].tx_bytes, 200);
    }

    #[test]
    fn minute_rollover_emits_aggregated_sample() {
        let mut r = Recorder::new(300, 1440);
        // two ticks inside minute 1 (ts 60..120)
        r.record(&[view("aa", 1000, 0, 2, 0)], 60);
        r.record(&[view("aa", 1000, 0, 5, 0)], 62);
        // tick in minute 2 (ts >= 120) triggers finalize of minute 1
        let finalized = r.record(&[view("aa", 0, 0, 0, 0)], 120);
        assert_eq!(finalized.len(), 1);
        let (mac, sample) = &finalized[0];
        assert_eq!(mac, "aa");
        assert_eq!(sample.ts, 60);          // bucket start = minute 1
        assert_eq!(sample.rx_bytes, 2000);  // 0 (first ever) + 2000 (2s*1000)
        assert_eq!(sample.conns, 5);        // max conns in the minute
        assert_eq!(r.minutes("aa").len(), 1);
    }
}
