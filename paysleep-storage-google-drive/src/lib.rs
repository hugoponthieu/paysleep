use async_trait::async_trait;
use paysleep_application::{Storage, StorageEntry};
use paysleep_core::{DomainError, Result};
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;
use yup_oauth2::{ApplicationSecret, InstalledFlowAuthenticator, InstalledFlowReturnMethod};

const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";
const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FOLDER_MIME_TYPE: &str = "application/vnd.google-apps.folder";

#[derive(Clone)]
pub struct GoogleDriveStorage {
    client: Client,
    client_secret: ApplicationSecret,
    token_cache_file: PathBuf,
    folder_id: String,
}

impl GoogleDriveStorage {
    pub async fn new(
        client_secret_file: &Path,
        token_cache_file: &Path,
        folder_id: Option<String>,
    ) -> Result<Self> {
        let client_secret = yup_oauth2::read_application_secret(client_secret_file)
            .await
            .map_err(storage_error)?;
        if let Some(parent) = token_cache_file.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(storage_error)?;
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(storage_error)?;
        let mut storage = Self {
            client,
            client_secret,
            token_cache_file: token_cache_file.to_owned(),
            folder_id: folder_id.unwrap_or_default(),
        };
        if storage.folder_id.is_empty() {
            storage.folder_id = storage.find_or_create_folder("Paysleep").await?;
        }
        Ok(storage)
    }

    async fn access_token(&self) -> Result<String> {
        let authenticator = InstalledFlowAuthenticator::builder(
            self.client_secret.clone(),
            InstalledFlowReturnMethod::HTTPRedirect,
        )
        .persist_tokens_to_disk(&self.token_cache_file)
        .build()
        .await
        .map_err(storage_error)?;
        authenticator
            .token(&[DRIVE_SCOPE])
            .await
            .map_err(storage_error)?
            .token()
            .map(str::to_owned)
            .ok_or_else(|| DomainError::Storage("Google n'a retourné aucun jeton".to_owned()))
    }

    async fn find_or_create_folder(&self, name: &str) -> Result<String> {
        if let Some(folder) = self
            .find_child("root", name, Some(FOLDER_MIME_TYPE))
            .await?
        {
            return Ok(folder.id);
        }
        self.create_folder("root", name).await
    }

    async fn create_folder(&self, parent_id: &str, name: &str) -> Result<String> {
        let token = self.access_token().await?;

        let response = checked(
            self.client
                .post(FILES_URL)
                .bearer_auth(token)
                .query(&[("fields", "id,name")])
                .json(&serde_json::json!({
                    "name": name,
                    "mimeType": FOLDER_MIME_TYPE,
                    "parents": [parent_id]
                }))
                .send()
                .await
                .map_err(storage_error)?,
        )
        .await?;
        response
            .json::<GoogleFile>()
            .await
            .map(|folder| folder.id)
            .map_err(storage_error)
    }

    async fn list_children(&self, parent_id: &str) -> Result<Vec<GoogleFile>> {
        let token = self.access_token().await?;
        let query = format!(
            "'{}' in parents and trashed = false",
            escape_query_value(parent_id)
        );
        let mut page_token: Option<String> = None;
        let mut files = Vec::new();
        loop {
            let mut request = self.client.get(FILES_URL).bearer_auth(&token).query(&[
                ("q", query.as_str()),
                ("fields", "nextPageToken,files(id,name,size,mimeType)"),
                ("pageSize", "1000"),
                ("spaces", "drive"),
                ("supportsAllDrives", "true"),
                ("includeItemsFromAllDrives", "true"),
            ]);
            if let Some(value) = &page_token {
                request = request.query(&[("pageToken", value)]);
            }
            let response = checked(request.send().await.map_err(storage_error)?).await?;
            let page = response.json::<FileList>().await.map_err(storage_error)?;
            files.extend(page.files);
            let Some(next_page_token) = page.next_page_token else {
                break;
            };
            page_token = Some(next_page_token);
        }
        Ok(files)
    }

    async fn find_child(
        &self,
        parent_id: &str,
        name: &str,
        mime_type: Option<&str>,
    ) -> Result<Option<GoogleFile>> {
        Ok(self
            .list_children(parent_id)
            .await?
            .into_iter()
            .find(|file| {
                file.name == name
                    && mime_type.is_none_or(|expected| file.mime_type.as_deref() == Some(expected))
            }))
    }

