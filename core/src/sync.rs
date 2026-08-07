//! Pure sync-diff logic: given what's on disk and what's in the cloud, decide
//! what to download and what to delete. No I/O, no async, no network — so it's
//! trivially unit-testable and fast.
//!
//! This is the **cost-critical** piece: the frames only ever pull the delta, so
//! a bug that over-reports `to_download` translates directly into Azure egress
//! charges. Keep it simple and well-tested.
//!
//! Photos are keyed by **blob name** only (the manifest stores names). A photo
//! replaced in-place under the same name is intentionally *not* re-downloaded;
//! curation adds/removes distinct filenames, which is the workflow we support.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::store::PhotoStore;

/// The actions needed to bring the local folder in line with the cloud.
/// `BTreeSet` inputs give deterministic, sorted output (nice for tests and logs).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncPlan {
    /// In the cloud but not local — fetch these.
    pub to_download: Vec<String>,
    /// Local but no longer in the cloud — remove these.
    pub to_delete: Vec<String>,
}

impl SyncPlan {
    /// True when the local folder already matches the cloud.
    pub fn is_noop(&self) -> bool {
        self.to_download.is_empty() && self.to_delete.is_empty()
    }
}

/// Compute the delta between the local photo set and the cloud photo set.
pub fn plan_sync(local: &BTreeSet<String>, cloud: &BTreeSet<String>) -> SyncPlan {
    SyncPlan {
        to_download: cloud.difference(local).cloned().collect(),
        to_delete: local.difference(cloud).cloned().collect(),
    }
}

/// Bring `local_dir` in line with the cloud: list what's on disk, diff it
/// against `store`, download new blobs, delete stale ones. The folder's
/// contents *are* the manifest — no separate state file to drift out of sync.
pub async fn run_sync(store: &impl PhotoStore, local_dir: &Path) -> Result<SyncPlan> {
    std::fs::create_dir_all(local_dir).with_context(|| format!("creating {local_dir:?}"))?;

    let local: BTreeSet<String> = std::fs::read_dir(local_dir)
        .with_context(|| format!("reading {local_dir:?}"))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();

    let cloud = store.list().await.context("listing cloud blobs")?;
    let plan = plan_sync(&local, &cloud);

    for name in &plan.to_download {
        let bytes = store
            .download(name)
            .await
            .with_context(|| format!("downloading {name}"))?;
        std::fs::write(local_dir.join(name), bytes).with_context(|| format!("writing {name}"))?;
    }
    for name in &plan.to_delete {
        std::fs::remove_file(local_dir.join(name)).with_context(|| format!("deleting {name}"))?;
    }

    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_change_is_a_noop() {
        let local = set(&["a.jpg", "b.jpg"]);
        let cloud = set(&["a.jpg", "b.jpg"]);
        let plan = plan_sync(&local, &cloud);
        assert!(plan.is_noop());
        assert!(plan.to_download.is_empty());
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn add_only_downloads_new_blobs() {
        let local = set(&["a.jpg"]);
        let cloud = set(&["a.jpg", "b.jpg", "c.jpg"]);
        let plan = plan_sync(&local, &cloud);
        assert_eq!(plan.to_download, vec!["b.jpg", "c.jpg"]);
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn delete_only_removes_stale_local_files() {
        let local = set(&["a.jpg", "b.jpg", "c.jpg"]);
        let cloud = set(&["a.jpg"]);
        let plan = plan_sync(&local, &cloud);
        assert!(plan.to_download.is_empty());
        assert_eq!(plan.to_delete, vec!["b.jpg", "c.jpg"]);
    }

    #[test]
    fn mixed_add_and_delete() {
        let local = set(&["keep.jpg", "old.jpg"]);
        let cloud = set(&["keep.jpg", "new.jpg"]);
        let plan = plan_sync(&local, &cloud);
        assert_eq!(plan.to_download, vec!["new.jpg"]);
        assert_eq!(plan.to_delete, vec!["old.jpg"]);
    }

    #[test]
    fn empty_manifest_downloads_everything() {
        let local = set(&[]);
        let cloud = set(&["a.jpg", "b.jpg"]);
        let plan = plan_sync(&local, &cloud);
        assert_eq!(plan.to_download, vec!["a.jpg", "b.jpg"]);
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn empty_cloud_deletes_everything() {
        let local = set(&["a.jpg", "b.jpg"]);
        let cloud = set(&[]);
        let plan = plan_sync(&local, &cloud);
        assert!(plan.to_download.is_empty());
        assert_eq!(plan.to_delete, vec!["a.jpg", "b.jpg"]);
    }

    #[test]
    fn both_empty_is_a_noop() {
        let plan = plan_sync(&set(&[]), &set(&[]));
        assert!(plan.is_noop());
    }

    #[test]
    fn same_name_is_not_redownloaded() {
        // A blob replaced under the same name is not re-fetched (documented behavior).
        let local = set(&["photo.jpg"]);
        let cloud = set(&["photo.jpg"]);
        assert!(plan_sync(&local, &cloud).is_noop());
    }

    #[tokio::test]
    async fn run_sync_downloads_and_deletes_against_a_fake_store() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stale.jpg"), b"old").unwrap();

        let store = crate::store::fake::FakeStore::new(&[
            ("new.jpg", b"fresh".as_slice()),
            ("stale.jpg", b"should not overwrite local".as_slice()),
        ]);
        // Local already has "stale.jpg" too, so only "new.jpg" is a real download;
        // remove it from the fake's cloud set to test the delete path instead.
        store.blobs.lock().unwrap().remove("stale.jpg");

        let plan = run_sync(&store, dir.path()).await.unwrap();

        assert_eq!(plan.to_download, vec!["new.jpg".to_string()]);
        assert_eq!(plan.to_delete, vec!["stale.jpg".to_string()]);
        assert_eq!(std::fs::read(dir.path().join("new.jpg")).unwrap(), b"fresh");
        assert!(!dir.path().join("stale.jpg").exists());
    }
}
