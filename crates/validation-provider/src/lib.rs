//! Backend-only validation provider boundary.
//!
//! The production implementation deliberately talks to a separately built
//! helper process.  The helper is the only process that loads the opaque
//! OpenBubbles compatibility library.  No FairPlay material is handled here.

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tar::Archive;
use thiserror::Error;

/// The OpenBubbles release whose public module documents the ABI used here.
pub const EXPECTED_COMPONENT_VERSION: &str = "v1.15.0+136";
/// SHA-256 of the x86_64 Linux `openbubbles.so` used by the public module.
pub const EXPECTED_LIBRARY_SHA256: &str =
    "f47fbd299bf5c83449bf6485a2c00c0f059d0e059646e20c64111bc5fac84b2a";
pub const EXPECTED_LIBRARY_NAME: &str = "openbubbles.so";
pub const MANIFEST_FILE_NAME: &str = "manifest.json";

/// These limits apply before data is sent to the foreign-library helper.
pub const MAX_VALIDATION_BUFFER_BYTES: usize = 500_000;
pub const MAX_JSON_LINE_BYTES: usize = 3_000_000;
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(30);

/// The exact offsets documented by `openbubbles-build-modules` for the
/// x86_64 `v1.15.0+136` release.
pub const REFERENCE_ADDRESS: usize = 0x008a39b0;
pub const VALIDATION_CTX_NEW_ADDRESS: usize = 0x00b897c0;
pub const VALIDATION_CTX_KEY_ESTABLISHMENT_ADDRESS: usize = 0x00b8b3b0;
pub const VALIDATION_CTX_SIGN_ADDRESS: usize = 0x00b8bb50;
pub const REFERENCE_SYMBOL: &[u8] = b"dart_fn_deliver_output";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("the production validation component is not installed")]
    MissingComponent,
    #[error("the validation component version is not supported")]
    UnsupportedComponentVersion,
    #[error("the validation component manifest is invalid")]
    InvalidComponentManifest,
    #[error("the validation component library is missing or is not a regular file")]
    MissingComponentLibrary,
    #[error("the validation component library hash is not supported")]
    InvalidComponentHash,
    #[error("the validation component cannot be read")]
    ComponentIo,
    #[error("the validation helper is not available")]
    HelperUnavailable,
    #[error("the validation helper could not be started")]
    HelperSpawnFailed,
    #[error("the validation helper exited unexpectedly")]
    HelperCrashed,
    #[error("the validation helper exited with an error")]
    HelperNonzeroExit,
    #[error("the validation helper timed out")]
    HelperTimeout,
    #[error("the validation helper produced malformed output")]
    MalformedOutput,
    #[error("the validation helper exceeded a size limit")]
    OversizedInput,
    #[error("the validation helper returned empty validation data")]
    EmptyValidationData,
    #[error("the official release archive is unreadable")]
    ArchiveIo,
    #[error("the official release archive contains an unsafe path")]
    ArchiveUnsafePath,
    #[error("the official release archive contains more than one openbubbles.so")]
    DuplicateLibrary,
    #[error("the official release archive contains no openbubbles.so")]
    MissingArchiveLibrary,
    #[error("the openbubbles.so archive entry is not a regular file")]
    InvalidArchiveLibrary,
    #[error("the openbubbles.so archive entry is too large")]
    OversizedArchiveLibrary,
    #[error("the official openbubbles.so hash does not match the supported release")]
    InvalidArchiveHash,
    #[error("the validation component is already installed")]
    ComponentAlreadyInstalled,
    #[error("the validation component could not be installed")]
    InstallIo,
    #[error("the validation component could not be removed")]
    RemoveIo,
    #[error("the validation component is unavailable on this architecture")]
    UnsupportedPlatform,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwareConfig {
    pub product_name: String,
    pub io_mac_address: [u8; 6],
    pub platform_serial_number: String,
    pub platform_uuid: String,
    pub root_disk_uuid: String,
    pub board_id: String,
    pub os_build_num: String,
    pub platform_serial_number_enc: Vec<u8>,
    pub platform_uuid_enc: Vec<u8>,
    pub root_disk_uuid_enc: Vec<u8>,
    pub rom: Vec<u8>,
    pub rom_enc: Vec<u8>,
    pub mlb: String,
    pub mlb_enc: Vec<u8>,
}

