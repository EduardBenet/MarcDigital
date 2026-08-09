//! Runtime configuration, loaded from environment variables.
//!
//! Fails fast: if any required credential/setting is missing or malformed the
//! app refuses to start (no baked-in defaults for secrets). Parsing is written
//! against a generic getter so it can be unit-tested without touching the
//! process-global environment.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};

/// Required (no defaults — missing means we refuse to start).
const REQUIRED: &[&str] = &[
    "AZURE_TENANT_ID",
    "AZURE_CLIENT_ID",
    "AZURE_CLIENT_SECRET",
    "AZURE_STORAGE_ACCOUNT",
    "CONTAINER_NAME",
];

/// Optional setting names + their defaults.
const DEFAULT_PHOTO_DIR: &str = "./synced_photos";
const DEFAULT_ROTATION_SECONDS: u64 = 30;
const DEFAULT_SYNC_INTERVAL_SECONDS: u64 = 1800; // 30 minutes
const DEFAULT_DISPLAY_ROTATION: u16 = 0;

/// Fully-validated configuration for a running frame.
///
/// `Debug` is implemented by hand so `client_secret` can be redacted: a stray
/// `{config:?}`, an `anyhow` context line, or a panic payload would otherwise
/// print the Entra secret straight into the balena device log, which shows
/// service variables in plaintext to anyone with fleet access.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    // Entra service-principal credentials (used to obtain read-only tokens).
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,

    // Azure Blob location.
    pub storage_account: String,
    pub container: String,

    // Local behavior.
    pub photo_dir: PathBuf,
    /// How long each photo stays on screen.
    pub rotation: Duration,
    pub sync_interval: Duration,
    /// Clockwise screen rotation in degrees: 0, 90, 180 or 270.
    ///
    /// Needed because a panel's native orientation and the frame's mounting
    /// orientation are independent: the Touch Display 2 is natively 720x1280
    /// portrait, so a frame hung landscape needs 90 or 270 here. Done in the
    /// renderer rather than via a balena host config because the KMS rotation
    /// knobs are fiddly and a wrong one costs a reboot cycle to discover.
    pub display_rotation: u16,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("tenant_id", &self.tenant_id)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("storage_account", &self.storage_account)
            .field("container", &self.container)
            .field("photo_dir", &self.photo_dir)
            .field("rotation", &self.rotation)
            .field("sync_interval", &self.sync_interval)
            .field("display_rotation", &self.display_rotation)
            .finish()
    }
}

impl Config {
    /// Load from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    /// Load from an arbitrary source. `get` returns `Some(value)` when a key is
    /// set. Kept generic so tests can supply a fake environment.
    pub fn from_source<F>(get: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        // Collect *all* missing required vars so the operator sees them at once,
        // rather than fixing them one failed boot at a time.
        let missing: Vec<&str> = REQUIRED
            .iter()
            .copied()
            .filter(|k| get(k).map(|v| v.trim().is_empty()).unwrap_or(true))
            .collect();
        if !missing.is_empty() {
            bail!(
                "missing required environment variables: {}",
                missing.join(", ")
            );
        }

        // Safe to unwrap the required ones now.
        let req = |key: &str| get(key).unwrap();

        let photo_dir = get("SYNCED_PHOTOS_DIR")
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_PHOTO_DIR.to_string());

        let rotation = parse_seconds(
            get("ROTATION_SECONDS"),
            DEFAULT_ROTATION_SECONDS,
            "ROTATION_SECONDS",
        )?;
        let sync_interval = parse_seconds(
            get("SYNC_INTERVAL_SECONDS"),
            DEFAULT_SYNC_INTERVAL_SECONDS,
            "SYNC_INTERVAL_SECONDS",
        )?;

        let display_rotation = parse_display_rotation(get("DISPLAY_ROTATION"))?;

        Ok(Config {
            tenant_id: req("AZURE_TENANT_ID"),
            client_id: req("AZURE_CLIENT_ID"),
            client_secret: req("AZURE_CLIENT_SECRET"),
            storage_account: req("AZURE_STORAGE_ACCOUNT"),
            container: req("CONTAINER_NAME"),
            photo_dir: PathBuf::from(photo_dir),
            rotation,
            sync_interval,
            display_rotation,
        })
    }
}

/// Parse `DISPLAY_ROTATION`, which must be one of 0/90/180/270.
///
/// Rejecting anything else is deliberate: an arbitrary angle would render the
/// slideshow askew with no obvious cause, and on an unattended frame nobody is
/// watching the logs. Better to refuse to start with a clear message.
fn parse_display_rotation(value: Option<String>) -> Result<u16> {
    let Some(raw) = value.filter(|v| !v.trim().is_empty()) else {
        return Ok(DEFAULT_DISPLAY_ROTATION);
    };
    match raw.trim() {
        "0" => Ok(0),
        "90" => Ok(90),
        "180" => Ok(180),
        "270" => Ok(270),
        other => bail!("DISPLAY_ROTATION must be 0, 90, 180 or 270, got {other:?}"),
    }
}

