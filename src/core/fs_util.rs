use std::path::Path;

use crate::error::{MeldrError, Result};

/// Write `value` as pretty-printed JSON to `path` atomically (tmp file + rename).
/// Creates parent directories if they don't exist.
pub fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    write_bytes_atomic(path, serde_json::to_string_pretty(value)?.as_bytes())
}

/// Write `content` to `path` atomically (tmp file + rename).
/// Creates parent directories if they don't exist.
///
/// The temp name mixes the pid with a process-global counter and the destination
/// file name: keying it on the pid alone made two concurrent writes to the same
/// directory from one process race on a single `.write-<pid>.tmp`.
pub fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;

    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".write-{}-{}-{stem}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));

    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        MeldrError::Io(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_json_atomic_round_trips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("out.json");
        let v = json!({"key": "value"});
        write_json_atomic(&path, &v).unwrap();
        let back: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn write_bytes_atomic_creates_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("a/b/c/out.txt");
        write_bytes_atomic(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn concurrent_writes_to_same_dir_do_not_clobber() {
        // Regression: the temp name used to be `.write-<pid>.tmp`, so two threads
        // writing different files into one directory raced on the same temp path
        // and could truncate or steal each other's content.
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let body = vec![b'a' + i as u8; 4096];
                    for round in 0..20 {
                        let path = dir.join(format!("file-{i}-{round}.bin"));
                        write_bytes_atomic(&path, &body).unwrap();
                        assert_eq!(std::fs::read(&path).unwrap(), body, "clobbered {i}/{round}");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        // No temp files may survive a clean run.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".write-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }
}
