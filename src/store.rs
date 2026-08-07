//! Azure Blob Storage implementation of `PhotoStore`, authenticated with an
//! Entra service principal (no SAS/key baked into the URL — the SDK
//! auto-refreshes short-lived tokens via `ClientSecretCredential`).

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use async_trait::async_trait;
use azure_core::credentials::Secret;
use azure_identity::ClientSecretCredential;
use azure_storage_blob::models::BlobClientDownloadOptions;
use azure_storage_blob::BlobContainerClient;
use futures::TryStreamExt;
use marcdigital_core::store::PhotoStore;
use url::Url;

pub struct AzureBlobStore {
    client: BlobContainerClient,
}

impl AzureBlobStore {
    pub fn new(
        tenant_id: &str,
        client_id: &str,
        client_secret: &str,
        storage_account: &str,
        container: &str,
    ) -> Result<Self> {
        let credential = ClientSecretCredential::new(
            tenant_id,
            client_id.to_string(),
            Secret::from(client_secret.to_string()),
            None,
        )
        .context("building Entra client-secret credential")?;

        let url = Url::parse(&format!(
            "https://{storage_account}.blob.core.windows.net/{container}"
        ))
        .context("building container URL")?;

        let client = BlobContainerClient::new(url, Some(credential), None)
            .context("building blob container client")?;

        Ok(Self { client })
    }
}

#[async_trait]
impl PhotoStore for AzureBlobStore {
    async fn list(&self) -> Result<BTreeSet<String>> {
        let mut pager = self.client.list_blobs(None).context("listing blobs")?;
        let mut names = BTreeSet::new();
        while let Some(blob) = pager.try_next().await.context("paging blob list")? {
            if let Some(name) = blob.name {
                names.insert(name);
            }
        }
        Ok(names)
    }

    async fn download(&self, name: &str) -> Result<Vec<u8>> {
        let response = self
            .client
            .blob_client(name)
            .download(Some(BlobClientDownloadOptions::default()))
            .await
            .with_context(|| format!("downloading {name}"))?;
        let bytes = response
            .body
            .collect()
            .await
            .with_context(|| format!("reading body for {name}"))?;
        Ok(bytes.to_vec())
    }
}
