use async_trait::async_trait;
use chrono::Utc;
use paysleep_core::{
    DomainError, Payslip, ReplicationState, Result, normalize_company,
    payslip_storage_key, validate_upload,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageEntry {
    pub key: String,
    pub size: Option<u64>,
}

#[async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, data: &[u8]) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Vec<u8>>;
    async fn list(&self) -> Result<Vec<StorageEntry>>;
    async fn health(&self) -> Result<()>;
}

#[async_trait]
pub trait PayslipRepository: Send + Sync {
    async fn load(&self) -> Result<Vec<Payslip>>;
    async fn save(&self, payslips: &[Payslip]) -> Result<()>;
}

pub struct NewPayslip {
    pub company: String,
    pub month: u8,
    pub year: i32,
    pub original_name: String,
    pub data: Vec<u8>,
}

pub struct PayslipService {
    repository: Arc<dyn PayslipRepository>,
    local: Arc<dyn Storage>,
    proton: Option<Arc<dyn Storage>>,
    google: Option<Arc<dyn Storage>>,
    payslips: Mutex<Vec<Payslip>>,
    max_upload_mb: usize,
}

impl PayslipService {
    pub async fn new(
        repository: Arc<dyn PayslipRepository>,
        local: Arc<dyn Storage>,
        proton: Option<Arc<dyn Storage>>,
        google: Option<Arc<dyn Storage>>,
        max_upload_mb: usize,
    ) -> Result<Self> {
        let payslips = repository.load().await?;
        Ok(Self {
            repository,
            local,
            proton,
            google,
            payslips: Mutex::new(payslips),
            max_upload_mb,
        })
    }

    pub async fn create(&self, input: NewPayslip) -> Result<Payslip> {
        validate_upload(
            &input.company,
            input.month,
            input.year,
            &input.original_name,
            &input.data,
            self.max_upload_mb,
        )?;
        let company = input.company.trim().to_owned();
        let normalized = normalize_company(&company);
        let mut payslips = self.payslips.lock().await;
        if payslips.iter().any(|payslip| {
            normalize_company(&payslip.company) == normalized
                && payslip.year == input.year
                && payslip.month == input.month
        }) {
            return Err(DomainError::Duplicate);
        }
        let id = Uuid::new_v4();
        let stored_name =
            payslip_storage_key(input.year, &company, input.month, &input.original_name);
        self.local.put(&stored_name, &input.data).await?;
        let proton_state = replicate(&self.proton, &stored_name, &input.data, "Proton", id).await;
        let google_state = replicate(&self.google, &stored_name, &input.data, "Google", id).await;
        let payslip = Payslip {
            id,
            company,
            year: input.year,
            month: input.month,
            original_name: input.original_name,
            stored_name,
            size: input.data.len() as u64,
            added_at: Utc::now(),
            proton_state,
            google_state,
        };
        payslips.push(payslip.clone());
        if let Err(error) = self.repository.save(&payslips).await {
            payslips.pop();
            return Err(error);
        }
        Ok(payslip)
    }

    pub async fn list(&self) -> Vec<Payslip> {
        let mut payslips = self.payslips.lock().await.clone();
        payslips.sort_by(|left, right| {
            (right.year, right.month, right.added_at).cmp(&(left.year, left.month, left.added_at))
        });
        payslips
    }

    pub async fn download(&self, id: Uuid) -> Result<(Payslip, Vec<u8>)> {
        let payslip = self
            .payslips
            .lock()
            .await
            .iter()
            .find(|payslip| payslip.id == id)
            .cloned()
            .ok_or(DomainError::NotFound)?;
        let data = self.local.get(&payslip.stored_name).await?;
        Ok((payslip, data))
    }

    pub async fn ready(&self) -> Result<()> {
        self.local.health().await
    }
}

async fn replicate(
    storage: &Option<Arc<dyn Storage>>,
    key: &str,
    data: &[u8],
    backend: &'static str,
    payslip_id: Uuid,
) -> ReplicationState {
    let Some(storage) = storage else {
        return ReplicationState::Disabled;
    };
    match storage.put(key, data).await {
        Ok(()) => ReplicationState::Synced,
        Err(error) => {
            tracing::warn!(%payslip_id, backend, "remote replication failed");
            ReplicationState::Failed(short_error(&error.to_string()))
        }
    }
}

fn short_error(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect()
}
