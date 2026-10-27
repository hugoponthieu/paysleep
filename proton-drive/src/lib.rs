//! Rust bindings for the file-management surface of the official Proton Drive CLI.
//!
//! This crate deliberately delegates authentication, end-to-end encryption and
//! Proton API compatibility to Proton's CLI. Arguments are passed directly to
//! `execve` (never through a shell), and JSON output is retained losslessly.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

mod json;
pub use json::{JsonError, JsonValue};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    BinaryNotFound(PathBuf),
    Spawn {
        path: PathBuf,
        source: std::io::Error,
    },
    BareBunRuntime,
    Incompatible(String),
    CommandFailed {
        status: ExitStatus,
        stderr: String,
    },
    NonUtf8,
    InvalidJson {
        source: JsonError,
        output: String,
    },
    MissingSource,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BinaryNotFound(path) => write!(
                f,
                "Proton Drive CLI path does not point to a file: {}",
                path.display()
            ),
            Self::Spawn { path, source } => write!(
                f,
                "failed to launch Proton Drive CLI at {}: {source}",
                path.display()
            ),
            Self::BareBunRuntime => f.write_str(
                "the selected executable is Bun, not a working Proton Drive standalone CLI",
            ),
            Self::Incompatible(message) => write!(f, "incompatible Proton Drive CLI: {message}"),
            Self::CommandFailed { status, stderr } => {
                write!(f, "Proton Drive CLI failed with {status}: {stderr}")
            }
            Self::NonUtf8 => f.write_str("Proton Drive CLI returned non-UTF-8 output"),
            Self::InvalidJson { source, output } => write!(
                f,
                "Proton Drive CLI returned invalid JSON: {source}; output: {output}"
            ),
            Self::MissingSource => f.write_str("at least one source path is required"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::InvalidJson { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Lossless representation of a Drive node or command result.
///
/// The SDK's JSON schema evolves quickly. Keeping the complete value avoids
/// silently discarding fields while the convenience accessors cover stable,
/// commonly needed properties.
#[derive(Clone, Debug, PartialEq)]
pub struct DriveObject(pub JsonValue);

impl DriveObject {
    pub fn as_json(&self) -> &JsonValue {
        &self.0
    }

    pub fn into_json(self) -> JsonValue {
        self.0
    }

    pub fn uid(&self) -> Option<&str> {
        string_at(&self.0, &["uid"])
    }

    pub fn node_type(&self) -> Option<&str> {
        string_at(&self.0, &["type"])
    }

    pub fn name(&self) -> Option<&str> {
        string_at(&self.0, &["name"])
            .or_else(|| string_at(&self.0, &["name", "value"]))
            .or_else(|| string_at(&self.0, &["name", "ok", "value"]))
    }

    pub fn claimed_size(&self) -> Option<u64> {
        self.0
            .get("claimedSize")
            .and_then(JsonValue::as_u64)
            .or_else(|| self.0.get("activeRevision")?.get("claimedSize")?.as_u64())
    }
}

fn string_at<'a>(value: &'a JsonValue, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_str()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeType {
    File,
    Folder,
}

impl NodeType {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadFileConflict {
    CreateNewRevision,
    Rename,
    Replace,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadFolderConflict {
    Merge,
    Rename,
    Replace,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadFileConflict {
    Rename,
    Remove,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadFolderConflict {
    Merge,
    Rename,
    Remove,
    Skip,
}

macro_rules! conflict_str {
    ($value:expr) => {
        match $value {
            UploadFileConflict::CreateNewRevision => "create-new-revision",
            UploadFileConflict::Rename => "rename",
            UploadFileConflict::Replace => "replace",
            UploadFileConflict::Skip => "skip",
        }
    };
    (upload_folder $value:expr) => {
        match $value {
            UploadFolderConflict::Merge => "merge",
            UploadFolderConflict::Rename => "rename",
            UploadFolderConflict::Replace => "replace",
            UploadFolderConflict::Skip => "skip",
        }
    };
    (download_file $value:expr) => {
        match $value {
            DownloadFileConflict::Rename => "rename",
            DownloadFileConflict::Remove => "remove",
            DownloadFileConflict::Skip => "skip",
        }
    };
    (download_folder $value:expr) => {
        match $value {
            DownloadFolderConflict::Merge => "merge",
            DownloadFolderConflict::Rename => "rename",
            DownloadFolderConflict::Remove => "remove",
            DownloadFolderConflict::Skip => "skip",
        }
    };
}

#[derive(Clone, Debug, Default)]
pub struct UploadOptions {
    pub file_conflict: Option<UploadFileConflict>,
    pub folder_conflict: Option<UploadFolderConflict>,
    pub skip_thumbnails: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DownloadOptions {
    pub file_conflict: Option<DownloadFileConflict>,
    pub folder_conflict: Option<DownloadFolderConflict>,
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub version_output: String,
}

/// Store used by the Proton Drive CLI for its authenticated session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialsStore {
    /// Operating-system secret store (GNOME Keyring, KWallet, Keychain, etc.).
    Keychain,
    /// The Unix `pass` password store.
    Pass,
    /// Plaintext file supported by the CLI for tests only.
    UnsafeFile,
}

impl CredentialsStore {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keychain => "keychain",
            Self::Pass => "pass",
            Self::UnsafeFile => "unsafe_file",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Client {
    binary: PathBuf,
    cache_dir: Option<PathBuf>,
    environment: BTreeMap<OsString, OsString>,
}

impl Client {
    pub fn new(binary: impl Into<PathBuf>) -> Result<Self> {
        let binary = binary.into();
        if !binary.is_file() {
            return Err(Error::BinaryNotFound(binary));
        }
        Ok(Self {
            binary,
            cache_dir: None,
            environment: BTreeMap::new(),
        })
    }

    /// Selects one portable data root for cache, state and CLI application data.
    pub fn with_cache_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(path.into());
        self
    }

    /// Selects where the CLI persists its authenticated session.
    pub fn with_credentials_store(mut self, store: CredentialsStore) -> Self {
        self.environment.insert(
            "PROTON_DRIVE_CREDENTIALS_STORE".into(),
            store.as_str().into(),
        );
        self
    }

    /// Adds an environment override for the child CLI process.
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Starts the official browser-based login flow.
    ///
    /// The CLI prints a URL that may be opened on another device and persists
    /// the resulting session in the configured credentials store. Proton does
    /// not accept a username or password through this command.
    pub fn login_interactive(&self) -> Result<()> {
        self.run_interactive_success([OsStr::new("auth"), OsStr::new("login")])
    }

    /// Removes the persisted CLI session from the configured credentials store.
    pub fn logout(&self) -> Result<()> {
        self.run_success(vec!["auth".into(), "logout".into()])
    }

    /// Verifies that the executable is a Proton Drive standalone CLI rather
    /// than a plain Bun runtime or an unrelated command.
    pub fn probe(&self) -> Result<Probe> {
        let output = self.output([OsStr::new("version")])?;
        let stdout = decode(output.stdout)?;
        let stderr = decode(output.stderr)?;
        let combined = format!("{stdout}{stderr}");
        if combined.contains("Bun is a fast JavaScript runtime") {
            return Err(Error::BareBunRuntime);
        }
        if !output.status.success() {
            let help = self.output([OsStr::new("--help")])?;
            let help_text = format!("{}{}", decode(help.stdout)?, decode(help.stderr)?);
            if help_text.contains("Bun is a fast JavaScript runtime") {
                return Err(Error::BareBunRuntime);
            }
            return Err(Error::CommandFailed {
                status: output.status,
                stderr,
            });
        }
        let lower = combined.to_ascii_lowercase();
        if !lower.contains("proton") && !lower.contains("drive") && !lower.contains("cli") {
            return Err(Error::Incompatible(combined.trim().to_owned()));
        }
        Ok(Probe {
            version_output: combined.trim().to_owned(),
        })
    }

    pub fn list(&self, remote_path: &str, node_type: Option<NodeType>) -> Result<Vec<DriveObject>> {
        let mut args = fs_args("list");
        if let Some(node_type) = node_type {
            args.extend(["--type".into(), node_type.as_str().into()]);
        }
        args.push(remote_path.into());
        self.run_json_objects(args)
    }

    pub fn info(&self, remote_path: &str) -> Result<DriveObject> {
        self.run_json_object(fs_args_with("info", [remote_path]))
    }

    pub fn size(&self, remote_path: &str) -> Result<DriveObject> {
        self.run_json_object(fs_args_with("size", [remote_path]))
    }

    pub fn create_folder(&self, parent: &str, name: &str) -> Result<DriveObject> {
        self.run_json_object(fs_args_with("create-folder", [parent, name]))
    }

    pub fn rename(&self, remote_path: &str, new_name: &str) -> Result<DriveObject> {
        self.run_json_object(fs_args_with("rename", [remote_path, new_name]))
    }

    pub fn copy(
        &self,
        sources: &[&str],
        target_parent: &str,
        name: Option<&str>,
    ) -> Result<Vec<DriveObject>> {
        let mut args = fs_args("copy");
        if let Some(name) = name {
            args.extend(["--name".into(), name.into()]);
        }
        add_sources(&mut args, sources)?;
        args.push(target_parent.into());
        self.run_json_objects(args)
    }

    pub fn move_nodes(&self, sources: &[&str], target_parent: &str) -> Result<Vec<DriveObject>> {
        let mut args = fs_args("move");
        add_sources(&mut args, sources)?;
        args.push(target_parent.into());
        self.run_json_objects(args)
    }

    pub fn trash(&self, paths: &[&str]) -> Result<Vec<DriveObject>> {
        self.multi_path("trash", paths)
    }

    pub fn restore(&self, paths: &[&str]) -> Result<Vec<DriveObject>> {
        self.multi_path("restore", paths)
    }

    pub fn delete(&self, paths: &[&str]) -> Result<Vec<DriveObject>> {
        self.multi_path("delete", paths)
    }

    /// Requests asynchronous deletion of every item in `/trash`.
    pub fn empty_trash(&self) -> Result<()> {
        self.run_success(fs_args("empty-trash"))
    }

    pub fn upload(
        &self,
        local_sources: &[impl AsRef<OsStr>],
        remote_parent: &str,
        options: &UploadOptions,
    ) -> Result<JsonValue> {
        if local_sources.is_empty() {
            return Err(Error::MissingSource);
        }
        let mut args = fs_args("upload");
        if let Some(strategy) = options.file_conflict {
            args.extend([
                "--file-conflict-strategy".into(),
                conflict_str!(strategy).into(),
            ]);
        }
        if let Some(strategy) = options.folder_conflict {
            args.extend([
                "--folder-conflict-strategy".into(),
                conflict_str!(upload_folder strategy).into(),
            ]);
        }
        if options.skip_thumbnails {
            args.push("--skip-thumbnails".into());
        }
        args.extend(
            local_sources
                .iter()
                .map(|path| path.as_ref().to_os_string()),
        );
        args.push(remote_parent.into());
        self.run_json(args)
    }

    pub fn download(
        &self,
        remote_sources: &[&str],
        local_folder: impl AsRef<OsStr>,
        options: &DownloadOptions,
    ) -> Result<JsonValue> {
        let mut args = fs_args("download");
        if let Some(strategy) = options.file_conflict {
            args.extend([
                "--file-conflict-strategy".into(),
                conflict_str!(download_file strategy).into(),
            ]);
        }
        if let Some(strategy) = options.folder_conflict {
            args.extend([
                "--folder-conflict-strategy".into(),
                conflict_str!(download_folder strategy).into(),
            ]);
        }
        add_sources(&mut args, remote_sources)?;
        args.push(local_folder.as_ref().to_os_string());
        self.run_json(args)
    }

    /// Runs an arbitrary CLI command in JSON mode. This is the forward-compatible
    /// escape hatch for commands introduced after this crate version.
    pub fn run_json(&self, args: Vec<OsString>) -> Result<JsonValue> {
        let output = self.output(args)?;
        let stdout = decode(output.stdout)?;
        let stderr = decode(output.stderr)?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                status: output.status,
                stderr,
            });
        }
        json::parse(&stdout).map_err(|source| Error::InvalidJson {
            source,
            output: stdout,
        })
    }

    fn run_success(&self, args: Vec<OsString>) -> Result<()> {
        let output = self.output(args)?;
        let stderr = decode(output.stderr)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                status: output.status,
                stderr,
            })
        }
    }

    fn run_interactive_success<I, S>(&self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let status = self
            .command()
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|source| Error::Spawn {
                path: self.binary.clone(),
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                status,
                stderr: "interactive login failed; see the CLI output above".to_owned(),
            })
        }
    }

    fn multi_path(&self, action: &str, paths: &[&str]) -> Result<Vec<DriveObject>> {
        let mut args = fs_args(action);
        add_sources(&mut args, paths)?;
        self.run_json_objects(args)
    }

    fn run_json_object(&self, args: Vec<OsString>) -> Result<DriveObject> {
        self.run_json(args).map(DriveObject)
    }

    fn run_json_objects(&self, args: Vec<OsString>) -> Result<Vec<DriveObject>> {
        match self.run_json(args)? {
            JsonValue::Array(values) => Ok(values.into_iter().map(DriveObject).collect()),
            value => Err(Error::Incompatible(format!(
                "expected a JSON array, received {value:?}"
            ))),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command.envs(&self.environment);
        if let Some(path) = &self.cache_dir {
            command.env("PROTON_DRIVE_CACHE_DIR", path);
        }
        command
    }

    fn output<I, S>(&self, args: I) -> Result<std::process::Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|source| Error::Spawn {
                path: self.binary.clone(),
                source,
            })
    }
}

