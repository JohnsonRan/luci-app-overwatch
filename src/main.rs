use overwatchd::account::Accountant;
use overwatchd::classify::LanClassifier;
use overwatchd::conntrack::{parse_all, ConntrackSource, ProcConntrack};
use overwatchd::names::NameTable;
use overwatchd::snapshot::build_snapshot;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

fn read_to_string_or_empty(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn main() {
    use overwatchd::config::{default_config, parse_uci};
    use overwatchd::control::Shared;
    use overwatchd::db::{SqliteStore, Store, Tier};
    use overwatchd::recorder::Recorder;

    let cfg = match std::fs::read_to_string("/etc/config/overwatch") {
        Ok(t) => parse_uci(&t),
        Err(_) => default_config(),
    };
    #[allow(unused_variables)]
    let dns_enabled = cfg.dns_stats_enabled;
    #[allow(unused_variables)]
    let dns_iface = cfg.dns_iface.clone();
    #[allow(unused_variables)]
    let dns_flush_secs = cfg.dns_flush_secs;
    let classifier = LanClassifier::new(cfg.lan_prefixes.clone());
    let interval = Duration::from_secs(cfg.sample_interval.max(1));

    let mut acct = Accountant::new(classifier);
    let source = ProcConntrack::new();

    // Ensure the db parent dir exists; a dev run or custom db_path may not have it.
    if let Some(parent) = std::path::Path::new(&cfg.db_path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("create_dir_all({}) failed: {e}", parent.display());
        }
    }
    let store = SqliteStore::open(&cfg.db_path).unwrap_or_else(|e| {
        eprintln!("db open failed ({e}); using in-memory store");
        SqliteStore::open_in_memory().expect("in-memory store")
    });
    let mut last_maint = 0i64;

    // Read once at startup: these are static facts about the running kernel.
    // hw_stats_advancing stays None; no verified sysfs source to read it from.
    let probes = overwatchd::probe::gather(
        &read_to_string_or_empty("/proc/sys/net/netfilter/nf_conntrack_acct"),
        std::path::Path::new("/sys/kernel/btf/vmlinux").exists(),
        &read_to_string_or_empty("/proc/modules"),
    );

    let shared = Arc::new(Mutex::new(Shared {
        clients: Vec::new(),
        recorder: Recorder::new(300, 1440),
        store,
        config: cfg,
        probes,
    }));

    #[cfg(unix)]
    {
        use overwatchd::server::serve;
        let sh = shared.clone();
        std::thread::spawn(move || serve("/var/run/overwatch.sock", sh));
    }

    #[cfg(target_os = "linux")]
    if dns_enabled {
        let sh = shared.clone();
        std::thread::spawn(move || overwatchd::dns_capture::run(&dns_iface, dns_flush_secs, sh));
    }

    loop {
        match source.read() {
            Ok(text) => {
                let flows = parse_all(&text);
                acct.update(&flows, now_secs());
                let names = NameTable::from_text(
                    &read_to_string_or_empty("/tmp/dhcp.leases"),
                    &read_to_string_or_empty("/proc/net/arp"),
                );
                let snap = build_snapshot(acct.stats(), &names);
                let now_i = now_secs() as i64;
                // Store errors are logged but never fatal: keep monitoring even if persistence is wedged.
                {
                    let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
                    let finalized = g.recorder.record(&snap, now_i);
                    for (mac, sample) in &finalized {
                        if let Err(e) = g.store.insert_samples(Tier::Min, mac, std::slice::from_ref(sample)) {
                            eprintln!("insert_samples({mac}) failed: {e}");
                        }
                    }
                    for v in &snap {
                        if let Err(e) = g.store.upsert_client(&v.mac, &v.host, &v.ip, now_i) {
                            eprintln!("upsert_client({}) failed: {e}", v.mac);
                        }
                    }
                    if now_i - last_maint >= 300 {
                        let ret = g.config.retention;
                        if let Err(e) = g.store.rollup() { eprintln!("rollup failed: {e}"); }
                        if let Err(e) = g.store.prune(now_i, &ret) { eprintln!("prune failed: {e}"); }
                        last_maint = now_i;
                    }
                    g.clients = snap.clone();
                }
                match serde_json::to_string(&snap) {
                    Ok(j) => println!("{j}"),
                    Err(e) => eprintln!("serialize error: {e}"),
                }
            }
            Err(e) => eprintln!("conntrack read error: {e}"),
        }
        sleep(interval);
    }
}