/// Parse an optional seconds value, falling back to `default`. A present but
/// non-numeric or zero value is a hard error (silently ignoring a typo'd
/// interval would be worse than refusing to start).
fn parse_seconds(value: Option<String>, default: u64, name: &str) -> Result<Duration> {
    match value {
        None => Ok(Duration::from_secs(default)),
        Some(v) if v.trim().is_empty() => Ok(Duration::from_secs(default)),
        Some(v) => {
            let n: u64 = v.trim().parse().map_err(|_| {
                anyhow::anyhow!("{} must be a whole number of seconds, got {:?}", name, v)
            })?;
            if n == 0 {
                bail!("{} must be greater than zero", name);
            }
            Ok(Duration::from_secs(n))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a getter over a fixed map (no global env involved).
    fn source(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    fn all_required() -> Vec<(&'static str, &'static str)> {
        vec![
            ("AZURE_TENANT_ID", "tenant"),
            ("AZURE_CLIENT_ID", "client"),
            ("AZURE_CLIENT_SECRET", "secret"),
            ("AZURE_STORAGE_ACCOUNT", "acct"),
            ("CONTAINER_NAME", "padrina"),
        ]
    }

    #[test]
    fn loads_when_all_required_present_and_applies_defaults() {
        let cfg = Config::from_source(source(&all_required())).expect("should load");
        assert_eq!(cfg.tenant_id, "tenant");
        assert_eq!(cfg.container, "padrina");
        assert_eq!(cfg.photo_dir, PathBuf::from(DEFAULT_PHOTO_DIR));
        assert_eq!(cfg.rotation, Duration::from_secs(DEFAULT_ROTATION_SECONDS));
        assert_eq!(
            cfg.sync_interval,
            Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECONDS)
        );
    }

    #[test]
    fn overrides_optional_values() {
        let mut pairs = all_required();
        pairs.push(("SYNCED_PHOTOS_DIR", "/data/photos"));
        pairs.push(("ROTATION_SECONDS", "10"));
        pairs.push(("SYNC_INTERVAL_SECONDS", "60"));
        let cfg = Config::from_source(source(&pairs)).expect("should load");
        assert_eq!(cfg.photo_dir, PathBuf::from("/data/photos"));
        assert_eq!(cfg.rotation, Duration::from_secs(10));
        assert_eq!(cfg.sync_interval, Duration::from_secs(60));
    }

    #[test]
    fn display_rotation_defaults_to_zero() {
        let cfg = Config::from_source(source(&all_required())).expect("should load");
        assert_eq!(cfg.display_rotation, DEFAULT_DISPLAY_ROTATION);
    }

    #[test]
    fn accepts_the_four_quarter_turns() {
        for (raw, expected) in [("0", 0u16), ("90", 90), ("180", 180), ("270", 270)] {
            let mut pairs = all_required();
            pairs.push(("DISPLAY_ROTATION", raw));
            let cfg = Config::from_source(source(&pairs)).expect("should load");
            assert_eq!(cfg.display_rotation, expected, "for {raw}");
        }
    }

    #[test]
    fn rejects_a_rotation_that_is_not_a_quarter_turn() {
        for raw in ["45", "-90", "360", "portrait", "90deg"] {
            let mut pairs = all_required();
            pairs.push(("DISPLAY_ROTATION", raw));
            let err = Config::from_source(source(&pairs)).unwrap_err().to_string();
            assert!(err.contains("DISPLAY_ROTATION"), "for {raw}, got: {err}");
        }
    }

    #[test]
    fn missing_required_lists_all_of_them() {
        let pairs = vec![("AZURE_TENANT_ID", "tenant")]; // only one present
        let err = Config::from_source(source(&pairs)).unwrap_err().to_string();
        assert!(err.contains("AZURE_CLIENT_ID"), "got: {err}");
        assert!(err.contains("AZURE_CLIENT_SECRET"), "got: {err}");
        assert!(err.contains("AZURE_STORAGE_ACCOUNT"), "got: {err}");
        assert!(err.contains("CONTAINER_NAME"), "got: {err}");
        assert!(
            !err.contains("AZURE_TENANT_ID"),
            "present var should not be listed: {err}"
        );
    }

    #[test]
    fn blank_required_counts_as_missing() {
        let mut pairs = all_required();
        // Blank out the secret.
        pairs
            .iter_mut()
            .find(|(k, _)| *k == "AZURE_CLIENT_SECRET")
            .unwrap()
            .1 = "   ";
        let err = Config::from_source(source(&pairs)).unwrap_err().to_string();
        assert!(err.contains("AZURE_CLIENT_SECRET"), "got: {err}");
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let mut pairs = all_required();
        pairs
            .iter_mut()
            .find(|(k, _)| *k == "AZURE_CLIENT_SECRET")
            .unwrap()
            .1 = "super-secret-value";
        let cfg = Config::from_source(source(&pairs)).expect("should load");

        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("super-secret-value"),
            "secret leaked into Debug output: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "got: {rendered}");
        // The non-sensitive fields are still useful for diagnostics.
        assert!(rendered.contains("acct"), "got: {rendered}");
    }

    #[test]
    fn non_numeric_interval_is_an_error() {
        let mut pairs = all_required();
        pairs.push(("ROTATION_SECONDS", "soon"));
        let err = Config::from_source(source(&pairs)).unwrap_err().to_string();
        assert!(err.contains("ROTATION_SECONDS"), "got: {err}");
    }

    #[test]
    fn zero_interval_is_an_error() {
        let mut pairs = all_required();
        pairs.push(("SYNC_INTERVAL_SECONDS", "0"));
        let err = Config::from_source(source(&pairs)).unwrap_err().to_string();
        assert!(err.contains("greater than zero"), "got: {err}");
    }
}
