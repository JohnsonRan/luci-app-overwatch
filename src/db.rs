use rusqlite::Connection;

use crate::sample::Sample;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Min,
    Hour,
    Day,
}

impl Tier {
    pub fn table(&self) -> &'static str {
        match self {
            Tier::Min => "samples_min",
            Tier::Hour => "samples_hour",
            Tier::Day => "samples_day",
        }
    }
}

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "store error: {}", self.0)
    }
}
impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError(e.to_string())
    }
}

pub trait Store {
    fn upsert_client(&self, mac: &str, host: &str, ip: &str, now: i64) -> Result<(), StoreError>;
    fn insert_samples(&self, tier: Tier, mac: &str, samples: &[Sample]) -> Result<(), StoreError>;
    fn query(&self, tier: Tier, mac: Option<&str>, from: i64, to: i64) -> Result<Vec<Sample>, StoreError>;
    fn add_dns_counts(&self, ts_day: i64, counts: &[(String, String, u64)]) -> Result<(), StoreError>;
    fn dns_top(&self, client: Option<&str>, from: i64, to: i64, limit: u64) -> Result<Vec<(String, u64)>, StoreError>;
}

pub struct SqliteStore {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS clients (
  mac TEXT PRIMARY KEY,
  host TEXT, ip TEXT, first_seen INTEGER, last_seen INTEGER
);
CREATE TABLE IF NOT EXISTS samples_min (
  ts INTEGER NOT NULL, client TEXT NOT NULL,
  rx_bytes INTEGER, tx_bytes INTEGER, rx_peak INTEGER, tx_peak INTEGER, conns INTEGER,
  PRIMARY KEY (ts, client)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS samples_hour (
  ts INTEGER NOT NULL, client TEXT NOT NULL,
  rx_bytes INTEGER, tx_bytes INTEGER, rx_peak INTEGER, tx_peak INTEGER, conns INTEGER,
  PRIMARY KEY (ts, client)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS samples_day (
  ts INTEGER NOT NULL, client TEXT NOT NULL,
  rx_bytes INTEGER, tx_bytes INTEGER, rx_peak INTEGER, tx_peak INTEGER, conns INTEGER,
  PRIMARY KEY (ts, client)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS dns_counts_day (
  ts INTEGER NOT NULL, client TEXT NOT NULL, domain TEXT NOT NULL, count INTEGER NOT NULL,
  PRIMARY KEY (ts, client, domain)
) WITHOUT ROWID;
";

impl SqliteStore {
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(SqliteStore { conn })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(SqliteStore { conn })
    }

    pub fn client_count(&self) -> Result<i64, StoreError> {
        let n = self.conn.query_row("SELECT COUNT(*) FROM clients", [], |r| r.get(0))?;
        Ok(n)
    }
}

impl Store for SqliteStore {
    fn upsert_client(&self, mac: &str, host: &str, ip: &str, now: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO clients (mac, host, ip, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(mac) DO UPDATE SET host=?2, ip=?3, last_seen=?4",
            rusqlite::params![mac, host, ip, now],
        )?;
        Ok(())
    }

    fn insert_samples(&self, tier: Tier, mac: &str, samples: &[Sample]) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT OR REPLACE INTO {} (ts, client, rx_bytes, tx_bytes, rx_peak, tx_peak, conns)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            tier.table()
        );
        // One transaction for the whole batch: per-row autocommit would fsync per sample (flash-costly).
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(&sql)?;
            for s in samples {
                stmt.execute(rusqlite::params![
                    s.ts, mac, s.rx_bytes as i64, s.tx_bytes as i64,
                    s.rx_peak as i64, s.tx_peak as i64, s.conns as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn query(&self, tier: Tier, mac: Option<&str>, from: i64, to: i64) -> Result<Vec<Sample>, StoreError> {
        let row_to_sample = |r: &rusqlite::Row| -> rusqlite::Result<Sample> {
            Ok(Sample {
                ts: r.get(0)?,
                rx_bytes: r.get::<_, i64>(1)? as u64,
                tx_bytes: r.get::<_, i64>(2)? as u64,
                rx_peak: r.get::<_, i64>(3)? as u64,
                tx_peak: r.get::<_, i64>(4)? as u64,
                conns: r.get::<_, i64>(5)? as u32,
            })
        };

        let mut out = Vec::new();
        match mac {
            Some(m) => {
                let sql = format!(
                    "SELECT ts, rx_bytes, tx_bytes, rx_peak, tx_peak, conns FROM {}
                     WHERE client=?1 AND ts>=?2 AND ts<=?3 ORDER BY ts",
                    tier.table()
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params![m, from, to], row_to_sample)?;
                for r in rows {
                    out.push(r?);
                }
            }
            None => {
                // Total series: bytes SUM across clients; peaks/conns use MAX (mirrors
                // sample::aggregate). conns = busiest single client, not sum of concurrent conns.
                let sql = format!(
                    "SELECT ts, SUM(rx_bytes), SUM(tx_bytes), MAX(rx_peak), MAX(tx_peak), MAX(conns)
                     FROM {} WHERE ts>=?1 AND ts<=?2 GROUP BY ts ORDER BY ts",
                    tier.table()
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params![from, to], row_to_sample)?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    fn add_dns_counts(&self, ts_day: i64, counts: &[(String, String, u64)]) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO dns_counts_day (ts, client, domain, count)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(ts, client, domain) DO UPDATE SET count = count + excluded.count",
            )?;
            for (mac, domain, count) in counts {
                stmt.execute(rusqlite::params![ts_day, mac, domain, *count as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn dns_top(&self, client: Option<&str>, from: i64, to: i64, limit: u64) -> Result<Vec<(String, u64)>, StoreError> {
        let row = |r: &rusqlite::Row| -> rusqlite::Result<(String, u64)> {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))
        };
        let mut out = Vec::new();
        match client {
            Some(c) => {
                let mut stmt = self.conn.prepare(
                    "SELECT domain, SUM(count) AS c FROM dns_counts_day
                     WHERE client=?1 AND ts>=?2 AND ts<=?3
                     GROUP BY domain ORDER BY c DESC LIMIT ?4",
                )?;
                let rows = stmt.query_map(rusqlite::params![c, from, to, limit as i64], row)?;
                for r in rows { out.push(r?); }
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT domain, SUM(count) AS c FROM dns_counts_day
                     WHERE ts>=?1 AND ts<=?2
                     GROUP BY domain ORDER BY c DESC LIMIT ?3",
                )?;
                let rows = stmt.query_map(rusqlite::params![from, to, limit as i64], row)?;
                for r in rows { out.push(r?); }
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Retention {
    pub min_secs: i64,
    pub hour_secs: i64,
    pub day_secs: i64,
    pub dns_days: i64,
}

pub fn default_retention() -> Retention {
    Retention {
        min_secs: 48 * 3600,
        hour_secs: 35 * 86400,
        day_secs: 395 * 86400,
        dns_days: 395 * 86400,
    }
}

impl SqliteStore {
    fn rollup_tier(&self, src: Tier, dst: Tier, bucket: i64) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT OR REPLACE INTO {dst} (ts, client, rx_bytes, tx_bytes, rx_peak, tx_peak, conns)
             SELECT (ts - (ts % {bucket})) AS b, client,
                    SUM(rx_bytes), SUM(tx_bytes), MAX(rx_peak), MAX(tx_peak), MAX(conns)
             FROM {src}
             GROUP BY b, client",
            dst = dst.table(),
            src = src.table(),
            bucket = bucket
        );
        self.conn.execute_batch(&sql)?;
        Ok(())
    }

    // Re-aggregates every source bucket; idempotent via INSERT OR REPLACE.
    pub fn rollup(&self) -> Result<(), StoreError> {
        self.rollup_tier(Tier::Min, Tier::Hour, 3600)?;
        self.rollup_tier(Tier::Hour, Tier::Day, 86400)?;
        Ok(())
    }

    pub fn prune(&self, now: i64, r: &Retention) -> Result<(), StoreError> {
        self.conn.execute(
            &format!("DELETE FROM {} WHERE ts < ?1", Tier::Min.table()),
            rusqlite::params![now - r.min_secs],
        )?;
        self.conn.execute(
            &format!("DELETE FROM {} WHERE ts < ?1", Tier::Hour.table()),
            rusqlite::params![now - r.hour_secs],
        )?;
        self.conn.execute(
            &format!("DELETE FROM {} WHERE ts < ?1", Tier::Day.table()),
            rusqlite::params![now - r.day_secs],
        )?;
        self.conn.execute(
            "DELETE FROM dns_counts_day WHERE ts < ?1",
            rusqlite::params![now - r.dns_days],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ts: i64, rx: u64, tx: u64) -> Sample {
        Sample { ts, rx_bytes: rx, tx_bytes: tx, rx_peak: rx, tx_peak: tx, conns: 1 }
    }

    #[test]
    fn insert_and_query_one_client() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.insert_samples(Tier::Min, "aa", &[s(60, 100, 10), s(120, 200, 20)]).unwrap();
        let got = store.query(Tier::Min, Some("aa"), 0, 1000).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].ts, 60);
        assert_eq!(got[1].rx_bytes, 200);
    }

    #[test]
    fn query_range_is_inclusive_and_ordered() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.insert_samples(Tier::Min, "aa", &[s(60, 1, 0), s(120, 2, 0), s(180, 3, 0)]).unwrap();
        let got = store.query(Tier::Min, Some("aa"), 60, 120).unwrap();
        assert_eq!(got.iter().map(|x| x.ts).collect::<Vec<_>>(), vec![60, 120]);
    }

    #[test]
    fn query_total_sums_across_clients_per_bucket() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.insert_samples(Tier::Min, "aa", &[s(60, 100, 10)]).unwrap();
        store.insert_samples(Tier::Min, "bb", &[s(60, 300, 30)]).unwrap();
        let total = store.query(Tier::Min, None, 0, 1000).unwrap();
        assert_eq!(total.len(), 1);
        assert_eq!(total[0].ts, 60);
        assert_eq!(total[0].rx_bytes, 400);
        assert_eq!(total[0].tx_bytes, 40);
    }

    #[test]
    fn upsert_client_is_idempotent_on_mac() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.upsert_client("aa", "laptop", "192.168.1.5", 100).unwrap();
        store.upsert_client("aa", "laptop-renamed", "192.168.1.9", 200).unwrap();
        // second upsert updates, does not duplicate; verified via a count query helper
        assert_eq!(store.client_count().unwrap(), 1);
        // host/ip/last_seen updated, but first_seen preserved from the initial insert
        let (host, ip, first, last): (String, String, i64, i64) = store
            .conn
            .query_row(
                "SELECT host, ip, first_seen, last_seen FROM clients WHERE mac='aa'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(host, "laptop-renamed");
        assert_eq!(ip, "192.168.1.9");
        assert_eq!(first, 100); // preserved
        assert_eq!(last, 200); // updated
    }

    #[test]
    fn rollup_aggregates_minutes_into_hours() {
        let store = SqliteStore::open_in_memory().unwrap();
        // three minute samples in the same hour bucket (3600..7200 -> bucket 3600).
        // The third carries low bytes but a high conns peak to exercise MAX(conns).
        store
            .insert_samples(
                Tier::Min,
                "aa",
                &[
                    s(3600, 100, 10),
                    s(3660, 200, 20),
                    Sample { ts: 3720, rx_bytes: 5, tx_bytes: 5, rx_peak: 5, tx_peak: 5, conns: 9 },
                ],
            )
            .unwrap();
        store.rollup().unwrap();
        let hours = store.query(Tier::Hour, Some("aa"), 0, 100_000).unwrap();
        assert_eq!(hours.len(), 1);
        assert_eq!(hours[0].ts, 3600);      // hour bucket start
        assert_eq!(hours[0].rx_bytes, 305); // SUM rx
        assert_eq!(hours[0].tx_bytes, 35);  // SUM tx
        assert_eq!(hours[0].rx_peak, 200);  // MAX rx_peak
        assert_eq!(hours[0].tx_peak, 20);   // MAX tx_peak
        assert_eq!(hours[0].conns, 9);      // MAX conns
    }

    #[test]
    fn prune_removes_rows_older_than_retention() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.insert_samples(Tier::Min, "aa", &[s(100, 1, 0), s(1_000_000, 2, 0)]).unwrap();
        let r = Retention { min_secs: 48 * 3600, hour_secs: 35 * 86400, day_secs: 395 * 86400, dns_days: 395 * 86400 };
        store.prune(1_000_000, &r).unwrap();
        let left = store.query(Tier::Min, Some("aa"), 0, 2_000_000).unwrap();
        assert_eq!(left.len(), 1);          // ts=100 pruned (older than 48h before now)
        assert_eq!(left[0].ts, 1_000_000);
    }

    #[test]
    fn add_dns_counts_accumulates_on_conflict() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.add_dns_counts(86400, &[("aa".to_string(), "example.com".to_string(), 3)]).unwrap();
        store.add_dns_counts(86400, &[("aa".to_string(), "example.com".to_string(), 2)]).unwrap();
        let top = store.dns_top(None, 0, 200_000, 10).unwrap();
        assert_eq!(top, vec![("example.com".to_string(), 5)]);
    }

    #[test]
    fn dns_top_filters_by_client_and_sorts_descending() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.add_dns_counts(86400, &[
            ("aa".to_string(), "a.com".to_string(), 1),
            ("aa".to_string(), "b.com".to_string(), 5),
            ("bb".to_string(), "c.com".to_string(), 9),
        ]).unwrap();
        let all = store.dns_top(None, 0, 200_000, 10).unwrap();
        assert_eq!(all, vec![
            ("c.com".to_string(), 9),
            ("b.com".to_string(), 5),
            ("a.com".to_string(), 1),
        ]);
        let aa_only = store.dns_top(Some("aa"), 0, 200_000, 10).unwrap();
        assert_eq!(aa_only, vec![("b.com".to_string(), 5), ("a.com".to_string(), 1)]);
    }

    #[test]
    fn dns_top_respects_limit() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.add_dns_counts(86400, &[
            ("aa".to_string(), "a.com".to_string(), 1),
            ("aa".to_string(), "b.com".to_string(), 2),
            ("aa".to_string(), "c.com".to_string(), 3),
        ]).unwrap();
        let top = store.dns_top(None, 0, 200_000, 2).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "c.com");
    }

    #[test]
    fn prune_removes_dns_counts_older_than_dns_days_retention() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.add_dns_counts(100, &[("aa".to_string(), "old.com".to_string(), 1)]).unwrap();
        store.add_dns_counts(1_000_000, &[("aa".to_string(), "new.com".to_string(), 1)]).unwrap();
        let r = Retention { min_secs: 48 * 3600, hour_secs: 35 * 86400, day_secs: 395 * 86400, dns_days: 48 * 3600 };
        store.prune(1_000_000, &r).unwrap();
        let left = store.dns_top(None, 0, 2_000_000, 10).unwrap();
        assert_eq!(left, vec![("new.com".to_string(), 1)]);
    }
}
