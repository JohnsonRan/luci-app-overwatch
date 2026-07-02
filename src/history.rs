use std::collections::BTreeMap;

use crate::db::{Store, Tier};
use crate::downsample::lttb;
use crate::recorder::Recorder;
use crate::sample::Sample;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistTier {
    Realtime,
    Day,
    Week,
    Month,
}

pub struct HistoryService<'a, S: Store> {
    recorder: &'a Recorder,
    store: &'a S,
}

impl<'a, S: Store> HistoryService<'a, S> {
    pub fn new(recorder: &'a Recorder, store: &'a S) -> Self {
        HistoryService { recorder, store }
    }

    pub fn series(
        &self,
        tier: HistTier,
        mac: Option<&str>,
        from: i64,
        to: i64,
        max_points: usize,
    ) -> Vec<Sample> {
        let raw: Vec<Sample> = match tier {
            HistTier::Realtime => self.ram_series(mac, from, to, true),
            HistTier::Day => self.ram_series(mac, from, to, false),
            HistTier::Week => self.store.query(Tier::Hour, mac, from, to).unwrap_or_default(),
            HistTier::Month => self.store.query(Tier::Day, mac, from, to).unwrap_or_default(),
        };
        lttb(&raw, max_points)
    }

    fn ram_series(&self, mac: Option<&str>, from: i64, to: i64, realtime: bool) -> Vec<Sample> {
        let in_range = |s: &Sample| s.ts >= from && s.ts <= to;
        match mac {
            Some(m) => {
                let v = if realtime { self.recorder.realtime(m) } else { self.recorder.minutes(m) };
                v.into_iter().filter(in_range).collect()
            }
            None => {
                // sum across all known clients per timestamp
                let macs = self.recorder.macs();
                let mut by_ts: BTreeMap<i64, Sample> = BTreeMap::new();
                for m in macs {
                    let v = if realtime { self.recorder.realtime(&m) } else { self.recorder.minutes(&m) };
                    for s in v.into_iter().filter(in_range) {
                        let e = by_ts.entry(s.ts).or_insert(Sample {
                            ts: s.ts, rx_bytes: 0, tx_bytes: 0, rx_peak: 0, tx_peak: 0, conns: 0,
                        });
                        e.rx_bytes = e.rx_bytes.saturating_add(s.rx_bytes);
                        e.tx_bytes = e.tx_bytes.saturating_add(s.tx_bytes);
                        e.rx_peak = e.rx_peak.max(s.rx_peak);
                        e.tx_peak = e.tx_peak.max(s.tx_peak);
                        e.conns = e.conns.max(s.conns);
                    }
                }
                by_ts.into_values().collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{SqliteStore, Store, Tier};
    use crate::recorder::Recorder;
    use crate::sample::Sample;

    #[test]
    fn week_tier_reads_hour_table_downsampled() {
        let store = SqliteStore::open_in_memory().unwrap();
        let samples: Vec<Sample> = (0..200)
            .map(|i| Sample { ts: i * 3600, rx_bytes: i as u64, tx_bytes: 0, rx_peak: 0, tx_peak: 0, conns: 0 })
            .collect();
        store.insert_samples(Tier::Hour, "aa", &samples).unwrap();
        let recorder = Recorder::new(300, 1440);
        let svc = HistoryService::new(&recorder, &store);
        let out = svc.series(HistTier::Week, Some("aa"), 0, 1_000_000, 50);
        assert_eq!(out.len(), 50);             // downsampled
        assert_eq!(out[0].ts, 0);              // endpoints preserved
        assert_eq!(out[49].ts, 199 * 3600);
    }

    #[test]
    fn realtime_tier_reads_ram_ring() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut recorder = Recorder::new(300, 1440);
        // need ClientView to feed recorder
        use crate::snapshot::ClientView;
        let v = |mac: &str, rx: u64| ClientView {
            mac: mac.to_string(), ip: String::new(), host: String::new(),
            rx_bps: rx, tx_bps: 0, rx_total: 0, tx_total: 0,
            conns_tcp: 0, conns_udp: 0,
        };
        recorder.record(&[v("aa", 1000)], 100);
        recorder.record(&[v("aa", 1000)], 102);
        let svc = HistoryService::new(&recorder, &store);
        let out = svc.series(HistTier::Realtime, Some("aa"), 0, 1000, 500);
        assert_eq!(out.len(), 2);              // both realtime ticks, no downsample needed
    }
}
