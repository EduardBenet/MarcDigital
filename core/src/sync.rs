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

/// Suffix for in-progress downloads. Chosen so `is_image` in the slideshow
/// never offers a partial file to the decoder.
const TEMP_SUFFIX: &str = ".tmp";

/// The actions needed to bring the local folder in line with the cloud.
/// `BTreeSet` inputs give deterministic, sorted output (nice for tests and logs).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncPlan {
    /// In the cloud but not local — fetch these.
    pub to_download: Vec<String>,
    /// Local but no longer in the cloud — remove these.
    pub to_delete: Vec<String>,
    /// Blob names refused as unsafe local filenames (see [`safe_name`]).
    /// Populated by [`run_sync`]; [`plan_sync`] always leaves it empty.
    pub rejected: Vec<String>,
    /// Individual downloads/deletes that errored, as `(name, reason)`.
    /// A failure here is survivable — the next sync retries it.
    pub failed: Vec<(String, String)>,
}

impl SyncPlan {
    /// True when the local folder already matches the cloud.
    pub fn is_noop(&self) -> bool {
        self.to_download.is_empty() && self.to_delete.is_empty()
    }

    /// True when the sync completed with nothing to report at all.
    pub fn is_clean_noop(&self) -> bool {
        self.is_noop() && self.rejected.is_empty() && self.failed.is_empty()
    }
}

/// Accept only blob names usable as a plain filename inside the photo folder.
///
/// Azure listing is flat, so a blob stored under a virtual folder comes back as
/// `2024/holiday.jpg`. Joining that onto the local directory points at a parent
/// that does not exist (the write fails *after* egress has been paid, and the
/// name never enters the local set, so it is re-downloaded on every single
/// run). A name containing `..` would escape the photo directory entirely.
///
/// This matters more from Phase 9 onward: the companion app lets family members
/// choose blob names, which makes these untrusted input.
pub fn safe_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    // Check separators explicitly: `Path` on Linux does not treat `\` as one,
    // but the name may have been produced on Windows.
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return false;
    }
    // Belt and braces: the name must be exactly one ordinary path component.
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// Compute the delta between the local photo set and the cloud photo set.
pub fn plan_sync(local: &BTreeSet<String>, cloud: &BTreeSet<String>) -> SyncPlan {
    SyncPlan {
        to_download: cloud.difference(local).cloned().collect(),
        to_delete: local.difference(cloud).cloned().collect(),
        ..SyncPlan::default()
    }
}

/// Read the names of the files currently in `local_dir`.
fn read_local(local_dir: &Path) -> Result<BTreeSet<String>> {
    Ok(std::fs::read_dir(local_dir)
        .with_context(|| format!("reading {local_dir:?}"))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect())
}

/// Fetch one blob and place it atomically.
///
/// The bytes land in `name.tmp` first and are renamed into place only once
/// complete, so a power cut — routine for a frame on a wall socket — can never
/// leave a truncated JPEG under the real name. That matters beyond a cosmetic
/// glitch: a truncated file *is* present in the local set, so the diff would
/// never re-download it and the photo would be silently missing forever.
async fn download_one(store: &impl PhotoStore, local_dir: &Path, name: &str) -> Result<()> {
    let bytes = store
        .download(name)
        .await
        .with_context(|| format!("downloading {name}"))?;

    let temp = local_dir.join(format!("{name}{TEMP_SUFFIX}"));
    let final_path = local_dir.join(name);

    std::fs::write(&temp, bytes).with_context(|| format!("writing {temp:?}"))?;
    if let Err(e) = std::fs::rename(&temp, &final_path) {
        // Do not leave the partial behind to be mistaken for a real photo.
        let _ = std::fs::remove_file(&temp);
        return Err(e).with_context(|| format!("renaming {temp:?} -> {final_path:?}"));
    }
    Ok(())
}

