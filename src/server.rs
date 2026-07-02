use crate::control::{dispatch, Shared};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cap on one request line, bounding read-buffer memory if a client sends no newline.
const MAX_REQ_BYTES: u64 = 64 * 1024;
/// Per-connection read timeout so a stalled client only wedges its own worker.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub fn serve(path: &str, shared: Arc<Mutex<Shared>>) {
    let _ = std::fs::remove_file(path); // clear stale socket
    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("control socket bind {path} failed: {e}");
            return;
        }
    };
    // Owner+group rw only: the socket has no auth of its own, so file perms are the gate.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660)) {
            eprintln!("control socket chmod {path} failed: {e}");
        }
    }
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                // One worker per connection so a stalled client can't block others.
                let shared = shared.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_conn(stream, &shared) {
                        eprintln!("control conn error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("control accept error: {e}"),
        }
    }
}

fn handle_conn(stream: UnixStream, shared: &Arc<Mutex<Shared>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream).take(MAX_REQ_BYTES);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(()); // client closed
    }
    let req: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let resp = {
        // Recover a poisoned lock: a panic in the scan loop must not kill the control plane.
        let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
        dispatch(&mut guard, method, &params)
    };
    writer.write_all(resp.to_string().as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config;
    use crate::control::Shared;
    use crate::db::SqliteStore;
    use crate::probe::ProbeStatus;
    use crate::recorder::Recorder;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Mutex};

    #[test]
    fn round_trips_a_status_request() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ow-test-{}.sock", std::process::id()));
        let path_s = path.to_string_lossy().to_string();

        let shared = Arc::new(Mutex::new(Shared {
            clients: vec![],
            recorder: Recorder::new(10, 10),
            store: SqliteStore::open_in_memory().unwrap(),
            config: default_config(),
            probes: ProbeStatus::default(),
        }));
        let p2 = path_s.clone();
        let sh = shared.clone();
        std::thread::spawn(move || serve(&p2, sh));

        // wait for the socket to appear
        for _ in 0..100 {
            if std::path::Path::new(&path_s).exists() { break; }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let mut stream = UnixStream::connect(&path_s).unwrap();
        stream.write_all(b"{\"method\":\"status\",\"params\":{}}\n").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["enabled"], true);
        let _ = std::fs::remove_file(&path_s);
    }

    #[test]
    fn socket_is_chmod_0660() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir();
        let path = dir.join(format!("ow-perm-test-{}.sock", std::process::id()));
        let path_s = path.to_string_lossy().to_string();

        let shared = Arc::new(Mutex::new(Shared {
            clients: vec![],
            recorder: Recorder::new(10, 10),
            store: SqliteStore::open_in_memory().unwrap(),
            config: default_config(),
            probes: ProbeStatus::default(),
        }));
        let p2 = path_s.clone();
        let sh = shared.clone();
        std::thread::spawn(move || serve(&p2, sh));

        for _ in 0..100 {
            if std::path::Path::new(&path_s).exists() { break; }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let meta = std::fs::metadata(&path_s).unwrap();
        // 0o777 mask isolates the permission bits from the socket-file-type bits.
        assert_eq!(meta.permissions().mode() & 0o777, 0o660);
        let _ = std::fs::remove_file(&path_s);
    }
}