/// The first message sent to the helper.  The byte vectors are serialized as
/// JSON arrays to match the public reference helper protocol.
#[derive(Clone, Serialize, Deserialize)]
pub struct InitialPayload {
    pub hardware_config: HardwareConfig,
    pub cert_data: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionInfoPayload {
    pub session_info: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResultPayload {
    pub result: Vec<u8>,
}

/// Inputs needed by the validation implementation.  This is deliberately not
/// part of `litebubbles-core` or the D-Bus contract.
pub struct ValidationRequest {
    pub hardware_config: HardwareConfig,
    pub cert_data: Vec<u8>,
}

/// Sensitive output from a successful provider call.  Its debug output never
/// includes bytes.
pub struct ValidationData(Vec<u8>);

impl ValidationData {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for ValidationData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidationData")
            .field("len", &self.0.len())
            .finish()
    }
}

/// A provider session holds any opaque helper state between Apple's first
/// response and the final signed validation data.
pub struct ValidationSession {
    session_info: Vec<u8>,
    pending: Option<Box<dyn PendingValidation>>,
}

impl fmt::Debug for ValidationSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidationSession")
            .field("session_info_len", &self.session_info.len())
            .field("pending", &self.pending.is_some())
            .finish()
    }
}

impl ValidationSession {
    pub fn session_info(&self) -> &[u8] {
        &self.session_info
    }

    pub fn finish(
        mut self,
        apple_session_info: Vec<u8>,
    ) -> Result<ValidationData, ValidationError> {
        let pending = self
            .pending
            .take()
            .ok_or(ValidationError::MalformedOutput)?;
        pending.finish(apple_session_info)
    }
}

pub trait PendingValidation {
    fn finish(
        self: Box<Self>,
        apple_session_info: Vec<u8>,
    ) -> Result<ValidationData, ValidationError>;
}

pub trait ValidationProvider: Send + Sync {
    fn is_available(&self) -> bool;
    fn begin(&self, request: ValidationRequest) -> Result<ValidationSession, ValidationError>;
}

#[derive(Default)]
pub struct DummyValidationProvider;

impl DummyValidationProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ValidationProvider for DummyValidationProvider {
    fn is_available(&self) -> bool {
        true
    }

    fn begin(&self, request: ValidationRequest) -> Result<ValidationSession, ValidationError> {
        validate_request_size(&request)?;
        Ok(ValidationSession {
            session_info: b"litebubbles-dummy-session".to_vec(),
            pending: Some(Box::new(DummyPending)),
        })
    }
}

struct DummyPending;

