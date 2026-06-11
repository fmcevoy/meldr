use std::path::Path;

use crate::error::{MeldrError, Result};

/// Write `value` as pretty-printed JSON to `path` atomically (tmp file + rename).
/// Creates parent directories if they don't exist.
pub fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    write_bytes_atomic(path, serde_json::to_string_pretty(value)?.as_bytes())
}

/// Write `content` to `path` atomically (tmp file + rename).
/// Creates parent directories if they don't exist.
pub fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".write-{}.tmp", std::process::id()));
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
}
