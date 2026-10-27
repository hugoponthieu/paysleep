mod config;

use actix_web::{App, HttpServer, web};
use config::Config;
use paysleep_application::{PayslipRepository, PayslipService, Storage};
use paysleep_storage_filesystem::{FileSystemStorage, JsonPayslipRepository};
use paysleep_storage_google_drive::GoogleDriveStorage;
use paysleep_storage_proton_drive::ProtonDriveStorage;
use paysleep_web::templates::Templates;
use paysleep_web::{AppState, Metrics, configure};
use std::io;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[actix_web::main]
async fn main() -> io::Result<()> {
    let config = Config::parse_validated();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&config.rust_log).map_err(io::Error::other)?)
        .init();
    let local: Arc<dyn Storage> = Arc::new(
        FileSystemStorage::new(&config.data_dir)
            .await
            .map_err(io::Error::other)?,
    );
    let proton: Option<Arc<dyn Storage>> = if config.proton_enabled {
        let storage = ProtonDriveStorage::new(
            config.proton_bin.clone(),
            config.proton_path.clone(),
            &config.data_dir,
            config
                .proton_state_dir
                .clone()
                .expect("validated Proton state directory"),
            config.proton_credentials_store.into(),
        )
        .map_err(io::Error::other)?;
        storage.health().await.map_err(io::Error::other)?;
        Some(Arc::new(storage))
    } else {
        None
    };
    let google: Option<Arc<dyn Storage>> = if config.google_enabled {
        let client_secret_file = config
            .google_client_secret_file
            .as_deref()
            .expect("validated Google client secret path");
        let token_cache_file = config
            .google_token_cache_file
            .as_deref()
            .expect("validated Google token cache path");
        let storage = GoogleDriveStorage::new(
            client_secret_file,
            token_cache_file,
            config.google_folder_id.clone(),
        )
        .await
        .map_err(io::Error::other)?;
        storage.health().await.map_err(io::Error::other)?;
        Some(Arc::new(storage))
    } else {
        None
    };
    let repository: Arc<dyn PayslipRepository> = Arc::new(
        JsonPayslipRepository::new(&config.data_dir)
            .await
            .map_err(io::Error::other)?,
    );
    let service = Arc::new(
        PayslipService::new(
            repository,
            local,
            proton,
            google,
            config.max_upload_mb,
        )
        .await
        .map_err(io::Error::other)?,
    );
    let state = web::Data::new(AppState {
        service,
        templates: Templates::new().map_err(io::Error::other)?,
        metrics: Metrics::default(),
        max_upload_mb: config.max_upload_mb,
    });
    tracing::info!(bind = %config.bind, "Paysleep started");
    HttpServer::new(move || App::new().app_data(state.clone()).configure(configure))
        .bind(config.bind)?
        .run()
        .await
}