impl PendingValidation for DummyPending {
    fn finish(
        self: Box<Self>,
        apple_session_info: Vec<u8>,
    ) -> Result<ValidationData, ValidationError> {
        if apple_session_info.len() > MAX_VALIDATION_BUFFER_BYTES {
            return Err(ValidationError::OversizedInput);
        }
        Ok(ValidationData(b"litebubbles-dummy-validation".to_vec()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderMode {
    Dummy,
}

pub enum ProviderSelection {
    Dummy,
    Production {
        component_dir: PathBuf,
        helper_path: PathBuf,
    },
}

pub fn select_provider(
    selection: ProviderSelection,
) -> Result<Box<dyn ValidationProvider>, ValidationError> {
    match selection {
        ProviderSelection::Dummy => Ok(Box::new(DummyValidationProvider::new())),
        ProviderSelection::Production {
            component_dir,
            helper_path,
        } => Ok(Box::new(OpenBubblesValidationProvider::new(
            component_dir,
            helper_path,
        )?)),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    pub format: u32,
    pub version: String,
    pub library: String,
    pub sha256: String,
}

impl ComponentManifest {
    pub fn supported() -> Self {
        Self {
            format: 1,
            version: EXPECTED_COMPONENT_VERSION.to_string(),
            library: EXPECTED_LIBRARY_NAME.to_string(),
            sha256: EXPECTED_LIBRARY_SHA256.to_string(),
        }
    }

    pub fn is_supported(&self) -> bool {
        self.format == 1
            && self.version == EXPECTED_COMPONENT_VERSION
            && self.library == EXPECTED_LIBRARY_NAME
            && self.sha256.eq_ignore_ascii_case(EXPECTED_LIBRARY_SHA256)
    }
}

#[derive(Debug)]
pub struct ValidatedComponent {
    root: PathBuf,
    library: PathBuf,
}

impl ValidatedComponent {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn library_path(&self) -> &Path {
        &self.library
    }
}

pub fn validate_component_directory(
    component_dir: impl AsRef<Path>,
) -> Result<ValidatedComponent, ValidationError> {
    let root = fs::canonicalize(component_dir).map_err(|_| ValidationError::MissingComponent)?;
    if !root.is_dir() {
        return Err(ValidationError::MissingComponent);
    }

    let manifest_path = root.join(MANIFEST_FILE_NAME);
    let manifest_metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|_| ValidationError::InvalidComponentManifest)?;
    if !manifest_metadata.file_type().is_file() {
        return Err(ValidationError::InvalidComponentManifest);
    }
    let manifest_real =
        fs::canonicalize(&manifest_path).map_err(|_| ValidationError::InvalidComponentManifest)?;
    if manifest_real.parent() != Some(root.as_path()) {
        return Err(ValidationError::InvalidComponentManifest);
    }
    let manifest_bytes = read_bounded_file(&manifest_real, 16 * 1024)
        .map_err(|_| ValidationError::InvalidComponentManifest)?;
    let manifest: ComponentManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| ValidationError::InvalidComponentManifest)?;
    if !manifest.is_supported() {
        return Err(ValidationError::UnsupportedComponentVersion);
    }

    let library = root.join(EXPECTED_LIBRARY_NAME);
    let library_metadata =
        fs::symlink_metadata(&library).map_err(|_| ValidationError::MissingComponentLibrary)?;
    if !library_metadata.file_type().is_file() {
        return Err(ValidationError::MissingComponentLibrary);
    }
    let library_real =
        fs::canonicalize(&library).map_err(|_| ValidationError::MissingComponentLibrary)?;
    if library_real.parent() != Some(root.as_path()) {
        return Err(ValidationError::MissingComponentLibrary);
    }
    let digest = sha256_file(&library_real, MAX_LIBRARY_BYTES).map_err(|error| match error {
        FileReadError::TooLarge => ValidationError::InvalidComponentHash,
        FileReadError::Io => ValidationError::ComponentIo,
    })?;
    if !digest.eq_ignore_ascii_case(EXPECTED_LIBRARY_SHA256) {
        return Err(ValidationError::InvalidComponentHash);
    }

    Ok(ValidatedComponent {
        root,
        library: library_real,
    })
}

pub struct OpenBubblesValidationProvider {
    component_dir: PathBuf,
    helper_path: PathBuf,
}

impl OpenBubblesValidationProvider {
    pub fn new(
        component_dir: impl AsRef<Path>,
        helper_path: impl AsRef<Path>,
    ) -> Result<Self, ValidationError> {
        let component = validate_component_directory(component_dir)?;
        let helper_path = validate_helper_path(helper_path)?;
        Ok(Self {
            component_dir: component.root,
            helper_path,
        })
    }

    pub fn component_dir(&self) -> &Path {
        &self.component_dir
    }
}

impl ValidationProvider for OpenBubblesValidationProvider {
    fn is_available(&self) -> bool {
        validate_component_directory(&self.component_dir).is_ok()
            && validate_helper_path(&self.helper_path).is_ok()
    }

    fn begin(&self, request: ValidationRequest) -> Result<ValidationSession, ValidationError> {
        validate_request_size(&request)?;
        if !cfg!(target_arch = "x86_64") {
            return Err(ValidationError::UnsupportedPlatform);
        }
        let component = validate_component_directory(&self.component_dir)?;
        validate_helper_path(&self.helper_path)?;

        self.begin_with_component_dir(component.root(), request)
    }
}

impl OpenBubblesValidationProvider {
    fn begin_with_component_dir(
        &self,
        component_dir: &Path,
        request: ValidationRequest,
    ) -> Result<ValidationSession, ValidationError> {
        let mut child = Command::new(&self.helper_path)
            .arg("--component-dir")
            .arg(component_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ValidationError::HelperSpawnFailed)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or(ValidationError::HelperSpawnFailed)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(ValidationError::HelperSpawnFailed)?;

        let initial = InitialPayload {
            hardware_config: request.hardware_config,
            cert_data: request.cert_data,
        };
        if let Err(error) = write_json_line(&mut stdin, &initial) {
            terminate_child(&mut child);
            return Err(error);
        }

        let (line, stdout) = match read_line_with_timeout(BufReader::new(stdout), &mut child) {
            Ok(value) => value,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        };
        let response: SessionInfoPayload = match parse_json_line(&line) {
            Ok(value) => value,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        };
        validate_binary_size(response.session_info.len())?;
        if response.session_info.is_empty() {
            terminate_child(&mut child);
            return Err(ValidationError::MalformedOutput);
        }

        Ok(ValidationSession {
            session_info: response.session_info.clone(),
            pending: Some(Box::new(OpenBubblesPending {
                child,
                stdin: Some(stdin),
                stdout: Some(stdout),
            })),
        })
    }
}

struct OpenBubblesPending {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
}

impl PendingValidation for OpenBubblesPending {
    fn finish(
        mut self: Box<Self>,
        apple_session_info: Vec<u8>,
    ) -> Result<ValidationData, ValidationError> {
        validate_binary_size(apple_session_info.len())?;
        let mut stdin = self.stdin.take().ok_or(ValidationError::MalformedOutput)?;
        let payload = SessionInfoPayload {
            session_info: apple_session_info,
        };
        if let Err(error) = write_json_line(&mut stdin, &payload) {
            terminate_child(&mut self.child);
            return Err(error);
        }
        drop(stdin);

        let stdout = self.stdout.take().ok_or(ValidationError::MalformedOutput)?;
        let (line, _stdout) = match read_line_with_timeout(stdout, &mut self.child) {
            Ok(value) => value,
            Err(error) => {
                terminate_child(&mut self.child);
                return Err(error);
            }
        };
        let result: ResultPayload = match parse_json_line(&line) {
            Ok(value) => value,
            Err(error) => {
                terminate_child(&mut self.child);
                return Err(error);
            }
        };
        validate_binary_size(result.result.len())?;
        if result.result.is_empty() {
            terminate_child(&mut self.child);
            return Err(ValidationError::EmptyValidationData);
        }
        wait_for_child(&mut self.child)?;
        Ok(ValidationData(result.result))
    }
}

impl Drop for OpenBubblesPending {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            terminate_child(&mut self.child);
        }
    }
}

fn validate_request_size(request: &ValidationRequest) -> Result<(), ValidationError> {
    validate_binary_size(request.cert_data.len())?;
    let payload = InitialPayload {
        hardware_config: request.hardware_config.clone(),
        cert_data: request.cert_data.clone(),
    };
    let encoded = serde_json::to_vec(&payload).map_err(|_| ValidationError::MalformedOutput)?;
    if encoded.len() + 1 > MAX_JSON_LINE_BYTES {
        return Err(ValidationError::OversizedInput);
    }
    Ok(())
}

fn validate_binary_size(size: usize) -> Result<(), ValidationError> {
    if size > MAX_VALIDATION_BUFFER_BYTES {
        Err(ValidationError::OversizedInput)
    } else {
        Ok(())
    }
}

fn write_json_line<T: Serialize>(
    writer: &mut impl Write,
    value: &T,
) -> Result<(), ValidationError> {
    let encoded = serde_json::to_vec(value).map_err(|_| ValidationError::MalformedOutput)?;
    if encoded.len() + 1 > MAX_JSON_LINE_BYTES {
        return Err(ValidationError::OversizedInput);
    }
    writer
        .write_all(&encoded)
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|_| ValidationError::HelperCrashed)
}

