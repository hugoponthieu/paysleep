pub mod templates;

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Responder, web};
use futures_util::StreamExt;
use paysleep_application::{NewPayslip, PayslipService};
use paysleep_core::{DomainError, safe_download_name};
use templates::Templates;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

pub struct AppState {
    pub service: Arc<PayslipService>,
    pub templates: Templates,
    pub metrics: Metrics,
    pub max_upload_mb: usize,
}

#[derive(Default)]
pub struct Metrics {
    requests: AtomicU64,
    uploads_ok: AtomicU64,
    uploads_failed: AtomicU64,
}

pub fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/", web::get().to(index))
        .route("/payslips", web::post().to(upload))
        .route("/payslips/{id}/download", web::get().to(download))
        .route("/health/live", web::get().to(live))
        .route("/health/ready", web::get().to(ready))
        .route("/metrics", web::get().to(metrics));
}

async fn index(state: web::Data<AppState>) -> impl Responder {
    state.metrics.request();
    render_response(StatusCode::OK, &state, &state.service.list().await, None)
}

async fn upload(
    request: HttpRequest,
    mut payload: web::Payload,
    state: web::Data<AppState>,
) -> impl Responder {
    state.metrics.request();
    let result = async {
        let content_type = request
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| bad_request("contenu multipart manquant"))?;
        let boundary = multipart_boundary(content_type)
            .ok_or_else(|| bad_request("limite multipart manquante"))?;
        let body_limit = state.max_upload_mb.saturating_mul(1024 * 1024) + 64 * 1024;
        let mut body = Vec::new();
        while let Some(chunk) = payload.next().await {
            let chunk = chunk.map_err(|_| bad_request("envoi interrompu"))?;
            if body.len().saturating_add(chunk.len()) > body_limit {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!(
                        "Le fichier dépasse la limite de {} Mo.",
                        state.max_upload_mb
                    ),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let fields = parse_multipart(&body, boundary.as_bytes())?;
        let company = text_field(&fields, "company")?;
        let month = text_field(&fields, "month")?
            .parse::<u8>()
            .map_err(|_| bad_request("mois invalide"))?;
        let year = text_field(&fields, "year")?
            .parse::<i32>()
            .map_err(|_| bad_request("année invalide"))?;
        let file = fields
            .iter()
            .find(|field| field.name == "file")
            .ok_or_else(|| bad_request("fichier PDF obligatoire"))?;
        let original_name = file
            .file_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| bad_request("nom de fichier manquant"))?;
        state
            .service
            .create(NewPayslip {
                company,
                month,
                year,
                original_name,
                data: file.data.clone(),
            })
            .await
            .map_err(domain_response)
    }
    .await;
    match result {
        Ok(_) => {
            state.metrics.uploads_ok.fetch_add(1, Ordering::Relaxed);
            render_response(
                StatusCode::CREATED,
                &state,
                &state.service.list().await,
                Some(Message::success("Bulletin archivé.")),
            )
        }
        Err((status, message)) => {
            state.metrics.uploads_failed.fetch_add(1, Ordering::Relaxed);
            render_response(
                status,
                &state,
                &state.service.list().await,
                Some(Message::error(message)),
            )
        }
    }
}

async fn download(path: web::Path<Uuid>, state: web::Data<AppState>) -> impl Responder {
    state.metrics.request();
    match state.service.download(path.into_inner()).await {
        Ok((payslip, data)) => {
            let safe_name = safe_download_name(&payslip.original_name);
            let ascii_name: String = safe_name
                .chars()
                .filter(|character| {
                    character.is_ascii_alphanumeric() || ".-_ ".contains(*character)
                })
                .collect();
            let fallback = if ascii_name.is_empty() {
                "bulletin.pdf"
            } else {
                &ascii_name
            };
            let disposition = format!(
                "attachment; filename=\"{fallback}\"; filename*=UTF-8''{}",
                percent_encode(&safe_name)
            );
            HttpResponse::Ok()
                .insert_header(("content-type", "application/pdf"))
                .insert_header(("content-disposition", disposition))
                .body(data)
        }
        Err(DomainError::NotFound) => HttpResponse::NotFound().body("Bulletin introuvable"),
        Err(_) => HttpResponse::InternalServerError().body("Téléchargement indisponible"),
    }
}

async fn live(state: web::Data<AppState>) -> impl Responder {
    state.metrics.request();
    HttpResponse::Ok().body("ok")
}

async fn ready(state: web::Data<AppState>) -> impl Responder {
    state.metrics.request();
    match state.service.ready().await {
        Ok(()) => HttpResponse::Ok().body("ready"),
        Err(_) => HttpResponse::ServiceUnavailable().body("not ready"),
    }
}

async fn metrics(state: web::Data<AppState>) -> impl Responder {
    state.metrics.request();
    HttpResponse::Ok()
        .insert_header(("content-type", "text/plain; version=0.0.4; charset=utf-8"))
        .body(state.metrics.render())
}

