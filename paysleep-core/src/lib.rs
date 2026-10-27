use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status", content = "message")]
pub enum ReplicationState {
    #[default]
    Disabled,
    Synced,
    Failed(String),
}

impl fmt::Display for ReplicationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("Désactivée"),
            Self::Synced => formatter.write_str("Synchronisée"),
            Self::Failed(_) => formatter.write_str("Échec"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Payslip {
    pub id: Uuid,
    pub company: String,
    pub year: i32,
    pub month: u8,
    pub original_name: String,
    pub stored_name: String,
    pub size: u64,
    pub added_at: DateTime<Utc>,
    pub proton_state: ReplicationState,
    #[serde(default)]
    pub google_state: ReplicationState,
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("l'entreprise est obligatoire")]
    EmptyCompany,
    #[error("le mois doit être compris entre 1 et 12")]
    InvalidMonth,
    #[error("l'année doit être comprise entre 2000 et {max}")]
    InvalidYear { max: i32 },
    #[error("le fichier doit avoir une extension .pdf")]
    InvalidExtension,
    #[error("le fichier ne possède pas la signature PDF attendue")]
    InvalidSignature,
    #[error("le fichier dépasse la limite de {max_mb} Mo")]
    TooLarge { max_mb: usize },
    #[error("un bulletin existe déjà pour cette entreprise et cette période")]
    Duplicate,
    #[error("stockage indisponible: {0}")]
    Storage(String),
    #[error("bulletin introuvable")]
    NotFound,
}

pub type Result<T> = std::result::Result<T, DomainError>;

pub fn validate_upload(
    company: &str,
    month: u8,
    year: i32,
    original_name: &str,
    data: &[u8],
    max_upload_mb: usize,
) -> Result<()> {
    if company.trim().is_empty() {
        return Err(DomainError::EmptyCompany);
    }
    if !(1..=12).contains(&month) {
        return Err(DomainError::InvalidMonth);
    }
    let max_year = Utc::now().year() + 1;
    if !(2000..=max_year).contains(&year) {
        return Err(DomainError::InvalidYear { max: max_year });
    }
    let is_pdf = std::path::Path::new(original_name)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Err(DomainError::InvalidExtension);
    }
    if !data.starts_with(b"%PDF-") {
        return Err(DomainError::InvalidSignature);
    }
    if data.len() > max_upload_mb.saturating_mul(1024 * 1024) {
        return Err(DomainError::TooLarge {
            max_mb: max_upload_mb,
        });
    }
    Ok(())
}

pub fn normalize_company(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn company_segment(value: &str) -> String {
    let normalized = normalize_company(value);
    let mut segment = String::new();
    let mut separator = false;
    for character in normalized.chars() {
        if character.is_ascii_alphanumeric() {
            segment.push(character);
            separator = false;
        } else if !separator && !segment.is_empty() {
            segment.push('-');
            separator = true;
        }
    }
    let result = segment.trim_matches('-');
    if result.is_empty() {
        "entreprise".to_owned()
    } else {
        result.to_owned()
    }
}

pub fn safe_download_name(value: &str) -> String {
    let file_name = std::path::Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bulletin.pdf");
    let cleaned: String = file_name
        .chars()
        .filter(|character| !character.is_control() && *character != '"' && *character != '\\')
        .collect();
    if cleaned.is_empty() {
        "bulletin.pdf".to_owned()
    } else {
        cleaned
    }
}

pub fn payslip_storage_key(year: i32, company: &str, month: u8, original_name: &str) -> String {
    format!(
        "{year}/{}/{month:02}/{}",
        company_segment(company),
        safe_download_name(original_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_business_fields_and_pdf() {
        assert!(validate_upload("ACME", 1, 2026, "fiche.PDF", b"%PDF-1.7", 1).is_ok());
        assert!(matches!(
            validate_upload("", 1, 2026, "fiche.pdf", b"%PDF-", 1),
            Err(DomainError::EmptyCompany)
        ));
        assert!(matches!(
            validate_upload("ACME", 13, 2026, "fiche.pdf", b"%PDF-", 1),
            Err(DomainError::InvalidMonth)
        ));
        assert!(matches!(
            validate_upload("ACME", 1, 1999, "fiche.pdf", b"%PDF-", 1),
            Err(DomainError::InvalidYear { .. })
        ));
        assert!(matches!(
            validate_upload("ACME", 1, 2026, "fiche.txt", b"%PDF-", 1),
            Err(DomainError::InvalidExtension)
        ));
        assert!(matches!(
            validate_upload("ACME", 1, 2026, "fiche.pdf", b"nope", 1),
            Err(DomainError::InvalidSignature)
        ));
    }

    #[test]
    fn normalizes_company_and_file_names() {
        assert_eq!(normalize_company("  ACME   France "), "acme france");
        assert_eq!(company_segment("Été & ACME"), "t-acme");
        assert_eq!(safe_download_name("../../fiche\".pdf"), "fiche.pdf");
        assert_eq!(
            payslip_storage_key(2026, "ACME France", 4, "../avril.pdf"),
            "2026/acme-france/04/avril.pdf"
        );
    }

    #[test]
    fn old_index_entries_default_google_replication_to_disabled() {
        let json = r#"{
            "id":"00000000-0000-0000-0000-000000000000",
            "company":"ACME",
            "year":2026,
            "month":4,
            "original_name":"avril.pdf",
            "stored_name":"stored.pdf",
            "size":9,
            "added_at":"2026-04-01T00:00:00Z",
            "proton_state":{"status":"disabled"}
        }"#;
        let payslip: Payslip = serde_json::from_str(json).unwrap();
        assert_eq!(payslip.google_state, ReplicationState::Disabled);
    }
}