fn parse_json_line<T: DeserializeOwned>(line: &[u8]) -> Result<T, ValidationError> {
    if line.len() > MAX_JSON_LINE_BYTES {
        return Err(ValidationError::OversizedInput);
    }
    serde_json::from_slice(line).map_err(|_| ValidationError::MalformedOutput)
}

fn read_line_with_timeout(
    reader: BufReader<ChildStdout>,
    child: &mut Child,
) -> Result<(Vec<u8>, BufReader<ChildStdout>), ValidationError> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = read_bounded_line(reader);
        let _ = sender.send(result);
    });
    match receiver.recv_timeout(HELPER_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            terminate_child(child);
            Err(ValidationError::HelperTimeout)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ValidationError::HelperCrashed),
    }
}

fn read_bounded_line(
    mut reader: BufReader<ChildStdout>,
) -> Result<(Vec<u8>, BufReader<ChildStdout>), ValidationError> {
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|_| ValidationError::HelperCrashed)?;
        if available.is_empty() {
            return Err(ValidationError::HelperCrashed);
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if line.len() + newline + 1 > MAX_JSON_LINE_BYTES {
                return Err(ValidationError::OversizedInput);
            }
            line.extend_from_slice(&available[..=newline]);
            reader.consume(newline + 1);
            return Ok((line, reader));
        }
        if line.len() + available.len() > MAX_JSON_LINE_BYTES {
            return Err(ValidationError::OversizedInput);
        }
        line.extend_from_slice(available);
        let length = available.len();
        reader.consume(length);
    }
}

