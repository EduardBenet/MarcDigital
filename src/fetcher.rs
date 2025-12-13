use azure_core::Result;
use azure_storage_blob::models::BlobClientDownloadOptions;
use azure_storage_blob::{BlobClient, BlobContainerClient, BlobContainerClientOptions};
use futures::TryStreamExt;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::io::{self, BufRead};
use std::path::Path;
use url::Url;

#[tokio::main]
pub async fn sync_folder(
    account_name: &str,
    key: &str,
    container: &str,
    local_path: &Path,
) -> Result<()> {
    let account = format!(
        "https://{}.blob.core.windows.net/{}?{}",
        account_name, container, key
    );

    let client = BlobContainerClient::from_url(
        Url::parse(&account).expect("Failed to parse URL"),
        None,
        Some(BlobContainerClientOptions::default()),
    )?;

    let mut pager = client.list_blobs(None)?;

    let mut cloud_blobs: HashMap<String, BlobClient> = HashMap::new();
    while let Some(blob) = pager.try_next().await.unwrap() {
        if let Some(blob_name) = &blob.name {
            if let Some(name_str) = &blob_name.content {
                // Insert into HashMap (clone the key, move the blob)
                cloud_blobs.insert(name_str.clone(), client.blob_client(&name_str));
            }
        }
    }

    let cloud_photos: HashSet<String> = cloud_blobs.keys().cloned().collect();

    let manifest_path = local_path.join("manifest.txt");

    let local_photos: HashSet<String> = read_manifest(&manifest_path).unwrap_or_default();

    let to_delete: HashSet<String> = local_photos.difference(&cloud_photos).cloned().collect();

    for photo_name in &to_delete {
        let file_path = local_path.join(photo_name);
        if Path::new(&file_path).is_file() {
            std::fs::remove_file(&file_path)?;
            println!("Deleted {}", photo_name);
        } else {
            println!("File {} does not exist, skipping deletion.", photo_name);
        }
    }

    let to_download: HashSet<String> = cloud_photos.difference(&local_photos).cloned().collect();

    for photo_name in &to_download {
        if let Some(blob) = cloud_blobs.get(photo_name) {
            // Call your download function here
            println!("Downloading {}", photo_name);

            let response = blob
                .download(Some(BlobClientDownloadOptions::default()))
                .await?;

            let file_path = local_path.join(photo_name);
            let bytes = response.into_body().collect().await?;

            std::fs::write(&file_path, bytes)?;
        }
    }

    // write new manifest
    write_manifest(&manifest_path)?;

    Ok(())
}

fn read_manifest<P: AsRef<Path>>(path: P) -> io::Result<HashSet<String>> {
    let file = File::open(path)?;
    let reader = io::BufReader::new(file);

    let mut local_files = HashSet::new();
    for line in reader.lines() {
        local_files.insert(line?);
    }

    Ok(local_files)
}

fn write_manifest(manifest_path: &Path) -> io::Result<()> {
    // Update manifest file by syncing the current state of the folder
    println!("Updating manifest at {:?}", &manifest_path);

    let dir = manifest_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "No parent directory"))?;

    let read_dir = dir.read_dir()?;
    let mut manifest_file = File::create(&manifest_path)?;

    for entry in read_dir {
        let fname_os = entry?.file_name();
        let fname = fname_os.to_string_lossy();

        if fname != "manifest.txt" {
            writeln!(manifest_file, "{}", fname)?;
        }
    }

    Ok(())
}
