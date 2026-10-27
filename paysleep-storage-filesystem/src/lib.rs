use async_trait::async_trait;
use paysleep_application::{PayslipRepository, Storage, StorageEntry};
use paysleep_core::{DomainError, Payslip, Result};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[derive(Debug)]
pub struct FileSystemStorage {
    root: PathBuf,
}

impl FileSystemStorage {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let root = data_dir.join("files");
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(storage_error)?;
        Ok(Self { root })
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        let relative = Path::new(key);
        if key.is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(DomainError::Storage("clé de stockage invalide".to_owned()));
        }
        Ok(self.root.join(relative))
    }
}

#[async_trait]
impl Storage for FileSystemStorage {
    async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let destination = self.path(key)?;
        let parent = destination
            .parent()
            .ok_or_else(|| DomainError::Storage("destination sans dossier parent".to_owned()))?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(storage_error)?;
        let temporary = parent.join(format!(".upload-{}", Uuid::new_v4()));
        tokio::fs::write(&temporary, data)
            .await
            .map_err(storage_error)?;
        if let Err(error) = tokio::fs::rename(&temporary, &destination).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(storage_error(error));
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        tokio::fs::read(self.path(key)?)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => DomainError::NotFound,
                _ => storage_error(error),
            })
    }

    async fn list(&self) -> Result<Vec<StorageEntry>> {
        let mut directories = vec![self.root.clone()];
        let mut entries = Vec::new();
        while let Some(path) = directories.pop() {
            let mut directory = tokio::fs::read_dir(path).await.map_err(storage_error)?;
            while let Some(entry) = directory.next_entry().await.map_err(storage_error)? {
                let file_type = entry.file_type().await.map_err(storage_error)?;
                if file_type.is_dir() {
                    directories.push(entry.path());
                } else if file_type.is_file() {
                    let path = entry.path();
                    let key = path
                        .strip_prefix(&self.root)
                        .map_err(|error| DomainError::Storage(error.to_string()))?
                        .to_string_lossy()
                        .into_owned();
                    entries.push(StorageEntry {
                        key,
                        size: Some(entry.metadata().await.map_err(storage_error)?.len()),
                    });
                }
            }
        }
        entries.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(entries)
    }

    async fn health(&self) -> Result<()> {
        let probe = self.root.join(format!(".health-{}", Uuid::new_v4()));
        tokio::fs::write(&probe, b"ok")
            .await
            .map_err(storage_error)?;
        tokio::fs::read(&probe).await.map_err(storage_error)?;
        tokio::fs::remove_file(&probe).await.map_err(storage_error)
    }
}

#[derive(Debug)]
pub struct JsonPayslipRepository {
    index_path: PathBuf,
}

impl JsonPayslipRepository {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(data_dir)
            .await
            .map_err(storage_error)?;
        Ok(Self {
            index_path: data_dir.join("index.json"),
        })
    }
}

#[async_trait]
impl PayslipRepository for JsonPayslipRepository {
    async fn load(&self) -> Result<Vec<Payslip>> {
        match tokio::fs::read(&self.index_path).await {
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|error| DomainError::Storage(format!("index.json invalide: {error}"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(storage_error(error)),
        }
    }

    async fn save(&self, payslips: &[Payslip]) -> Result<()> {
        let data = serde_json::to_vec_pretty(payslips)
            .map_err(|error| DomainError::Storage(error.to_string()))?;
        let parent = self
            .index_path
            .parent()
            .ok_or_else(|| DomainError::Storage("chemin de l'index sans parent".to_owned()))?;
        let temporary = parent.join(format!(".index-{}.json", Uuid::new_v4()));
        tokio::fs::write(&temporary, data)
            .await
            .map_err(storage_error)?;
        if let Err(error) = tokio::fs::rename(&temporary, &self.index_path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(storage_error(error));
        }
        Ok(())
    }
}

fn storage_error(error: std::io::Error) -> DomainError {
    DomainError::Storage(error.to_string())
}