fn decode(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|_| Error::NonUtf8)
}

fn fs_args(action: &str) -> Vec<OsString> {
    vec!["filesystem".into(), action.into(), "--json".into()]
}

fn fs_args_with<const N: usize>(action: &str, values: [&str; N]) -> Vec<OsString> {
    let mut args = fs_args(action);
    args.extend(values.into_iter().map(OsString::from));
    args
}

fn add_sources(args: &mut Vec<OsString>, sources: &[&str]) -> Result<()> {
    if sources.is_empty() {
        return Err(Error::MissingSource);
    }
    args.extend(sources.iter().map(OsString::from));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    struct TestDir(PathBuf);

    #[cfg(unix)]
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn fake_cli(script: &str) -> (TestDir, PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "proton-drive-rs-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let dir = TestDir(path);
        let path = dir.0.join("proton-drive");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    #[test]
    #[cfg(unix)]
    fn probes_and_parses_list_json() {
        let (_dir, path) = fake_cli(
            r##"#!/bin/sh
if [ "$1" = version ]; then echo "Proton Drive CLI 0.9.0"; exit 0; fi
printf '[{"uid":"u1","type":"file","name":{"ok":true,"value":"hello.txt"},"future":42}]\n'
"##,
        );
        let client = Client::new(path).unwrap();
        assert!(client.probe().unwrap().version_output.contains("0.9.0"));
        let values = client.list("/my-files", Some(NodeType::File)).unwrap();
        assert_eq!(values[0].uid(), Some("u1"));
        assert_eq!(values[0].name(), Some("hello.txt"));
        assert_eq!(
            values[0]
                .as_json()
                .get("future")
                .and_then(JsonValue::as_u64),
            Some(42)
        );
    }

    #[test]
    #[cfg(unix)]
    fn rejects_bare_bun_runtime() {
        let (_dir, path) = fake_cli(
            "#!/bin/sh\nif [ \"$1\" = version ]; then echo 'error: Script not found' >&2; exit 1; fi\necho 'Bun is a fast JavaScript runtime, package manager, bundler, and test runner.'\n",
        );
        assert!(matches!(
            Client::new(path).unwrap().probe(),
            Err(Error::BareBunRuntime)
        ));
    }

    #[test]
    fn rejects_empty_source_lists_without_spawning() {
        let client = Client {
            binary: PathBuf::from("not-used"),
            cache_dir: None,
            environment: BTreeMap::new(),
        };
        assert!(matches!(client.trash(&[]), Err(Error::MissingSource)));
        let no_local_sources: [&str; 0] = [];
        assert!(matches!(
            client.upload(&no_local_sources, "/my-files", &UploadOptions::default()),
            Err(Error::MissingSource)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn login_uses_configured_state_and_credentials_store() {
        let (dir, path) = fake_cli(
            r##"#!/bin/sh
printf '%s\n' "$@" > "$PROTON_DRIVE_CACHE_DIR/args"
printf '%s\n' "$PROTON_DRIVE_CREDENTIALS_STORE" > "$PROTON_DRIVE_CACHE_DIR/store"
"##,
        );
        let state = dir.0.join("state");
        fs::create_dir(&state).unwrap();
        Client::new(path)
            .unwrap()
            .with_cache_dir(&state)
            .with_credentials_store(CredentialsStore::Pass)
            .login_interactive()
            .unwrap();

        assert_eq!(
            fs::read_to_string(state.join("args")).unwrap(),
            "auth\nlogin\n"
        );
        assert_eq!(fs::read_to_string(state.join("store")).unwrap(), "pass\n");
    }
}
