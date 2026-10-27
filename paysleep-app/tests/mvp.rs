use actix_web::{App, http::StatusCode, test, web};
use async_trait::async_trait;
use paysleep_application::{NewPayslip, PayslipService, Storage, StorageEntry};
use paysleep_core::{DomainError, ReplicationState, Result};
use paysleep_storage_filesystem::{FileSystemStorage, JsonPayslipRepository};
use paysleep_web::templates::Templates;
use paysleep_web::{AppState, Metrics, configure};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("paysleep-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn build_service(
    directory: &Path,
    proton: Option<Arc<dyn Storage>>,
    google: Option<Arc<dyn Storage>>,
) -> Arc<PayslipService> {
    let local: Arc<dyn Storage> = Arc::new(FileSystemStorage::new(directory).await.unwrap());
    let repository = Arc::new(JsonPayslipRepository::new(directory).await.unwrap());
    Arc::new(
        PayslipService::new(repository, local, proton, google, 2)
            .await
            .unwrap(),
    )
}

fn input() -> NewPayslip {
    NewPayslip {
        company: "ACME France".to_owned(),
        month: 4,
        year: 2026,
        original_name: "avril.pdf".to_owned(),
        data: b"%PDF-1.7\ntest".to_vec(),
    }
}

#[actix_web::test]
async fn filesystem_writes_reads_and_lists() {
    let directory = TestDir::new();
    let storage = FileSystemStorage::new(directory.path()).await.unwrap();
    storage.put("test.pdf", b"%PDF-test").await.unwrap();
    assert_eq!(storage.get("test.pdf").await.unwrap(), b"%PDF-test");
    assert_eq!(
        storage.list().await.unwrap(),
        vec![StorageEntry {
            key: "test.pdf".to_owned(),
            size: Some(9),
        }]
    );
}

#[actix_web::test]
async fn filesystem_supports_year_company_month_layout() {
    let directory = TestDir::new();
    let storage = FileSystemStorage::new(directory.path()).await.unwrap();
    let key = "2026/acme-france/04/avril.pdf";
    storage.put(key, b"%PDF-test").await.unwrap();

    assert_eq!(storage.get(key).await.unwrap(), b"%PDF-test");
    assert!(directory.path().join("files").join(key).is_file());
    assert_eq!(storage.list().await.unwrap()[0].key, key);
}

#[actix_web::test]
async fn rejects_duplicates_and_survives_restart() {
    let directory = TestDir::new();
    let service = build_service(directory.path(), None, None).await;
    let created = service.create(input()).await.unwrap();
    assert_eq!(created.stored_name, "2026/acme-france/04/avril.pdf");
    assert!(matches!(
        service.create(input()).await,
        Err(DomainError::Duplicate)
    ));
    drop(service);
    let restarted = build_service(directory.path(), None, None).await;
    assert_eq!(restarted.list().await.len(), 1);
    assert_eq!(
        restarted.download(created.id).await.unwrap().1,
        b"%PDF-1.7\ntest"
    );
}

struct FailingRemote;

#[async_trait]
impl Storage for FailingRemote {
    async fn put(&self, _key: &str, _data: &[u8]) -> Result<()> {
        Err(DomainError::Storage("hors ligne".to_owned()))
    }

    async fn get(&self, _key: &str) -> Result<Vec<u8>> {
        Err(DomainError::Storage("hors ligne".to_owned()))
    }

    async fn list(&self) -> Result<Vec<StorageEntry>> {
        Err(DomainError::Storage("hors ligne".to_owned()))
    }

    async fn health(&self) -> Result<()> {
        Err(DomainError::Storage("hors ligne".to_owned()))
    }
}

#[actix_web::test]
async fn proton_failure_keeps_local_copy() {
    let directory = TestDir::new();
    let proton: Arc<dyn Storage> = Arc::new(FailingRemote);
    let service = build_service(directory.path(), Some(proton), None).await;
    let created = service.create(input()).await.unwrap();
    assert!(matches!(created.proton_state, ReplicationState::Failed(_)));
    assert_eq!(
        service.download(created.id).await.unwrap().1,
        b"%PDF-1.7\ntest"
    );
}

#[actix_web::test]
async fn google_failure_keeps_local_copy() {
    let directory = TestDir::new();
    let google: Arc<dyn Storage> = Arc::new(FailingRemote);
    let service = build_service(directory.path(), None, Some(google)).await;
    let created = service.create(input()).await.unwrap();
    assert!(matches!(created.google_state, ReplicationState::Failed(_)));
    assert_eq!(
        service.download(created.id).await.unwrap().1,
        b"%PDF-1.7\ntest"
    );
}

#[actix_web::test]
async fn http_upload_then_download_and_duplicate_conflict() {
    let directory = TestDir::new();
    let service = build_service(directory.path(), None, None).await;
    let state = web::Data::new(AppState {
        service: service.clone(),
        templates: Templates::new().unwrap(),
        metrics: Metrics::default(),
        max_upload_mb: 2,
    });
    let app = test::init_service(App::new().app_data(state).configure(configure)).await;
    let boundary = "paysleep-boundary";
    let body = multipart_body(boundary);
    let request = test::TestRequest::post()
        .uri("/payslips")
        .insert_header((
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        ))
        .set_payload(body.clone())
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let payslip = service.list().await.pop().unwrap();
    let request = test::TestRequest::get()
        .uri(&format!("/payslips/{}/download", payslip.id))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(test::read_body(response).await, b"%PDF-1.7\nhttp"[..]);

    let request = test::TestRequest::post()
        .uri("/payslips")
        .insert_header((
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        ))
        .set_payload(body)
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::CONFLICT
    );
}

fn multipart_body(boundary: &str) -> Vec<u8> {
    format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"company\"\r\n\r\nACME France\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"month\"\r\n\r\n4\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"year\"\r\n\r\n2026\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"avril.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.7\nhttp\r\n\
         --{boundary}--\r\n"
    )
    .into_bytes()
}