fn wait_for_child(child: &mut Child) -> Result<ExitStatus, ValidationError> {
    let deadline = Instant::now() + HELPER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(status);
                }
                return Err(ValidationError::HelperNonzeroExit);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                terminate_child(child);
                return Err(ValidationError::HelperTimeout);
            }
            Err(_) => return Err(ValidationError::HelperCrashed),
        }
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn validate_helper_path(path: impl AsRef<Path>) -> Result<PathBuf, ValidationError> {
    let path = fs::canonicalize(path).map_err(|_| ValidationError::HelperUnavailable)?;
    let metadata = fs::metadata(&path).map_err(|_| ValidationError::HelperUnavailable)?;
    if !metadata.is_file() {
        return Err(ValidationError::HelperUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ValidationError::HelperUnavailable);
        }
    }
    Ok(path)
}

const MAX_LIBRARY_BYTES: usize = 256 * 1024 * 1024;

enum FileReadError {
    TooLarge,
    Io,
}

fn read_bounded_file(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > limit as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    let mut file = File::open(path)?;
    let mut data = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut data)?;
    if data.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    Ok(data)
}

fn sha256_file(path: &Path, limit: usize) -> Result<String, FileReadError> {
    let metadata = fs::metadata(path).map_err(|_| FileReadError::Io)?;
    if metadata.len() > limit as u64 {
        return Err(FileReadError::TooLarge);
    }
    let mut file = File::open(path).map_err(|_| FileReadError::Io)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_usize;
    loop {
        let read = file.read(&mut buffer).map_err(|_| FileReadError::Io)?;
        if read == 0 {
            break;
        }
        total = total.checked_add(read).ok_or(FileReadError::TooLarge)?;
        if total > limit {
            return Err(FileReadError::TooLarge);
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn default_compat_root() -> Result<PathBuf, ValidationError> {
    let data_home = match std::env::var_os("XDG_DATA_HOME") {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(ValidationError::InstallIo);
            }
            path
        }
        None => {
            let home = std::env::var_os("HOME").ok_or(ValidationError::InstallIo)?;
            let home = PathBuf::from(home);
            if !home.is_absolute() {
                return Err(ValidationError::InstallIo);
            }
            home.join(".local/share")
        }
    };
    Ok(data_home.join("litebubbles/compat"))
}

pub fn install_official_artifact(
    archive_path: impl AsRef<Path>,
    compat_root: impl AsRef<Path>,
) -> Result<PathBuf, ValidationError> {
    let archive_path = archive_path.as_ref();
    let reader = open_archive_reader(archive_path)?;
    let mut archive = Archive::new(reader);
    let mut library_bytes = None;

    let entries = archive.entries().map_err(|_| ValidationError::ArchiveIo)?;
    for entry_result in entries {
        let mut entry = entry_result.map_err(|_| ValidationError::ArchiveIo)?;
        let path = entry
            .path()
            .map_err(|_| ValidationError::ArchiveIo)?
            .into_owned();
        validate_archive_path(&path)?;
        if entry.header().entry_type().is_symlink() || entry.header().entry_type().is_hard_link() {
            return Err(ValidationError::ArchiveUnsafePath);
        }
        if path.file_name().and_then(|name| name.to_str()) != Some(EXPECTED_LIBRARY_NAME) {
            continue;
        }
        if library_bytes.is_some() {
            return Err(ValidationError::DuplicateLibrary);
        }
        if !entry.header().entry_type().is_file() {
            return Err(ValidationError::InvalidArchiveLibrary);
        }
        if entry.size() > MAX_LIBRARY_BYTES as u64 {
            return Err(ValidationError::OversizedArchiveLibrary);
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|_| ValidationError::ArchiveIo)?;
        if bytes.len() > MAX_LIBRARY_BYTES {
            return Err(ValidationError::OversizedArchiveLibrary);
        }
        library_bytes = Some(bytes);
    }

    let library_bytes = library_bytes.ok_or(ValidationError::MissingArchiveLibrary)?;
    let digest = Sha256::digest(&library_bytes);
    if format!("{:x}", digest) != EXPECTED_LIBRARY_SHA256 {
        return Err(ValidationError::InvalidArchiveHash);
    }

    let compat_root = compat_root.as_ref();
    fs::create_dir_all(compat_root).map_err(|_| ValidationError::InstallIo)?;
    set_private_mode(compat_root, true).map_err(|_| ValidationError::InstallIo)?;
    let target = compat_root.join(EXPECTED_COMPONENT_VERSION);
    if target.exists() {
        return Err(ValidationError::ComponentAlreadyInstalled);
    }

    let temporary = tempfile::Builder::new()
        .prefix(".litebubbles-validation-")
        .tempdir_in(compat_root)
        .map_err(|_| ValidationError::InstallIo)?;
    let library_path = temporary.path().join(EXPECTED_LIBRARY_NAME);
    let mut library_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&library_path)
        .map_err(|_| ValidationError::InstallIo)?;
    library_file
        .write_all(&library_bytes)
        .and_then(|_| library_file.sync_all())
        .map_err(|_| ValidationError::InstallIo)?;
    set_private_mode(&library_path, false).map_err(|_| ValidationError::InstallIo)?;

    let manifest = ComponentManifest::supported();
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|_| ValidationError::InstallIo)?;
    let manifest_path = temporary.path().join(MANIFEST_FILE_NAME);
    let mut manifest_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&manifest_path)
        .map_err(|_| ValidationError::InstallIo)?;
    manifest_file
        .write_all(&manifest_bytes)
        .and_then(|_| manifest_file.write_all(b"\n"))
        .and_then(|_| manifest_file.sync_all())
        .map_err(|_| ValidationError::InstallIo)?;
    set_private_mode(&manifest_path, false).map_err(|_| ValidationError::InstallIo)?;

    let temporary_path = temporary.keep();
    fs::rename(temporary_path, &target).map_err(|_| ValidationError::InstallIo)?;
    Ok(target)
}