/// Bring `local_dir` in line with the cloud: list what's on disk, diff it
/// against `store`, download new blobs, delete stale ones. The folder's
/// contents *are* the manifest — no separate state file to drift out of sync.
///
/// Only a failure to *plan* (listing local or cloud) aborts. Individual blob
/// failures are collected into [`SyncPlan::failed`] and the rest of the sync
/// proceeds: one flaky response must not leave the frame with a partial photo
/// set until somebody power-cycles it.
pub async fn run_sync(store: &impl PhotoStore, local_dir: &Path) -> Result<SyncPlan> {
    std::fs::create_dir_all(local_dir).with_context(|| format!("creating {local_dir:?}"))?;

    let local = read_local(local_dir)?;
    let listed = store.list().await.context("listing cloud blobs")?;

    // Split the cloud listing into names we can safely write and names we cannot.
    let (cloud, rejected): (BTreeSet<String>, Vec<String>) =
        listed
            .into_iter()
            .fold((BTreeSet::new(), Vec::new()), |mut acc, name| {
                if safe_name(&name) {
                    acc.0.insert(name);
                } else {
                    acc.1.push(name);
                }
                acc
            });

    let mut plan = plan_sync(&local, &cloud);
    plan.rejected = rejected;

    let mut downloaded = Vec::new();
    for name in &plan.to_download {
        match download_one(store, local_dir, name).await {
            Ok(()) => downloaded.push(name.clone()),
            Err(e) => plan.failed.push((name.clone(), format!("{e:#}"))),
        }
    }
    // Report what actually landed, so callers never over-report success.
    plan.to_download = downloaded;

    let mut deleted = Vec::new();
    for name in &plan.to_delete {
        match std::fs::remove_file(local_dir.join(name)) {
            Ok(()) => deleted.push(name.clone()),
            // Already gone is the end state we wanted, not a failure. This is
            // routine rather than exotic: a stale `x.jpg.tmp` left by an
            // interrupted download is both a delete candidate *and* the temp
            // path the retry of `x.jpg` writes to, so the download above
            // consumes it before we get here.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => deleted.push(name.clone()),
            Err(e) => plan.failed.push((name.clone(), format!("deleting: {e}"))),
        }
    }
    plan.to_delete = deleted;

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

    #[test]
    fn accepts_ordinary_filenames() {
        for name in [
            "a.jpg",
            "IMG-20251018-WA0012.jpg",
            "holiday photo.png",
            ".hidden",
        ] {
            assert!(safe_name(name), "{name} should be accepted");
        }
    }

    #[test]
    fn rejects_names_that_are_not_a_single_plain_component() {
        for name in [
            "",
            ".",
            "..",
            "2024/holiday.jpg", // virtual folder in a flat listing
            "../escape.jpg",    // traversal out of the photo dir
            "/absolute.jpg",
            "windows\\style.jpg",
            "a/../../b.jpg",
        ] {
            assert!(!safe_name(name), "{name:?} should be rejected");
        }
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

    #[tokio::test]
    async fn unsafe_blob_names_are_rejected_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::fake::FakeStore::new(&[
            ("ok.jpg", b"fine".as_slice()),
            ("2024/nested.jpg", b"nope".as_slice()),
            ("../escape.jpg", b"nope".as_slice()),
        ]);

        let plan = run_sync(&store, dir.path()).await.unwrap();

        assert_eq!(plan.to_download, vec!["ok.jpg".to_string()]);
        assert_eq!(plan.rejected.len(), 2, "got {:?}", plan.rejected);
        assert!(plan.failed.is_empty());
        // Nothing escaped the photo directory.
        assert!(!dir.path().parent().unwrap().join("escape.jpg").exists());
    }

    #[tokio::test]
    async fn a_failing_download_does_not_abort_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::fake::FakeStore::new(&[
            ("a.jpg", b"aaa".as_slice()),
            ("z.jpg", b"zzz".as_slice()),
        ]);
        // Listed by the cloud but not downloadable: simulates a transient error.
        store
            .extra_listed
            .lock()
            .unwrap()
            .insert("m.jpg".to_string());

        let plan = run_sync(&store, dir.path()).await.unwrap();

        // Both good blobs landed even though the middle one failed.
        assert_eq!(
            plan.to_download,
            vec!["a.jpg".to_string(), "z.jpg".to_string()]
        );
        assert_eq!(plan.failed.len(), 1);
        assert_eq!(plan.failed[0].0, "m.jpg");
        assert!(dir.path().join("a.jpg").exists());
        assert!(dir.path().join("z.jpg").exists());
        // The failed one is absent, so the next sync will retry it.
        assert!(!dir.path().join("m.jpg").exists());
    }

    #[tokio::test]
    async fn downloads_leave_no_temp_files_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::fake::FakeStore::new(&[("a.jpg", b"aaa".as_slice())]);

        run_sync(&store, dir.path()).await.unwrap();

        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(TEMP_SUFFIX))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[tokio::test]
    async fn a_stale_temp_file_is_cleaned_up_by_the_next_sync() {
        // A power cut mid-download leaves `x.jpg.tmp`. The retry of `x.jpg`
        // writes to that very path and renames it into place, so the partial is
        // consumed rather than deleted separately - and the delete pass has to
        // treat the now-absent file as done, not as an error.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.jpg.tmp"), b"partial").unwrap();
        let store = crate::store::fake::FakeStore::new(&[("x.jpg", b"complete".as_slice())]);

        let plan = run_sync(&store, dir.path()).await.unwrap();

        assert_eq!(plan.to_download, vec!["x.jpg".to_string()]);
        assert_eq!(plan.to_delete, vec!["x.jpg.tmp".to_string()]);
        assert!(plan.failed.is_empty(), "got {:?}", plan.failed);
        assert!(!dir.path().join("x.jpg.tmp").exists());
        assert_eq!(
            std::fs::read(dir.path().join("x.jpg")).unwrap(),
            b"complete"
        );
    }
}