    async fn key_parent<'a>(&self, key: &'a str, create: bool) -> Result<(String, &'a str)> {
        let parts = key_parts(key)?;
        let (file_name, folders) = parts
            .split_last()
            .ok_or_else(|| DomainError::Storage("clé Google vide".to_owned()))?;
        let mut parent_id = self.folder_id.clone();
        for folder_name in folders {
            parent_id = match self
                .find_child(&parent_id, folder_name, Some(FOLDER_MIME_TYPE))
                .await?
            {
                Some(folder) => folder.id,
                None if create => self.create_folder(&parent_id, folder_name).await?,
                None => return Err(DomainError::NotFound),
            };
        }
        Ok((parent_id, file_name))
    }

    async fn file_id(&self, key: &str) -> Result<String> {
        let (parent_id, file_name) = self.key_parent(key, false).await?;
        self.list_children(&parent_id)
            .await?
            .into_iter()
            .find(|file| {
                file.name == file_name && file.mime_type.as_deref() != Some(FOLDER_MIME_TYPE)
            })
            .map(|file| file.id)
            .ok_or(DomainError::NotFound)
    }
}

#[async_trait]
impl Storage for GoogleDriveStorage {
    async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let (parent_id, file_name) = self.key_parent(key, true).await?;
        let token = self.access_token().await?;
        let boundary = format!("paysleep-{}", Uuid::new_v4());
        let metadata = serde_json::json!({
            "name": file_name,
            "parents": [parent_id],
        });
        let mut body = Vec::with_capacity(data.len() + 512);
        body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/pdf\r\n\r\n").as_bytes());
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        checked(
            self.client
                .post(UPLOAD_URL)
                .bearer_auth(token)
                .query(&[("uploadType", "multipart"), ("supportsAllDrives", "true")])
                .header(
                    "content-type",
                    format!("multipart/related; boundary={boundary}"),
                )
                .body(body)
                .send()
                .await
                .map_err(storage_error)?,
        )
        .await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let token = self.access_token().await?;
        let id = self.file_id(key).await?;
        let response = checked(
            self.client
                .get(format!("{FILES_URL}/{id}"))
                .bearer_auth(token)
                .query(&[("alt", "media"), ("supportsAllDrives", "true")])
                .send()
                .await
                .map_err(storage_error)?,
        )
        .await?;
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(storage_error)
    }

    async fn list(&self) -> Result<Vec<StorageEntry>> {
        let mut pending = vec![(self.folder_id.clone(), String::new())];
        let mut entries = Vec::new();
        while let Some((folder_id, prefix)) = pending.pop() {
            for file in self.list_children(&folder_id).await? {
                let key = if prefix.is_empty() {
                    file.name.clone()
                } else {
                    format!("{prefix}/{}", file.name)
                };
                if file.mime_type.as_deref() == Some(FOLDER_MIME_TYPE) {
                    pending.push((file.id, key));
                } else {
                    entries.push(StorageEntry {
                        key,
                        size: file.size.and_then(|size| size.parse().ok()),
                    });
                }
            }
        }
        entries.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(entries)
    }

    async fn health(&self) -> Result<()> {
        let token = self.access_token().await?;
        let response = checked(
            self.client
                .get(format!("{FILES_URL}/{}", self.folder_id))
                .bearer_auth(token)
                .query(&[
                    ("fields", "id,mimeType,trashed"),
                    ("supportsAllDrives", "true"),
                ])
                .send()
                .await
                .map_err(storage_error)?,
        )
        .await?;
        let folder = response
            .json::<FolderMetadata>()
            .await
            .map_err(storage_error)?;
        if folder.trashed || folder.mime_type != "application/vnd.google-apps.folder" {
            return Err(DomainError::Storage(
                "l'identifiant Google ne désigne pas un dossier accessible".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    #[serde(default)]
    files: Vec<GoogleFile>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct GoogleFile {
    id: String,
    name: String,
    size: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderMetadata {
    mime_type: String,
    #[serde(default)]
    trashed: bool,
}

async fn checked(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let message = response.text().await.unwrap_or_default();
    let message = message
        .chars()
        .filter(|character| !character.is_control())
        .take(200)
        .collect::<String>();
    let description = if status == StatusCode::UNAUTHORIZED {
        "authentification Google refusée".to_owned()
    } else if message.is_empty() {
        format!("Google Drive a répondu HTTP {status}")
    } else {
        format!("Google Drive a répondu HTTP {status}: {message}")
    };
    Err(DomainError::Storage(description))
}

fn escape_query_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn key_parts(key: &str) -> Result<Vec<&str>> {
    let parts = key.split('/').collect::<Vec<_>>();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains('\\'))
    {
        return Err(DomainError::Storage("clé Google invalide".to_owned()));
    }
    Ok(parts)
}

fn storage_error(error: impl std::fmt::Display) -> DomainError {
    DomainError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_drive_query_values() {
        assert_eq!(escape_query_value("a'b\\c"), "a\\'b\\\\c");
    }

    #[test]
    fn splits_hierarchical_storage_keys() {
        assert_eq!(
            key_parts("2026/acme/04/avril.pdf").unwrap(),
            ["2026", "acme", "04", "avril.pdf"]
        );
        assert!(key_parts("2026/../secret.pdf").is_err());
        assert!(key_parts("/absolute.pdf").is_err());
    }
}
