//! Abstraction over the cloud photo source, so sync logic can be tested
//! against a fake without ever touching Azure.

use std::collections::BTreeSet;

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait PhotoStore: Send + Sync {
    /// Names of all blobs currently in the cloud container.
    async fn list(&self) -> Result<BTreeSet<String>>;
    /// Fetch the full contents of one blob.
    async fn download(&self, name: &str) -> Result<Vec<u8>>;
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory `PhotoStore` for exercising sync logic without a network.
    #[derive(Default)]
    pub struct FakeStore {
        pub blobs: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl FakeStore {
        pub fn new(blobs: &[(&str, &[u8])]) -> Self {
            Self {
                blobs: Mutex::new(
                    blobs
                        .iter()
                        .map(|(name, bytes)| (name.to_string(), bytes.to_vec()))
                        .collect(),
                ),
            }
        }
    }

    #[async_trait]
    impl PhotoStore for FakeStore {
        async fn list(&self) -> Result<BTreeSet<String>> {
            Ok(self.blobs.lock().unwrap().keys().cloned().collect())
        }

        async fn download(&self, name: &str) -> Result<Vec<u8>> {
            self.blobs
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no such blob: {name}"))
        }
    }
}
