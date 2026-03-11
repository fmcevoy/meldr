use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{MeldrError, Result};

/// A snapshot of package HEAD SHAs taken before a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSnapshot {
    pub timestamp: u64,
    pub branch: String,
    pub packages: HashMap<String, String>,
}

/// A log entry recording the outcome of a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncLogEntry {
    pub timestamp: u64,
    pub branch: String,
    pub outcomes: Vec<PackageSyncLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSyncLogEntry {
    pub package: String,
    pub status: String,
    pub method: String,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

fn snapshots_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".meldr").join("sync-snapshots")
}

fn sync_log_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".meldr").join("sync-log.jsonl")
}

pub fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn save_snapshot(workspace_root: &Path, snapshot: &SyncSnapshot) -> Result<()> {
    let dir = snapshots_dir(workspace_root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", snapshot.timestamp));
    let content = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn load_latest_snapshot(workspace_root: &Path, branch: &str) -> Result<Option<SyncSnapshot>> {
    let dir = snapshots_dir(workspace_root);
    if !dir.exists() {
        return Ok(None);
    }

    let mut snapshots: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();

    // Sort descending by filename (which is the timestamp)
    snapshots.sort_by(|a, b| b.cmp(a));

    for path in snapshots {
        let content = std::fs::read_to_string(&path)?;
        let snapshot: SyncSnapshot = serde_json::from_str(&content)?;
        if snapshot.branch == branch {
            return Ok(Some(snapshot));
        }
    }

    Ok(None)
}

pub fn prune_snapshots(workspace_root: &Path, keep: usize) -> Result<()> {
    let dir = snapshots_dir(workspace_root);
    if !dir.exists() {
        return Ok(());
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();

    // Sort descending — newest first
    files.sort_by(|a, b| b.cmp(a));

    // Remove everything after `keep`
    for path in files.iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }

    Ok(())
}

pub fn append_log(workspace_root: &Path, entry: &SyncLogEntry) -> Result<()> {
    let dir = workspace_root.join(".meldr");
    std::fs::create_dir_all(&dir)?;
    let path = sync_log_path(workspace_root);

    let line = serde_json::to_string(entry)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(MeldrError::Io)?;
    writeln!(file, "{line}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_load_snapshot() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".meldr")).unwrap();

        let mut packages = HashMap::new();
        packages.insert("frontend".to_string(), "abc123".to_string());
        packages.insert("backend".to_string(), "def456".to_string());

        let snapshot = SyncSnapshot {
            timestamp: 1000,
            branch: "feature-x".to_string(),
            packages,
        };

        save_snapshot(root, &snapshot).unwrap();

        let loaded = load_latest_snapshot(root, "feature-x").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.branch, "feature-x");
        assert_eq!(loaded.packages.len(), 2);
        assert_eq!(loaded.packages["frontend"], "abc123");
    }

    #[test]
    fn test_load_latest_returns_most_recent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".meldr")).unwrap();

        let mut pkgs1 = HashMap::new();
        pkgs1.insert("frontend".to_string(), "old_sha".to_string());
        save_snapshot(
            root,
            &SyncSnapshot {
                timestamp: 1000,
                branch: "feature-x".to_string(),
                packages: pkgs1,
            },
        )
        .unwrap();

        let mut pkgs2 = HashMap::new();
        pkgs2.insert("frontend".to_string(), "new_sha".to_string());
        save_snapshot(
            root,
            &SyncSnapshot {
                timestamp: 2000,
                branch: "feature-x".to_string(),
                packages: pkgs2,
            },
        )
        .unwrap();

        let loaded = load_latest_snapshot(root, "feature-x").unwrap().unwrap();
        assert_eq!(loaded.timestamp, 2000);
        assert_eq!(loaded.packages["frontend"], "new_sha");
    }

    #[test]
    fn test_load_filters_by_branch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".meldr")).unwrap();

        save_snapshot(
            root,
            &SyncSnapshot {
                timestamp: 1000,
                branch: "branch-a".to_string(),
                packages: HashMap::new(),
            },
        )
        .unwrap();

        save_snapshot(
            root,
            &SyncSnapshot {
                timestamp: 2000,
                branch: "branch-b".to_string(),
                packages: HashMap::new(),
            },
        )
        .unwrap();

        let loaded = load_latest_snapshot(root, "branch-a").unwrap().unwrap();
        assert_eq!(loaded.branch, "branch-a");
        assert_eq!(loaded.timestamp, 1000);

        let none = load_latest_snapshot(root, "nonexistent").unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn test_prune_snapshots() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".meldr")).unwrap();

        for ts in [1000, 2000, 3000, 4000, 5000] {
            save_snapshot(
                root,
                &SyncSnapshot {
                    timestamp: ts,
                    branch: "main".to_string(),
                    packages: HashMap::new(),
                },
            )
            .unwrap();
        }

        prune_snapshots(root, 2).unwrap();

        let dir = snapshots_dir(root);
        let mut remaining: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        remaining.sort();
        assert_eq!(remaining.len(), 2);
        // The two newest snapshots (4000, 5000) should be kept
        assert_eq!(remaining, vec!["4000.json", "5000.json"]);

        // Verify the oldest snapshots were actually removed
        assert!(!dir.join("1000.json").exists());
        assert!(!dir.join("2000.json").exists());
        assert!(!dir.join("3000.json").exists());

        // Verify kept files are valid and contain expected data
        let content = std::fs::read_to_string(dir.join("5000.json")).unwrap();
        let snap: SyncSnapshot = serde_json::from_str(&content).unwrap();
        assert_eq!(snap.timestamp, 5000);
        assert_eq!(snap.branch, "main");
    }

    #[test]
    fn test_append_log() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".meldr")).unwrap();

        let entry = SyncLogEntry {
            timestamp: 1000,
            branch: "feature-x".to_string(),
            outcomes: vec![PackageSyncLogEntry {
                package: "frontend".to_string(),
                status: "synced".to_string(),
                method: "rebase".to_string(),
                ahead: Some(0),
                behind: Some(3),
            }],
        };

        append_log(root, &entry).unwrap();
        append_log(root, &entry).unwrap();

        let content = std::fs::read_to_string(sync_log_path(root)).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line should be valid JSON
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();

            // Verify expected top-level fields
            assert_eq!(parsed["timestamp"], 1000);
            assert_eq!(parsed["branch"], "feature-x");

            // Verify outcomes array
            let outcomes = parsed["outcomes"].as_array().unwrap();
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0]["package"], "frontend");
            assert_eq!(outcomes[0]["status"], "synced");
            assert_eq!(outcomes[0]["method"], "rebase");
            assert_eq!(outcomes[0]["ahead"], 0);
            assert_eq!(outcomes[0]["behind"], 3);
        }

        // Verify each line can be deserialized back to SyncLogEntry
        let deserialized: SyncLogEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(deserialized.branch, "feature-x");
        assert_eq!(deserialized.outcomes[0].package, "frontend");
        assert_eq!(deserialized.outcomes[0].ahead, Some(0));
        assert_eq!(deserialized.outcomes[0].behind, Some(3));
    }

    #[test]
    fn test_no_snapshots_dir_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let loaded = load_latest_snapshot(tmp.path(), "any").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_prune_nonexistent_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Should not error
        prune_snapshots(tmp.path(), 5).unwrap();
    }
}