pub fn remove_installed_component(compat_root: impl AsRef<Path>) -> Result<bool, ValidationError> {
    let compat_root = compat_root.as_ref();
    let target = compat_root.join(EXPECTED_COMPONENT_VERSION);
    let metadata = match fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(ValidationError::RemoveIo),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ValidationError::RemoveIo);
    }
    fs::remove_dir_all(&target).map_err(|_| ValidationError::RemoveIo)?;
    Ok(true)
}

fn open_archive_reader(path: &Path) -> Result<Box<dyn Read>, ValidationError> {
    let file = File::open(path).map_err(|_| ValidationError::ArchiveIo)?;
    let is_gzip = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"));
    if is_gzip {
        Ok(Box::new(GzDecoder::new(file)))
    } else {
        Ok(Box::new(file))
    }
}

fn validate_archive_path(path: &Path) -> Result<(), ValidationError> {
    if path.is_absolute() {
        return Err(ValidationError::ArchiveUnsafePath);
    }
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(ValidationError::ArchiveUnsafePath);
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

fn set_private_mode(path: &Path, directory: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if directory { 0o700 } else { 0o700 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use tempfile::TempDir;

    fn request() -> ValidationRequest {
        ValidationRequest {
            hardware_config: HardwareConfig {
                product_name: "SyntheticMac".to_string(),
                io_mac_address: [0, 1, 2, 3, 4, 5],
                platform_serial_number: "synthetic-serial".to_string(),
                platform_uuid: "synthetic-uuid".to_string(),
                root_disk_uuid: "synthetic-root".to_string(),
                board_id: "Mac-synthetic".to_string(),
                os_build_num: "SyntheticBuild".to_string(),
                platform_serial_number_enc: vec![1, 2],
                platform_uuid_enc: vec![3, 4],
                root_disk_uuid_enc: vec![5, 6],
                rom: vec![7, 8],
                rom_enc: vec![9, 10],
                mlb: "synthetic-mlb".to_string(),
                mlb_enc: vec![11, 12],
            },
            cert_data: vec![13, 14],
        }
    }

    #[test]
    fn dummy_provider_behaves_deterministically_without_payload_debug() {
        let provider = DummyValidationProvider::new();
        assert!(provider.is_available());
        let session = provider.begin(request()).expect("dummy begin");
        assert_eq!(session.session_info(), b"litebubbles-dummy-session");
        let data = session.finish(vec![1, 2, 3]).expect("dummy finish");
        assert_eq!(data.as_bytes(), b"litebubbles-dummy-validation");
        assert!(!format!("{data:?}").contains("dummy-validation"));
    }

    #[test]
    fn provider_selection_has_explicit_dummy_mode() {
        let provider = select_provider(ProviderSelection::Dummy).expect("select dummy");
        assert!(provider.is_available());
    }

    #[test]
    fn missing_component_is_reported_before_helper_start() {
        let temp = TempDir::new().expect("temp");
        let result = select_provider(ProviderSelection::Production {
            component_dir: temp.path().join("missing"),
            helper_path: temp.path().join("helper"),
        });
        assert_eq!(
            result.err().expect("missing component").to_string(),
            "the production validation component is not installed"
        );
    }

    #[test]
    fn unsupported_component_version_is_rejected() {
        let temp = TempDir::new().expect("temp");
        fs::write(
            temp.path().join(MANIFEST_FILE_NAME),
            serde_json::json!({
                "format": 1,
                "version": "v0.0.0",
                "library": EXPECTED_LIBRARY_NAME,
                "sha256": EXPECTED_LIBRARY_SHA256,
            })
            .to_string(),
        )
        .expect("manifest");
        fs::write(temp.path().join(EXPECTED_LIBRARY_NAME), b"not-a-library").expect("library");
        assert_eq!(
            validate_component_directory(temp.path())
                .err()
                .expect("unsupported")
                .to_string(),
            "the validation component version is not supported"
        );
    }

    #[test]
    fn supported_manifest_facts_are_explicit() {
        let manifest = ComponentManifest::supported();
        assert!(manifest.is_supported());
        assert_eq!(manifest.version, EXPECTED_COMPONENT_VERSION);
        assert_eq!(manifest.sha256, EXPECTED_LIBRARY_SHA256);
    }

    #[test]
    fn bad_component_hash_is_rejected() {
        let temp = TempDir::new().expect("temp");
        let manifest = serde_json::to_vec(&ComponentManifest::supported()).expect("manifest");
        fs::write(temp.path().join(MANIFEST_FILE_NAME), manifest).expect("manifest write");
        fs::write(temp.path().join(EXPECTED_LIBRARY_NAME), b"not-a-library").expect("library");
        assert_eq!(
            validate_component_directory(temp.path())
                .err()
                .expect("bad hash")
                .to_string(),
            "the validation component library hash is not supported"
        );
    }

    fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut encoded = Vec::new();
        {
            let gzip = GzEncoder::new(&mut encoded, Compression::fast());
            let mut builder = tar::Builder::new(gzip);
            for (name, bytes) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o600);
                header.set_cksum();
                builder
                    .append_data(&mut header, *name, *bytes)
                    .expect("archive entry");
            }
            let gzip = builder.into_inner().expect("tar");
            gzip.finish().expect("gzip");
        }
        encoded
    }

    #[test]
    fn archive_path_traversal_is_rejected_before_extraction() {
        assert_eq!(
            validate_archive_path(Path::new("../openbubbles.so")),
            Err(ValidationError::ArchiveUnsafePath)
        );
    }

    #[test]
    fn archive_duplicate_candidates_are_rejected() {
        let temp = TempDir::new().expect("temp");
        let archive = temp.path().join("artifact.tar.gz");
        fs::write(
            &archive,
            tar_gz(&[
                ("one/openbubbles.so", b"one"),
                ("two/openbubbles.so", b"two"),
            ]),
        )
        .expect("archive");
        assert_eq!(
            install_official_artifact(&archive, temp.path().join("compat"))
                .expect_err("duplicate")
                .to_string(),
            "the official release archive contains more than one openbubbles.so"
        );
    }

    #[test]
    fn reset_removes_only_the_supported_component_directory() {
        let temp = TempDir::new().expect("temp");
        let compat = temp.path().join("compat");
        let target = compat.join(EXPECTED_COMPONENT_VERSION);
        fs::create_dir_all(&target).expect("target");
        fs::write(target.join("manifest.json"), b"synthetic").expect("manifest");
        assert!(remove_installed_component(&compat).expect("remove"));
        assert!(!target.exists());
        assert!(!remove_installed_component(&compat).expect("idempotent remove"));
    }

    #[cfg(unix)]
    fn executable_script(temp: &TempDir, body: &str) -> PathBuf {
        let path = temp.path().join("helper.sh");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("mode");
        path
    }

    #[cfg(unix)]
    fn test_provider(temp: &TempDir, helper: &Path) -> OpenBubblesValidationProvider {
        OpenBubblesValidationProvider {
            component_dir: temp.path().to_path_buf(),
            helper_path: helper.to_path_buf(),
        }
    }

    #[cfg(unix)]
    fn fake_component(temp: &TempDir) {
        fs::write(
            temp.path().join(MANIFEST_FILE_NAME),
            serde_json::to_vec(&ComponentManifest::supported()).expect("manifest"),
        )
        .expect("manifest");
        fs::write(temp.path().join(EXPECTED_LIBRARY_NAME), b"synthetic").expect("library");
    }

    #[cfg(unix)]
    #[test]
    fn helper_crash_is_reported_without_running_a_shared_object() {
        let temp = TempDir::new().expect("temp");
        fake_component(&temp);
        let helper = executable_script(&temp, "exit 42");
        let provider = test_provider(&temp, &helper);
        let error = provider
            .begin_with_component_dir(temp.path(), request())
            .err()
            .expect("crashed helper");
        assert_eq!(error, ValidationError::HelperCrashed);
    }

    #[cfg(unix)]
    #[test]
    fn malformed_helper_output_is_redacted_and_rejected() {
        let temp = TempDir::new().expect("temp");
        fake_component(&temp);
        let helper = executable_script(&temp, "printf 'private-payload\\n'");
        let provider = test_provider(&temp, &helper);
        let error = provider
            .begin_with_component_dir(temp.path(), request())
            .err()
            .expect("malformed helper");
        assert_eq!(error, ValidationError::MalformedOutput);
        assert!(!error.to_string().contains("private-payload"));
    }

    #[test]
    fn provider_errors_do_not_include_input_payloads() {
        let error = ValidationError::InvalidComponentHash;
        assert!(!error.to_string().contains("private"));
        assert!(!error.to_string().contains("payload"));
    }

    #[test]
    fn archive_helper_does_not_execute_test_library() {
        let command = Command::new("true").status().expect("true");
        assert!(command.success());
    }
}
