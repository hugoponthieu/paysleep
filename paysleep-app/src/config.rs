use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};
use std::env;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "paysleep-app",
    version,
    about = "Archive privée de bulletins de paie"
)]
pub struct Config {
    #[arg(long, env = "PAYSLEEP_BIND", default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,

    #[arg(long, env = "PAYSLEEP_DATA_DIR", default_value = "./data")]
    pub data_dir: PathBuf,

    #[arg(
        long,
        env = "PAYSLEEP_MAX_UPLOAD_MB",
        default_value_t = 20,
        value_parser = parse_positive_usize
    )]
    pub max_upload_mb: usize,

    #[arg(long, env = "PAYSLEEP_PROTON_ENABLED", default_value_t = false)]
    pub proton_enabled: bool,

    #[arg(long, env = "PAYSLEEP_PROTON_BIN", default_value = "proton-drive")]
    pub proton_bin: PathBuf,

    #[arg(
        long,
        env = "PAYSLEEP_PROTON_PATH",
        default_value = "/my-files/Paysleep"
    )]
    pub proton_path: String,

    #[arg(long, env = "PAYSLEEP_PROTON_STATE_DIR")]
    pub proton_state_dir: Option<PathBuf>,

    #[arg(
        long,
        env = "PAYSLEEP_PROTON_CREDENTIALS_STORE",
        default_value = "keychain"
    )]
    pub proton_credentials_store: ProtonCredentialsStore,

    #[arg(long, env = "PAYSLEEP_GOOGLE_ENABLED", default_value_t = false)]
    pub google_enabled: bool,

    #[arg(long, env = "PAYSLEEP_GOOGLE_CLIENT_SECRET_FILE")]
    pub google_client_secret_file: Option<PathBuf>,

    #[arg(long, env = "PAYSLEEP_GOOGLE_TOKEN_CACHE_FILE")]
    pub google_token_cache_file: Option<PathBuf>,

    #[arg(long, env = "PAYSLEEP_GOOGLE_FOLDER_ID")]
    pub google_folder_id: Option<String>,

    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub rust_log: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ProtonCredentialsStore {
    Keychain,
    Pass,
    #[value(name = "unsafe_file")]
    UnsafeFile,
}

impl From<ProtonCredentialsStore> for paysleep_storage_proton_drive::CredentialsStore {
    fn from(value: ProtonCredentialsStore) -> Self {
        match value {
            ProtonCredentialsStore::Keychain => Self::Keychain,
            ProtonCredentialsStore::Pass => Self::Pass,
            ProtonCredentialsStore::UnsafeFile => Self::UnsafeFile,
        }
    }
}

impl Config {
    pub fn parse_validated() -> Self {
        Self::try_parse_validated_from(env::args_os()).unwrap_or_else(|error| error.exit())
    }

    pub fn try_parse_validated_from<I, T>(arguments: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        Self::try_parse_from(arguments)?.validate()
    }

    fn validate(mut self) -> Result<Self, clap::Error> {
        if self.proton_enabled {
            self.proton_bin = resolve_binary(&self.proton_bin).ok_or_else(|| {
                configuration_error(format!(
                    "PAYSLEEP_PROTON_BIN est introuvable: {}",
                    self.proton_bin.display()
                ))
            })?;
            if !self.proton_path.starts_with('/') || self.proton_path.trim() == "/" {
                return Err(configuration_error(
                    "PAYSLEEP_PROTON_PATH doit être un dossier absolu non racine",
                ));
            }
            if self.proton_state_dir.is_none() {
                self.proton_state_dir = Some(self.data_dir.join("proton-drive"));
            }
        }
        if self.google_enabled {
            let client_secret = self.google_client_secret_file.as_ref().ok_or_else(|| {
                configuration_error(
                    "PAYSLEEP_GOOGLE_CLIENT_SECRET_FILE est obligatoire quand Google est activé",
                )
            })?;
            if !client_secret.is_file() {
                return Err(configuration_error(format!(
                    "PAYSLEEP_GOOGLE_CLIENT_SECRET_FILE est introuvable: {}",
                    client_secret.display()
                )));
            }
            validate_desktop_client_secret(client_secret)?;
            if self.google_token_cache_file.is_none() {
                self.google_token_cache_file = Some(self.data_dir.join("google-oauth-token.json"));
            }
            if self.google_folder_id.as_deref() == Some("") {
                self.google_folder_id = None;
            } else if self
                .google_folder_id
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(configuration_error(
                    "PAYSLEEP_GOOGLE_FOLDER_ID ne doit pas contenir uniquement des espaces",
                ));
            }
        }
        Ok(self)
    }
}

