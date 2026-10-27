use async_trait::async_trait;
use paysleep_application::{Storage, StorageEntry};
use paysleep_core::{DomainError, Result};
use proton_drive::{Client, DownloadOptions, NodeType, UploadOptions};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub use proton_drive::CredentialsStore;

#[derive(Clone, Debug)]
pub struct ProtonDriveStorage {
    client: Client,
    remote_path: String,
    temporary_root: PathBuf,
}

impl ProtonDriveStorage {
    pub fn new(
        binary: PathBuf,
        remote_path: String,
        data_dir: &Path,
        state_dir: PathBuf,
        credentials_store: CredentialsStore,
    ) -> Result<Self> {
        let client = Client::new(binary)
            .map_err(proton_error)?
            .with_cache_dir(state_dir)
            .with_credentials_store(credentials_store);
        Ok(Self {
            client,
            remote_path,
            temporary_root: data_dir.join("proton-transfer"),
        })
    }

    fn remote_key(&self, key: &str) -> String {
        format!("{}/{}", self.remote_path.trim_end_matches('/'), key)
    }
}

#[async_trait]
impl Storage for ProtonDriveStorage {
    async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let staging_root = self.temporary_root.join(Uuid::new_v4().to_string());
        let local = staging_root.join(key);
        let parent = local
            .parent()
            .ok_or_else(|| DomainError::Storage("destination Proton invalide".to_owned()))?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(storage_error)?;
        tokio::fs::write(&local, data)
            .await
            .map_err(storage_error)?;
        let client = self.client.clone();
        let remote = self.remote_path.clone();
        let upload_path = key
            .split('/')
            .next()
            .map(|component| staging_root.join(component))
            .ok_or_else(|| DomainError::Storage("clé Proton vide".to_owned()))?;
        let result = tokio::task::spawn_blocking(move || {
            let options = UploadOptions {
                file_conflict: Some(proton_drive::UploadFileConflict::CreateNewRevision),
                folder_conflict: Some(proton_drive::UploadFolderConflict::Merge),
                skip_thumbnails: true,
            };
            client.upload(&[upload_path.as_os_str()], &remote, &options)
        })
        .await
        .map_err(join_error)?
        .map_err(proton_error)
        .map(|_| ());
        let _ = tokio::fs::remove_dir_all(staging_root).await;
        result
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let folder = self.temporary_root.join(Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&folder)
            .await
            .map_err(storage_error)?;
        let client = self.client.clone();
        let remote = self.remote_key(key);
        let target = folder.clone();
        tokio::task::spawn_blocking(move || {
            client.download(&[&remote], &target, &DownloadOptions::default())
        })
        .await
        .map_err(join_error)?
        .map_err(proton_error)?;
        let file_name = Path::new(key)
            .file_name()
            .ok_or_else(|| DomainError::Storage("clé Proton invalide".to_owned()))?;
        let result = tokio::fs::read(folder.join(file_name))
            .await
            .map_err(storage_error);
        let _ = tokio::fs::remove_dir_all(folder).await;
        result
    }

    async fn list(&self) -> Result<Vec<StorageEntry>> {
        let client = self.client.clone();
        let remote = self.remote_path.clone();
        let objects =
            tokio::task::spawn_blocking(move || client.list(&remote, Some(NodeType::File)))
                .await
                .map_err(join_error)?
                .map_err(proton_error)?;
        Ok(objects
            .into_iter()
            .filter_map(|object| {
                Some(StorageEntry {
                    key: object.name()?.to_owned(),
                    size: object.claimed_size(),
                })
            })
            .collect())
    }

    async fn health(&self) -> Result<()> {
        let client = self.client.clone();
        let remote = self.remote_path.clone();
        tokio::task::spawn_blocking(move || {
            client.probe()?;
            client.list(&remote, None).map(|_| ())
        })
        .await
        .map_err(join_error)?
        .map_err(proton_error)
    }
}

fn storage_error(error: std::io::Error) -> DomainError {
    DomainError::Storage(error.to_string())
}

fn proton_error(error: proton_drive::Error) -> DomainError {
    DomainError::Storage(error.to_string())
}

fn join_error(error: tokio::task::JoinError) -> DomainError {
    DomainError::Storage(error.to_string())
}