impl Metrics {
    fn request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    fn render(&self) -> String {
        format!(
            "# HELP paysleep_http_requests_total HTTP requests received.\n\
             # TYPE paysleep_http_requests_total counter\n\
             paysleep_http_requests_total {}\n\
             # HELP paysleep_uploads_total Payslip upload attempts.\n\
             # TYPE paysleep_uploads_total counter\n\
             paysleep_uploads_total{{result=\"success\"}} {}\n\
             paysleep_uploads_total{{result=\"failure\"}} {}\n",
            self.requests.load(Ordering::Relaxed),
            self.uploads_ok.load(Ordering::Relaxed),
            self.uploads_failed.load(Ordering::Relaxed)
        )
    }
}

struct MultipartField {
    name: String,
    file_name: Option<String>,
    data: Vec<u8>,
}

fn multipart_boundary(content_type: &str) -> Option<String> {
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return None;
    }
    content_type.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        name.eq_ignore_ascii_case("boundary")
            .then(|| value.trim_matches('"').to_owned())
    })
}

fn parse_multipart(
    body: &[u8],
    boundary: &[u8],
) -> std::result::Result<Vec<MultipartField>, (StatusCode, String)> {
    if boundary.is_empty() || boundary.len() > 200 {
        return Err(bad_request("limite multipart invalide"));
    }
    let mut delimiter = b"--".to_vec();
    delimiter.extend_from_slice(boundary);
    let mut fields = Vec::new();
    for raw_part in split_bytes(body, &delimiter).into_iter().skip(1) {
        let part = raw_part.strip_prefix(b"\r\n").unwrap_or(raw_part);
        if part.starts_with(b"--") {
            break;
        }
        let part = part.strip_suffix(b"\r\n").unwrap_or(part);
        let header_end =
            find_bytes(part, b"\r\n\r\n").ok_or_else(|| bad_request("champ multipart invalide"))?;
        let headers = std::str::from_utf8(&part[..header_end])
            .map_err(|_| bad_request("en-têtes multipart invalides"))?;
        let disposition = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .ok_or_else(|| bad_request("description de champ manquante"))?;
        let name = disposition_parameter(disposition, "name")
            .ok_or_else(|| bad_request("nom de champ manquant"))?;
        let file_name = disposition_parameter(disposition, "filename");
        fields.push(MultipartField {
            name,
            file_name,
            data: part[header_end + 4..].to_vec(),
        });
    }
    Ok(fields)
}

fn disposition_parameter(value: &str, key: &str) -> Option<String> {
    value.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name.eq_ignore_ascii_case(key)).then(|| value.trim_matches('"').to_owned())
    })
}

fn text_field(
    fields: &[MultipartField],
    name: &str,
) -> std::result::Result<String, (StatusCode, String)> {
    let field = fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| bad_request(&format!("champ {name} obligatoire")))?;
    std::str::from_utf8(&field.data)
        .map(str::to_owned)
        .map_err(|_| bad_request(&format!("champ {name} invalide")))
}

fn split_bytes<'a>(value: &'a [u8], separator: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(offset) = find_bytes(&value[start..], separator) {
        parts.push(&value[start..start + offset]);
        start += offset + separator.len();
    }
    parts.push(&value[start..]);
    parts
}

fn find_bytes(value: &[u8], needle: &[u8]) -> Option<usize> {
    value
        .windows(needle.len())
        .position(|window| window == needle)
}

fn bad_request(message: &str) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.to_owned())
}

fn domain_response(error: DomainError) -> (StatusCode, String) {
    let status = match error {
        DomainError::Duplicate => StatusCode::CONFLICT,
        DomainError::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        DomainError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        DomainError::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    };
    let message = if matches!(error, DomainError::Storage(_)) {
        "Le stockage local est momentanément indisponible.".to_owned()
    } else {
        error.to_string()
    };
    (status, message)
}

fn page_response(status: StatusCode, body: String) -> HttpResponse {
    HttpResponse::build(status)
        .insert_header(("content-type", "text/html; charset=utf-8"))
        .body(body)
}

fn render_response(
    status: StatusCode,
    state: &AppState,
    payslips: &[paysleep_core::Payslip],
    message: Option<Message>,
) -> HttpResponse {
    let message = message
        .as_ref()
        .map(|message| (message.class, message.text.as_str()));
    match state
        .templates
        .render_index(payslips, message, state.max_upload_mb)
    {
        Ok(body) => page_response(status, body),
        Err(error) => {
            tracing::error!(error = %error, "HTML template rendering failed");
            HttpResponse::InternalServerError().body("Rendu de la page indisponible")
        }
    }
}

struct Message {
    class: &'static str,
    text: String,
}

impl Message {
    fn success(text: &str) -> Self {
        Self {
            class: "success",
            text: text.to_owned(),
        }
    }

    fn error(text: String) -> Self {
        Self {
            class: "error",
            text,
        }
    }
}

fn percent_encode(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                char::from(*byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binary_multipart() {
        let body = b"--x\r\nContent-Disposition: form-data; name=\"company\"\r\n\r\nACME\r\n--x\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-\0data\r\n--x--\r\n";
        let fields = parse_multipart(body, b"x").unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[1].data, b"%PDF-\0data");
    }
}