fn validate_desktop_client_secret(path: &std::path::Path) -> Result<(), clap::Error> {
    let data = std::fs::read(path).map_err(|error| {
        configuration_error(format!(
            "impossible de lire PAYSLEEP_GOOGLE_CLIENT_SECRET_FILE: {error}"
        ))
    })?;
    let document: serde_json::Value = serde_json::from_slice(&data).map_err(|error| {
        configuration_error(format!("client secret Google JSON invalide: {error}"))
    })?;
    if document.get("installed").is_none() {
        return Err(configuration_error(
            "le client secret Google doit être de type « Application de bureau » (clé JSON `installed`), pas « Application Web »",
        ));
    }
    Ok(())
}

fn configuration_error(message: impl Into<String>) -> clap::Error {
    Config::command().error(ErrorKind::ValueValidation, message.into())
}

fn parse_positive_usize(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|error| format!("valeur entière invalide: {error}"))?;
    if parsed == 0 {
        Err("la valeur doit être supérieure à zéro".to_owned())
    } else {
        Ok(parsed)
    }
}

fn resolve_binary(value: &std::path::Path) -> Option<PathBuf> {
    if value.components().count() > 1 {
        return value.is_file().then(|| value.to_owned());
    }
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(value))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_configuration() {
        let config = Config::try_parse_validated_from([
            "paysleep-app",
            "--bind",
            "127.0.0.1:9000",
            "--data-dir",
            "/tmp/paysleep",
            "--max-upload-mb",
            "5",
            "--rust-log",
            "warn",
        ])
        .unwrap();
        assert_eq!(config.bind, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(config.data_dir, PathBuf::from("/tmp/paysleep"));
        assert_eq!(config.max_upload_mb, 5);
        assert_eq!(config.rust_log, "warn");
    }

    #[test]
    fn rejects_zero_upload_limit() {
        let result = Config::try_parse_validated_from(["paysleep-app", "--max-upload-mb", "0"]);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_proton_path() {
        let executable = std::env::current_exe().unwrap();
        let result = Config::try_parse_validated_from([
            OsString::from("paysleep-app"),
            OsString::from("--proton-enabled"),
            OsString::from("--proton-bin"),
            executable.into_os_string(),
            OsString::from("--proton-path"),
            OsString::from("/"),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn defaults_proton_state_to_the_data_directory() {
        let executable = std::env::current_exe().unwrap();
        let config = Config::try_parse_validated_from([
            OsString::from("paysleep-app"),
            OsString::from("--data-dir"),
            OsString::from("/tmp/paysleep"),
            OsString::from("--proton-enabled"),
            OsString::from("--proton-bin"),
            executable.into_os_string(),
        ])
        .unwrap();
        assert_eq!(
            config.proton_state_dir,
            Some(PathBuf::from("/tmp/paysleep/proton-drive"))
        );
        assert_eq!(
            config.proton_credentials_store,
            ProtonCredentialsStore::Keychain
        );
    }

    #[test]
    fn rejects_google_without_client_secret() {
        let result = Config::try_parse_validated_from(["paysleep-app", "--google-enabled"]);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_web_oauth_client() {
        let path =
            std::env::temp_dir().join(format!("paysleep-web-oauth-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, r#"{"web":{"client_id":"redacted"}}"#).unwrap();
        let result = Config::try_parse_validated_from([
            OsString::from("paysleep-app"),
            OsString::from("--google-enabled"),
            OsString::from("--google-client-secret-file"),
            path.clone().into_os_string(),
        ]);
        let _ = std::fs::remove_file(path);
        assert!(result.is_err());
    }
}
