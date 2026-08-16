//! Safe, auditable desktop file operations.
//!
//! Analysis never mutates the filesystem or starts a process. Destructive work
//! is exposed as an explicit worker command so the UI can require confirmation
//! before enqueueing it.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
};

const MIN_UNINSTALL_MATCH_SCORE: u32 = 100;
const WORKER_COMMAND_CAPACITY: usize = 512;
const DENIED_EXECUTABLE_HOSTS: &[&str] = &[
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
    "rundll32.exe",
    "regsvr32.exe",
    "explorer.exe",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DroppedItemKind {
    RegularFile,
    Directory,
    ApplicationShortcut,
    Executable,
    UnsupportedApplicationLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOperationErrorKind {
    #[allow(dead_code, reason = "constructed by the explicit non-Windows fallback")]
    UnsupportedPlatform,
    InvalidPath,
    NotFound,
    ProtectedPath,
    ReparsePoint,
    NotTrashable,
    ShortcutResolution,
    RegistryRead,
    NoUninstallCandidate,
    UnsafeUninstallCommand,
    Io,
    WorkerDisconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOperationError {
    pub kind: FileOperationErrorKind,
    pub message: String,
}

impl FileOperationError {
    fn new(kind: FileOperationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn unsupported() -> Self {
        Self::new(
            FileOperationErrorKind::UnsupportedPlatform,
            "file operations are supported only on Windows",
        )
    }
}

impl fmt::Display for FileOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FileOperationError {}

pub type FileOperationResultValue<T> = Result<T, FileOperationError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTrashTarget {
    pub path: PathBuf,
    pub kind: DroppedItemKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegistryHive {
    CurrentUser,
    LocalMachine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegistryView {
    Registry32,
    Registry64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallEntry {
    pub hive: RegistryHive,
    pub view: RegistryView,
    pub key_name: String,
    pub display_name: String,
    pub publisher: Option<String>,
    pub install_location: Option<PathBuf>,
    pub display_icon: Option<String>,
    pub uninstall_string: String,
    pub windows_installer: bool,
    pub system_component: bool,
    pub no_remove: bool,
    pub parent_key_name: Option<String>,
    pub release_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShortcut {
    pub shortcut_path: PathBuf,
    pub target_path: Option<PathBuf>,
    pub arguments: Option<String>,
    pub msi_product_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationIdentity {
    pub source_path: PathBuf,
    pub display_name_hint: String,
    pub target_executable: Option<PathBuf>,
    pub shortcut_arguments: Option<String>,
    pub msi_product_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchEvidence {
    MsiProductCode,
    ExactDisplayIcon,
    InsideInstallLocation,
    ExactDisplayName,
    ExactExecutableStem,
    ApplicationTarget(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallLaunchPlan {
    Msi {
        product_code: String,
    },
    Exe {
        executable: PathBuf,
        arguments: Vec<OsString>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallCandidate {
    pub entry: UninstallEntry,
    pub score: u32,
    pub evidence: Vec<MatchEvidence>,
    pub launch_plan: UninstallLaunchPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropAnalysis {
    Trashable(ValidatedTrashTarget),
    Uninstall(Box<UninstallCandidate>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperationCommand {
    AnalyzeDrop {
        request_id: u64,
        path: PathBuf,
    },
    MoveToTrash {
        request_id: u64,
        path: PathBuf,
    },
    LaunchUninstall {
        request_id: u64,
        candidate: Box<UninstallCandidate>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperationResult {
    DropAnalyzed {
        request_id: u64,
        result: FileOperationResultValue<DropAnalysis>,
    },
    MovedToTrash {
        request_id: u64,
        path: PathBuf,
        result: FileOperationResultValue<()>,
    },
    UninstallStarted {
        request_id: u64,
        result: FileOperationResultValue<u32>,
    },
}

enum WorkerMessage {
    Command(FileOperationCommand),
    Shutdown,
}

pub struct FileOperationsWorker {
    command_sender: SyncSender<WorkerMessage>,
    result_receiver: Receiver<FileOperationResult>,
    worker_thread: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl FileOperationsWorker {
    pub fn spawn() -> FileOperationResultValue<Self> {
        let (command_sender, command_receiver) = mpsc::sync_channel(WORKER_COMMAND_CAPACITY);
        let (result_sender, result_receiver) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_thread = thread::Builder::new()
            .name("sunk-file-operations".into())
            .spawn(move || worker_loop(&command_receiver, &result_sender, &worker_shutdown))
            .map_err(|error| {
                FileOperationError::new(
                    FileOperationErrorKind::Io,
                    format!("failed to start file-operation worker: {error}"),
                )
            })?;

        Ok(Self {
            command_sender,
            result_receiver,
            worker_thread: Some(worker_thread),
            shutdown,
        })
    }

    pub fn send(&self, command: FileOperationCommand) -> FileOperationResultValue<()> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(FileOperationError::new(
                FileOperationErrorKind::WorkerDisconnected,
                "file-operation worker is shutting down",
            ));
        }
        self.command_sender
            .try_send(WorkerMessage::Command(command))
            .map_err(|error| match error {
                TrySendError::Full(_) => FileOperationError::new(
                    FileOperationErrorKind::Io,
                    "file-operation queue is full",
                ),
                TrySendError::Disconnected(_) => FileOperationError::new(
                    FileOperationErrorKind::WorkerDisconnected,
                    "file-operation worker is no longer available",
                ),
            })
    }

    pub fn try_receive(&self) -> FileOperationResultValue<Option<FileOperationResult>> {
        match self.result_receiver.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(FileOperationError::new(
                FileOperationErrorKind::WorkerDisconnected,
                "file-operation worker stopped unexpectedly",
            )),
        }
    }
}

impl Drop for FileOperationsWorker {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.command_sender.try_send(WorkerMessage::Shutdown);
        // Dropping a JoinHandle detaches it. The cancellation flag prevents the
        // worker from starting another queued operation after this point, while
        // avoiding an unbounded UI-thread wait for an in-flight Shell call.
        let _ = self.worker_thread.take();
    }
}

fn worker_loop(
    command_receiver: &Receiver<WorkerMessage>,
    result_sender: &Sender<FileOperationResult>,
    shutdown: &AtomicBool,
) {
    while let Ok(message) = command_receiver.recv() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let WorkerMessage::Command(command) = message else {
            break;
        };
        let result = match command {
            FileOperationCommand::AnalyzeDrop { request_id, path } => {
                FileOperationResult::DropAnalyzed {
                    request_id,
                    result: analyze_drop(&path),
                }
            }
            FileOperationCommand::MoveToTrash { request_id, path } => {
                let result = move_to_recycle_bin(&path);
                FileOperationResult::MovedToTrash {
                    request_id,
                    path,
                    result,
                }
            }
            FileOperationCommand::LaunchUninstall {
                request_id,
                candidate,
            } => FileOperationResult::UninstallStarted {
                request_id,
                result: launch_uninstall(&candidate),
            },
        };
        if result_sender.send(result).is_err() {
            break;
        }
    }
}

pub fn classify_drop_shape(
    path: &Path,
    is_directory: bool,
    is_reparse_point: bool,
) -> FileOperationResultValue<DroppedItemKind> {
    if is_reparse_point {
        return Err(FileOperationError::new(
            FileOperationErrorKind::ReparsePoint,
            "reparse points and symbolic links are not accepted",
        ));
    }
    if is_directory {
        return Ok(DroppedItemKind::Directory);
    }

    let extension = path.extension().and_then(OsStr::to_str).unwrap_or_default();
    if extension.eq_ignore_ascii_case("lnk") {
        Ok(DroppedItemKind::ApplicationShortcut)
    } else if extension.eq_ignore_ascii_case("exe") {
        Ok(DroppedItemKind::Executable)
    } else if extension.eq_ignore_ascii_case("appref-ms")
        || extension.eq_ignore_ascii_case("url")
        || extension.eq_ignore_ascii_case("website")
    {
        Ok(DroppedItemKind::UnsupportedApplicationLink)
    } else {
        Ok(DroppedItemKind::RegularFile)
    }
}

pub fn normalize_windows_path(path: &Path) -> Option<String> {
    normalize_windows_path_text(&path.as_os_str().to_string_lossy())
}

fn normalize_windows_path_text(text: &str) -> Option<String> {
    let mut value = text.trim().replace('/', "\\");
    if let Some(stripped) = value.strip_prefix(r"\\?\UNC\") {
        value = format!(r"\\{stripped}");
    } else if let Some(stripped) = value.strip_prefix(r"\\?\") {
        value = stripped.to_owned();
    }
    if value.is_empty() || value.contains('\0') {
        return None;
    }

    while value.len() > 3 && value.ends_with('\\') {
        value.pop();
    }
    Some(value.to_lowercase())
}

pub fn path_is_within(path: &Path, directory: &Path) -> bool {
    let (Some(path), Some(directory)) = (
        normalize_windows_path(path),
        normalize_windows_path(directory),
    ) else {
        return false;
    };
    path == directory
        || path
            .strip_prefix(&directory)
            .is_some_and(|remainder| remainder.starts_with('\\'))
}

fn names_match(left: &str, right: &str) -> bool {
    normalize_display_name(left) == normalize_display_name(right)
}

fn normalize_display_name(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn path_stem_text(path: &Path) -> String {
    path.file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

pub fn analyze_drop(path: &Path) -> FileOperationResultValue<DropAnalysis> {
    let kind = classify_existing_path(path)?;
    match kind {
        DroppedItemKind::RegularFile | DroppedItemKind::Directory => {
            validate_trash_target(path).map(DropAnalysis::Trashable)
        }
        DroppedItemKind::ApplicationShortcut | DroppedItemKind::Executable => {
            let identity = application_identity(path, kind)?;
            let entries = enumerate_uninstall_entries()?;
            select_uninstall_candidate(&identity, &entries)
                .map(Box::new)
                .map(DropAnalysis::Uninstall)
        }
        DroppedItemKind::UnsupportedApplicationLink => Err(FileOperationError::new(
            FileOperationErrorKind::NotTrashable,
            "AppRef/MSIX links are not supported by the classic uninstaller backend",
        )),
    }
}

pub fn parse_display_icon_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let path = if let Some(remainder) = value.strip_prefix('"') {
        let closing_quote = remainder.find('"')?;
        &remainder[..closing_quote]
    } else if let Some((possible_path, possible_index)) = value.rsplit_once(',') {
        if possible_index.trim().parse::<i32>().is_ok() {
            possible_path.trim()
        } else {
            value
        }
    } else {
        value
    };
    (!path.is_empty()).then(|| PathBuf::from(path))
}

pub fn normalize_product_code(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() != 38 || !value.starts_with('{') || !value.ends_with('}') {
        return None;
    }
    let bytes = value.as_bytes();
    for &separator in &[9_usize, 14, 19, 24] {
        if bytes.get(separator) != Some(&b'-') {
            return None;
        }
    }
    if bytes[1..37].iter().enumerate().any(|(index, byte)| {
        let absolute = index + 1;
        !matches!(absolute, 9 | 14 | 19 | 24) && !byte.is_ascii_hexdigit()
    }) {
        return None;
    }
    Some(value.to_ascii_uppercase())
}

fn uninstall_entry_is_eligible(entry: &UninstallEntry) -> bool {
    if entry.display_name.trim().is_empty()
        || entry.uninstall_string.trim().is_empty()
        || entry.system_component
        || entry.no_remove
        || entry
            .parent_key_name
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return false;
    }

    !entry.release_type.as_deref().is_some_and(|release_type| {
        matches!(
            release_type.trim().to_ascii_lowercase().as_str(),
            "update" | "hotfix" | "security update"
        )
    })
}

#[derive(Debug)]
struct ScoredEntry<'a> {
    entry: &'a UninstallEntry,
    score: u32,
    evidence: Vec<MatchEvidence>,
    launch_plan: UninstallLaunchPlan,
}

fn score_uninstall_entry<'a>(
    identity: &ApplicationIdentity,
    entry: &'a UninstallEntry,
) -> Option<ScoredEntry<'a>> {
    if !uninstall_entry_is_eligible(entry) {
        return None;
    }

    let mut score = 0;
    let mut evidence = Vec::new();
    if let (Some(expected), Some(actual)) = (
        identity
            .msi_product_code
            .as_deref()
            .and_then(normalize_product_code),
        normalize_product_code(&entry.key_name),
    ) && expected == actual
    {
        score += 1_000;
        evidence.push(MatchEvidence::MsiProductCode);
    }

    let target = identity.target_executable.as_deref();
    if let (Some(target), Some(display_icon)) = (target, entry.display_icon.as_deref())
        && let Some(icon_path) = parse_display_icon_path(display_icon)
        && paths_equal(target, &icon_path)
    {
        score += 500;
        evidence.push(MatchEvidence::ExactDisplayIcon);
    }

    if let (Some(target), Some(install_location)) = (target, entry.install_location.as_deref())
        && install_location_is_specific(install_location)
        && path_is_within(target, install_location)
    {
        score += 300;
        evidence.push(MatchEvidence::InsideInstallLocation);
    }

    if names_match(&identity.display_name_hint, &entry.display_name) {
        score += 150;
        evidence.push(MatchEvidence::ExactDisplayName);
    }
    if let Some(target) = target
        && names_match(&path_stem_text(target), &entry.display_name)
    {
        score += 100;
        evidence.push(MatchEvidence::ExactExecutableStem);
    }

    // Arguments do not veto an otherwise eligible match. Browser-hosted and
    // alternate-mode shortcuts can therefore expose their registered host in
    // the confirmation dialog, where the user sees the exact uninstall source.
    if score < MIN_UNINSTALL_MATCH_SCORE {
        return None;
    }
    let launch_plan = build_launch_plan(entry, identity.msi_product_code.as_deref()).ok()?;
    if !launch_plan_is_bound_to_identity(identity, entry, &launch_plan) {
        return None;
    }
    if let Some(target) = target {
        evidence.push(MatchEvidence::ApplicationTarget(target.to_owned()));
    }
    Some(ScoredEntry {
        entry,
        score,
        evidence,
        launch_plan,
    })
}

pub fn select_uninstall_candidate(
    identity: &ApplicationIdentity,
    entries: &[UninstallEntry],
) -> FileOperationResultValue<UninstallCandidate> {
    let mut scored = entries
        .iter()
        .filter_map(|entry| score_uninstall_entry(identity, entry))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                registry_hive_rank(left.entry.hive).cmp(&registry_hive_rank(right.entry.hive))
            })
            .then_with(|| {
                registry_view_rank(left.entry.view).cmp(&registry_view_rank(right.entry.view))
            })
            .then_with(|| {
                left.entry
                    .display_name
                    .to_lowercase()
                    .cmp(&right.entry.display_name.to_lowercase())
            })
            .then_with(|| {
                left.entry
                    .key_name
                    .to_lowercase()
                    .cmp(&right.entry.key_name.to_lowercase())
            })
            .then_with(|| {
                left.entry
                    .uninstall_string
                    .to_lowercase()
                    .cmp(&right.entry.uninstall_string.to_lowercase())
            })
    });

    let Some(best) = scored.first() else {
        return Err(FileOperationError::new(
            FileOperationErrorKind::NoUninstallCandidate,
            "no uninstall entry matched the dropped application",
        ));
    };

    Ok(UninstallCandidate {
        entry: best.entry.clone(),
        score: best.score,
        evidence: best.evidence.clone(),
        launch_plan: best.launch_plan.clone(),
    })
}

const fn registry_hive_rank(hive: RegistryHive) -> u8 {
    match hive {
        RegistryHive::CurrentUser => 0,
        RegistryHive::LocalMachine => 1,
    }
}

const fn registry_view_rank(view: RegistryView) -> u8 {
    match view {
        RegistryView::Registry64 => 0,
        RegistryView::Registry32 => 1,
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    match (normalize_windows_path(left), normalize_windows_path(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn install_location_is_specific(path: &Path) -> bool {
    let Some(path) = normalize_windows_path(path) else {
        return false;
    };
    if !is_local_absolute_windows_path(&path) {
        return false;
    }
    path[3..]
        .split('\\')
        .filter(|component| !component.is_empty())
        .count()
        >= 2
}

pub fn build_launch_plan(
    entry: &UninstallEntry,
    matched_product_code: Option<&str>,
) -> FileOperationResultValue<UninstallLaunchPlan> {
    let entry_product_code = normalize_product_code(&entry.key_name);
    if let Some(matched_product_code) = matched_product_code.and_then(normalize_product_code) {
        if entry_product_code.as_ref() != Some(&matched_product_code) {
            return Err(unsafe_command(
                "advertised MSI product code does not match the uninstall registry key",
            ));
        }
        return Ok(UninstallLaunchPlan::Msi {
            product_code: matched_product_code,
        });
    }
    if entry.windows_installer
        && let Some(product_code) = entry_product_code
    {
        return Ok(UninstallLaunchPlan::Msi { product_code });
    }
    let plan = parse_uninstall_command(&entry.uninstall_string)?;
    if let UninstallLaunchPlan::Msi { product_code } = &plan
        && entry_product_code.as_ref() != Some(product_code)
    {
        return Err(unsafe_command(
            "MSI command product code does not match the uninstall registry key",
        ));
    }
    Ok(plan)
}

fn launch_plan_is_bound_to_identity(
    identity: &ApplicationIdentity,
    entry: &UninstallEntry,
    plan: &UninstallLaunchPlan,
) -> bool {
    match plan {
        UninstallLaunchPlan::Msi { product_code } => {
            normalize_product_code(&entry.key_name).as_ref() == Some(product_code)
                && identity
                    .msi_product_code
                    .as_deref()
                    .and_then(normalize_product_code)
                    .is_none_or(|advertised| advertised == product_code.as_str())
        }
        UninstallLaunchPlan::Exe { executable, .. } => {
            let executable_text = executable.as_os_str().to_string_lossy();
            validate_uninstall_executable_text(&executable_text).is_ok()
        }
    }
}

pub fn parse_uninstall_command(command: &str) -> FileOperationResultValue<UninstallLaunchPlan> {
    let arguments = parse_registered_uninstall_arguments(command)?;
    let Some(executable) = arguments.first() else {
        return Err(unsafe_command("uninstall command is empty"));
    };
    let executable_path = PathBuf::from(executable);
    let executable_name = executable_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();

    if executable_name.eq_ignore_ascii_case("msiexec.exe")
        || executable_name.eq_ignore_ascii_case("msiexec")
    {
        let product_code = extract_msi_product_code(&arguments[1..]).ok_or_else(|| {
            unsafe_command("MSI uninstall command does not contain one valid product code")
        })?;
        return Ok(UninstallLaunchPlan::Msi { product_code });
    }

    validate_uninstall_executable_text(executable)?;
    Ok(UninstallLaunchPlan::Exe {
        executable: executable_path,
        arguments: arguments[1..].iter().map(OsString::from).collect(),
    })
}

fn parse_registered_uninstall_arguments(command: &str) -> FileOperationResultValue<Vec<String>> {
    let parsed = parse_windows_command_line(command)?;
    let first_is_usable = parsed.first().is_some_and(|executable| {
        let file_name = Path::new(executable)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        file_name.eq_ignore_ascii_case("msiexec")
            || file_name.eq_ignore_ascii_case("msiexec.exe")
            || validate_uninstall_executable_text(executable).is_ok()
    });
    if first_is_usable {
        return Ok(parsed);
    }

    let Some((executable, argument_tail)) = unquoted_executable_prefix(command) else {
        return Ok(parsed);
    };
    let mut recovered = vec![executable.to_owned()];
    if !argument_tail.trim().is_empty() {
        // CommandLineToArgvW has no arguments-only mode. A fixed synthetic argv[0]
        // lets Windows apply its quoting and backslash rules to the real tail.
        let synthetic = format!(r#"sunk-argv.exe {}"#, argument_tail.trim_start());
        let mut tail = parse_windows_command_line(&synthetic)?;
        if !tail.is_empty() {
            tail.remove(0);
        }
        recovered.extend(tail);
    }
    Ok(recovered)
}

fn unquoted_executable_prefix(command: &str) -> Option<(&str, &str)> {
    let command = command.trim_start();
    if command.starts_with('"') {
        return None;
    }

    let bytes = command.as_bytes();
    for index in 0..bytes.len().saturating_sub(3) {
        if bytes[index..index + 4].eq_ignore_ascii_case(b".exe") {
            let end = index + 4;
            if end == bytes.len() || bytes[end].is_ascii_whitespace() {
                let executable = command[..end].trim_end();
                if is_local_absolute_windows_path(executable) {
                    return Some((executable, &command[end..]));
                }
            }
        }
    }
    None
}

fn unsafe_command(message: impl Into<String>) -> FileOperationError {
    FileOperationError::new(FileOperationErrorKind::UnsafeUninstallCommand, message)
}

fn validate_uninstall_executable_text(executable: &str) -> FileOperationResultValue<()> {
    if !is_local_absolute_windows_path(executable) {
        return Err(unsafe_command(
            "uninstaller must be an absolute path on a local drive",
        ));
    }
    if executable
        .replace('/', "\\")
        .split('\\')
        .any(|component| matches!(component, "." | ".."))
    {
        return Err(unsafe_command("uninstaller path contains dot components"));
    }

    let path = Path::new(executable);
    if !path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(unsafe_command("uninstaller must be an executable file"));
    }
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if DENIED_EXECUTABLE_HOSTS
        .iter()
        .any(|denied| file_name.eq_ignore_ascii_case(denied))
    {
        return Err(unsafe_command("shell and script hosts are not allowed"));
    }
    Ok(())
}

fn is_local_absolute_windows_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn extract_msi_product_code(arguments: &[String]) -> Option<String> {
    let mut product_codes = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let trimmed = argument.trim();
        let option = trimmed.get(..2).unwrap_or_default();
        if option.eq_ignore_ascii_case("/x") || option.eq_ignore_ascii_case("/i") {
            if let Some(attached) = trimmed.get(2..).filter(|value| !value.is_empty()) {
                if let Some(code) = normalize_product_code(attached) {
                    product_codes.push(code);
                }
            } else if let Some(code) = arguments
                .get(index + 1)
                .and_then(|value| normalize_product_code(value))
            {
                product_codes.push(code);
            }
        }
    }
    product_codes.sort_unstable();
    product_codes.dedup();
    (product_codes.len() == 1).then(|| product_codes.remove(0))
}

#[cfg(target_os = "windows")]
fn parse_windows_command_line(command: &str) -> FileOperationResultValue<Vec<String>> {
    use std::mem::MaybeUninit;

    use windows::{
        Win32::{
            Foundation::{HLOCAL, LocalFree},
            UI::Shell::CommandLineToArgvW,
        },
        core::PCWSTR,
    };

    if command.trim().is_empty() || command.contains('\0') {
        return Err(unsafe_command("uninstall command is empty or contains NUL"));
    }
    let command = command.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut argument_count = MaybeUninit::<i32>::uninit();
    // SAFETY: the command is NUL-terminated and `argument_count` is writable.
    let argument_pointer =
        unsafe { CommandLineToArgvW(PCWSTR(command.as_ptr()), argument_count.as_mut_ptr()) };
    if argument_pointer.is_null() {
        return Err(unsafe_command(
            "Windows could not parse the uninstall command",
        ));
    }

    // SAFETY: CommandLineToArgvW initialized the count and returned an array of that length.
    let argument_count = unsafe { argument_count.assume_init() };
    let arguments = if argument_count <= 0 {
        Err(unsafe_command(
            "Windows returned an empty uninstall command",
        ))
    } else {
        // SAFETY: the returned array remains valid until LocalFree below.
        let argument_count = usize::try_from(argument_count)
            .map_err(|_| unsafe_command("Windows returned an invalid argument count"))?;
        let argument_slice =
            unsafe { std::slice::from_raw_parts(argument_pointer, argument_count) };
        argument_slice
            .iter()
            .map(|argument| {
                // SAFETY: every PWSTR returned by CommandLineToArgvW is NUL-terminated.
                String::from_utf16(unsafe { argument.as_wide() })
                    .map_err(|_| unsafe_command("uninstall command contains invalid UTF-16"))
            })
            .collect()
    };
    // SAFETY: the allocation was returned by CommandLineToArgvW.
    unsafe {
        LocalFree(Some(HLOCAL(argument_pointer.cast())));
    }
    arguments
}

#[cfg(not(target_os = "windows"))]
fn parse_windows_command_line(command: &str) -> FileOperationResultValue<Vec<String>> {
    let _ = command;
    Err(FileOperationError::unsupported())
}

pub fn classify_existing_path(path: &Path) -> FileOperationResultValue<DroppedItemKind> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;

        let metadata =
            std::fs::symlink_metadata(path).map_err(|error| io_path_error(path, error))?;
        let is_reparse_point = metadata.file_attributes() & 0x400 != 0;
        classify_drop_shape(path, metadata.is_dir(), is_reparse_point)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err(FileOperationError::unsupported())
    }
}

pub fn validate_trash_target(path: &Path) -> FileOperationResultValue<ValidatedTrashTarget> {
    #[cfg(target_os = "windows")]
    {
        if !path.is_absolute() || !is_local_path(path) {
            return Err(FileOperationError::new(
                FileOperationErrorKind::InvalidPath,
                "only absolute paths on local drive letters can be moved to the recycle bin",
            ));
        }
        let kind = classify_existing_path(path)?;
        if !matches!(
            kind,
            DroppedItemKind::RegularFile | DroppedItemKind::Directory
        ) {
            return Err(FileOperationError::new(
                FileOperationErrorKind::NotTrashable,
                "application launchers must not be deleted as ordinary files",
            ));
        }
        let canonical = std::fs::canonicalize(path).map_err(|error| io_path_error(path, error))?;
        if !is_local_path(&canonical) {
            return Err(FileOperationError::new(
                FileOperationErrorKind::InvalidPath,
                "only paths on local drive letters can be moved to the recycle bin",
            ));
        }

        let restricted_subtrees = protected_subtrees()?;
        let protected_roots = protected_roots()?;
        let running_executable = std::env::current_exe()
            .map_err(|error| protection_resolution_error("running executable", error))?;
        let running_executable = std::fs::canonicalize(&running_executable)
            .map_err(|error| protection_resolution_error("running executable", error))?;
        if target_intersects_protected_paths(
            &canonical,
            &restricted_subtrees,
            &protected_roots,
            Some(&running_executable),
        ) {
            return Err(FileOperationError::new(
                FileOperationErrorKind::ProtectedPath,
                "the selected item intersects a protected system or application path",
            ));
        }

        Ok(ValidatedTrashTarget {
            path: canonical,
            kind,
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err(FileOperationError::unsupported())
    }
}

fn target_intersects_protected_paths(
    candidate: &Path,
    restricted_subtrees: &[PathBuf],
    protected_roots: &[PathBuf],
    running_executable: Option<&Path>,
) -> bool {
    is_drive_root(candidate)
        || restricted_subtrees.iter().any(|protected| {
            path_is_within(candidate, protected) || path_is_within(protected, candidate)
        })
        || protected_roots.iter().any(|protected| {
            paths_equal(candidate, protected) || path_is_within(protected, candidate)
        })
        || running_executable.is_some_and(|executable| {
            paths_equal(candidate, executable) || path_is_within(executable, candidate)
        })
}

fn is_drive_root(path: &Path) -> bool {
    normalize_windows_path(path).is_some_and(|path| {
        let bytes = path.as_bytes();
        bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
    })
}

#[cfg(target_os = "windows")]
fn is_local_path(path: &Path) -> bool {
    normalize_windows_path(path).is_some_and(|path| is_local_absolute_windows_path(&path))
}

#[cfg(target_os = "windows")]
fn protected_subtrees() -> FileOperationResultValue<Vec<PathBuf>> {
    windows_backend::system_protected_subtrees()
}

#[cfg(target_os = "windows")]
fn protected_roots() -> FileOperationResultValue<Vec<PathBuf>> {
    windows_backend::user_protected_roots()
}

#[cfg(target_os = "windows")]
fn protection_resolution_error(context: &str, error: impl fmt::Display) -> FileOperationError {
    FileOperationError::new(
        FileOperationErrorKind::ProtectedPath,
        format!("failed to resolve protected {context}: {error}"),
    )
}

pub fn move_to_recycle_bin(path: &Path) -> FileOperationResultValue<()> {
    #[cfg(target_os = "windows")]
    {
        let target = validate_trash_target(path)?;
        windows_backend::move_to_recycle_bin(&target.path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err(FileOperationError::unsupported())
    }
}

fn io_path_error(path: &Path, error: std::io::Error) -> FileOperationError {
    let kind = if error.kind() == std::io::ErrorKind::NotFound {
        FileOperationErrorKind::NotFound
    } else {
        FileOperationErrorKind::Io
    };
    FileOperationError::new(kind, format!("{}: {error}", path.display()))
}

fn application_identity(
    path: &Path,
    kind: DroppedItemKind,
) -> FileOperationResultValue<ApplicationIdentity> {
    match kind {
        DroppedItemKind::ApplicationShortcut => {
            let shortcut = resolve_shortcut(path)?;
            Ok(ApplicationIdentity {
                source_path: path.to_owned(),
                display_name_hint: path_stem_text(path),
                target_executable: shortcut.target_path,
                shortcut_arguments: shortcut.arguments,
                msi_product_code: shortcut.msi_product_code,
            })
        }
        DroppedItemKind::Executable => {
            let target = std::fs::canonicalize(path).map_err(|error| io_path_error(path, error))?;
            Ok(ApplicationIdentity {
                source_path: path.to_owned(),
                display_name_hint: path_stem_text(path),
                target_executable: Some(target),
                shortcut_arguments: None,
                msi_product_code: None,
            })
        }
        _ => Err(FileOperationError::new(
            FileOperationErrorKind::InvalidPath,
            "the dropped item is not an application launcher",
        )),
    }
}

pub fn resolve_shortcut(path: &Path) -> FileOperationResultValue<ResolvedShortcut> {
    #[cfg(target_os = "windows")]
    {
        windows_backend::resolve_shortcut(path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err(FileOperationError::unsupported())
    }
}

pub fn enumerate_uninstall_entries() -> FileOperationResultValue<Vec<UninstallEntry>> {
    #[cfg(target_os = "windows")]
    {
        windows_backend::enumerate_uninstall_entries()
    }

    #[cfg(not(target_os = "windows"))]
    {
        Err(FileOperationError::unsupported())
    }
}

pub fn launch_uninstall(candidate: &UninstallCandidate) -> FileOperationResultValue<u32> {
    #[cfg(target_os = "windows")]
    {
        windows_backend::launch_uninstall(candidate)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = candidate;
        Err(FileOperationError::unsupported())
    }
}

#[cfg(target_os = "windows")]
mod windows_backend {
    use std::{
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            fs::MetadataExt,
        },
        process::Command,
        ptr,
    };

    use windows::{
        Win32::{
            System::{
                ApplicationInstallationAndServicing::MsiGetShortcutTargetW,
                Com::{
                    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
                    CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
                },
                Environment::ExpandEnvironmentStringsW,
                SystemInformation::GetSystemDirectoryW,
            },
            UI::Shell::{
                FOF_NO_UI, FOFX_ADDUNDORECORD, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE,
                FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles,
                FOLDERID_ProgramFilesX64, FOLDERID_ProgramFilesX86, FOLDERID_Windows,
                FileOperation, IFileOperation, IShellItem, IShellLinkW, KF_FLAG_DEFAULT,
                SHCreateItemFromParsingName, SHGetKnownFolderPath, SLGP_RAWPATH, ShellLink,
            },
        },
        core::{GUID, Interface, PCWSTR, PWSTR},
    };
    use winreg::{
        HKCU, HKLM, RegKey,
        enums::{KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY},
    };

    use super::*;

    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    const MAX_WINDOWS_PATH: usize = 32_768;

    pub(super) fn system_protected_subtrees() -> FileOperationResultValue<Vec<PathBuf>> {
        [
            ("Windows", &FOLDERID_Windows),
            ("Program Files", &FOLDERID_ProgramFiles),
            ("Program Files x64", &FOLDERID_ProgramFilesX64),
            ("Program Files x86", &FOLDERID_ProgramFilesX86),
            ("ProgramData", &FOLDERID_ProgramData),
        ]
        .into_iter()
        .map(|(name, folder_id)| known_folder_path(name, folder_id))
        .collect()
    }

    pub(super) fn user_protected_roots() -> FileOperationResultValue<Vec<PathBuf>> {
        Ok(vec![known_folder_path("user profile", &FOLDERID_Profile)?])
    }

    fn known_folder_path(name: &str, folder_id: &GUID) -> FileOperationResultValue<PathBuf> {
        // SAFETY: the known-folder identifier is valid and the current process token is used.
        let raw = unsafe { SHGetKnownFolderPath(folder_id, KF_FLAG_DEFAULT, None) }
            .map_err(|error| protection_resolution_error(name, error))?;
        // SAFETY: SHGetKnownFolderPath returns a NUL-terminated CoTaskMem allocation.
        let path = PathBuf::from(OsString::from_wide(unsafe { raw.as_wide() }));
        // SAFETY: `raw` was allocated by SHGetKnownFolderPath and is freed exactly once.
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        let canonical = std::fs::canonicalize(&path)
            .map_err(|error| protection_resolution_error(name, error))?;
        if !is_local_path(&canonical) {
            return Err(protection_resolution_error(
                name,
                "known folder is not on a local drive",
            ));
        }
        Ok(canonical)
    }

    struct ComGuard;

    impl ComGuard {
        fn initialize(
            error_kind: FileOperationErrorKind,
            context: &'static str,
        ) -> FileOperationResultValue<Self> {
            // SAFETY: initialization and cleanup occur on the same worker thread.
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .map_err(|error| {
                    FileOperationError::new(
                        error_kind,
                        format!("failed to initialize COM for {context}: {error}"),
                    )
                })?;
            Ok(Self)
        }
    }

    pub(super) fn move_to_recycle_bin(path: &Path) -> FileOperationResultValue<()> {
        let _com = ComGuard::initialize(FileOperationErrorKind::Io, "recycle-bin operation")?;
        let wide_path = wide_null(path.as_os_str());
        let extended_prefix = ['\\' as u16, '\\' as u16, '?' as u16, '\\' as u16];
        let shell_path = if wide_path.starts_with(&extended_prefix) {
            &wide_path[extended_prefix.len()..]
        } else {
            &wide_path
        };

        // SAFETY: COM is initialized on this worker thread. The operation flags
        // explicitly require recycling and early failure; no permanent-delete
        // fallback flag is present.
        unsafe {
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(recycle_error)?;
            operation
                .SetOperationFlags(
                    FOF_NO_UI | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE | FOFX_ADDUNDORECORD,
                )
                .map_err(recycle_error)?;
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(shell_path.as_ptr()), None)
                .map_err(recycle_error)?;
            operation.DeleteItem(&item, None).map_err(recycle_error)?;
            operation.PerformOperations().map_err(recycle_error)?;
            if operation
                .GetAnyOperationsAborted()
                .map_err(recycle_error)?
                .as_bool()
            {
                return Err(FileOperationError::new(
                    FileOperationErrorKind::NotTrashable,
                    "Windows aborted the recycle-only operation",
                ));
            }
        }

        if path.exists() {
            return Err(FileOperationError::new(
                FileOperationErrorKind::NotTrashable,
                "Windows did not remove the source after the recycle-only operation",
            ));
        }
        Ok(())
    }

    fn recycle_error(error: windows::core::Error) -> FileOperationError {
        FileOperationError::new(
            FileOperationErrorKind::NotTrashable,
            format!("Windows recycle-only operation failed: {error}"),
        )
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            // SAFETY: paired with the successful initialization above.
            unsafe { CoUninitialize() };
        }
    }

    pub(super) fn resolve_shortcut(path: &Path) -> FileOperationResultValue<ResolvedShortcut> {
        if !path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            return Err(FileOperationError::new(
                FileOperationErrorKind::ShortcutResolution,
                "shortcut path does not have a .lnk extension",
            ));
        }
        let canonical = std::fs::canonicalize(path).map_err(|error| io_path_error(path, error))?;
        if !is_local_path(&canonical) {
            return Err(FileOperationError::new(
                FileOperationErrorKind::ShortcutResolution,
                "network shortcuts are not accepted",
            ));
        }

        let msi_product_code = advertised_msi_product_code(&canonical);
        let shell_result = read_shell_link(&canonical);
        let (target_path, arguments) = match shell_result {
            Ok((target, arguments)) => (target, arguments),
            Err(_) if msi_product_code.is_some() => (None, None),
            Err(error) => return Err(error),
        };
        if target_path.is_none() && msi_product_code.is_none() {
            return Err(FileOperationError::new(
                FileOperationErrorKind::ShortcutResolution,
                "shortcut contains neither an executable target nor an MSI product code",
            ));
        }

        Ok(ResolvedShortcut {
            shortcut_path: canonical,
            target_path,
            arguments,
            msi_product_code,
        })
    }

    fn advertised_msi_product_code(path: &Path) -> Option<String> {
        let shortcut = wide_null(path.as_os_str());
        let mut product = [0_u16; 39];
        let mut feature = [0_u16; 39];
        let mut component = [0_u16; 39];
        // SAFETY: all buffers are writable and the shortcut path is NUL-terminated.
        let result = unsafe {
            MsiGetShortcutTargetW(
                PCWSTR(shortcut.as_ptr()),
                Some(PWSTR(product.as_mut_ptr())),
                Some(PWSTR(feature.as_mut_ptr())),
                Some(PWSTR(component.as_mut_ptr())),
            )
        };
        (result == 0)
            .then(|| utf16_buffer_to_string(&product))
            .and_then(|code| normalize_product_code(&code))
    }

    fn read_shell_link(path: &Path) -> FileOperationResultValue<(Option<PathBuf>, Option<String>)> {
        let _com = ComGuard::initialize(
            FileOperationErrorKind::ShortcutResolution,
            "shortcut resolution",
        )?;
        // SAFETY: COM is initialized for this thread and ShellLink is an in-process server.
        let shell_link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                .map_err(shortcut_error)?;
        let persist: IPersistFile = shell_link.cast().map_err(shortcut_error)?;
        let shortcut = wide_null(path.as_os_str());
        // SAFETY: the path is NUL-terminated and the link is opened read-only.
        unsafe { persist.Load(PCWSTR(shortcut.as_ptr()), STGM_READ) }.map_err(shortcut_error)?;

        let mut target = vec![0_u16; MAX_WINDOWS_PATH];
        // SAFETY: the output buffer is writable; no resolve/search/update operation is invoked.
        unsafe { shell_link.GetPath(&mut target, ptr::null_mut(), SLGP_RAWPATH.0 as u32) }
            .map_err(shortcut_error)?;
        let target = utf16_buffer_to_string(&target);
        let target_path = if target.trim().is_empty() {
            None
        } else {
            let expanded = expand_environment(&target);
            let path = PathBuf::from(expanded);
            let metadata =
                std::fs::symlink_metadata(&path).map_err(|error| io_path_error(&path, error))?;
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err(FileOperationError::new(
                    FileOperationErrorKind::ShortcutResolution,
                    "shortcut target is missing, not a file, or is a reparse point",
                ));
            }
            let canonical =
                std::fs::canonicalize(&path).map_err(|error| io_path_error(&path, error))?;
            if !is_local_path(&canonical)
                || !canonical
                    .extension()
                    .and_then(OsStr::to_str)
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            {
                return Err(FileOperationError::new(
                    FileOperationErrorKind::ShortcutResolution,
                    "shortcut target is not a local executable",
                ));
            }
            Some(canonical)
        };

        let mut arguments = vec![0_u16; MAX_WINDOWS_PATH];
        // SAFETY: the output buffer is writable for the duration of the call.
        unsafe { shell_link.GetArguments(&mut arguments) }.map_err(shortcut_error)?;
        let arguments = utf16_buffer_to_string(&arguments);
        let arguments = (!arguments.trim().is_empty()).then_some(arguments);
        Ok((target_path, arguments))
    }

    fn shortcut_error(error: windows::core::Error) -> FileOperationError {
        FileOperationError::new(
            FileOperationErrorKind::ShortcutResolution,
            format!("failed to read shortcut: {error}"),
        )
    }

    pub(super) fn enumerate_uninstall_entries() -> FileOperationResultValue<Vec<UninstallEntry>> {
        let mut entries = Vec::new();
        for (hive, root) in [
            (RegistryHive::CurrentUser, HKCU),
            (RegistryHive::LocalMachine, HKLM),
        ] {
            for (view, view_flag) in [
                (RegistryView::Registry64, KEY_WOW64_64KEY),
                (RegistryView::Registry32, KEY_WOW64_32KEY),
            ] {
                let uninstall =
                    match root.open_subkey_with_flags(UNINSTALL_KEY, KEY_READ | view_flag) {
                        Ok(key) => key,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            return Err(FileOperationError::new(
                                FileOperationErrorKind::RegistryRead,
                                format!("failed to open uninstall registry view: {error}"),
                            ));
                        }
                    };
                append_registry_entries(&mut entries, &uninstall, hive, view, view_flag);
            }
        }
        Ok(entries)
    }

    fn append_registry_entries(
        entries: &mut Vec<UninstallEntry>,
        uninstall: &RegKey,
        hive: RegistryHive,
        view: RegistryView,
        view_flag: u32,
    ) {
        for key_name in uninstall.enum_keys().filter_map(Result::ok) {
            let Ok(key) = uninstall.open_subkey_with_flags(&key_name, KEY_READ | view_flag) else {
                continue;
            };
            let Some(display_name) = registry_string(&key, "DisplayName") else {
                continue;
            };
            let Some(uninstall_string) = registry_string(&key, "UninstallString") else {
                continue;
            };
            entries.push(UninstallEntry {
                hive,
                view,
                key_name,
                display_name,
                publisher: registry_string(&key, "Publisher"),
                install_location: registry_string(&key, "InstallLocation")
                    .and_then(|value| canonical_install_location(&expand_environment(&value))),
                display_icon: registry_string(&key, "DisplayIcon")
                    .map(|value| expand_environment(&value)),
                uninstall_string: expand_environment(&uninstall_string),
                windows_installer: registry_dword(&key, "WindowsInstaller") == Some(1),
                system_component: registry_dword(&key, "SystemComponent") == Some(1),
                no_remove: registry_dword(&key, "NoRemove") == Some(1),
                parent_key_name: registry_string(&key, "ParentKeyName"),
                release_type: registry_string(&key, "ReleaseType"),
            });
        }
    }

    fn registry_string(key: &RegKey, name: &str) -> Option<String> {
        key.get_value::<String, _>(name)
            .ok()
            .map(|value| value.trim_matches('\0').trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn registry_dword(key: &RegKey, name: &str) -> Option<u32> {
        key.get_value(name).ok()
    }

    fn canonical_install_location(value: &str) -> Option<PathBuf> {
        let path = PathBuf::from(value);
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return None;
        }
        let canonical = std::fs::canonicalize(path).ok()?;
        is_local_path(&canonical).then_some(canonical)
    }

    fn expand_environment(value: &str) -> String {
        let source = wide_null(OsStr::new(value));
        // SAFETY: the source is NUL-terminated; a null destination requests the size.
        let required = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), None) };
        if required == 0 || required as usize > MAX_WINDOWS_PATH {
            return value.to_owned();
        }
        let mut destination = vec![0_u16; required as usize];
        // SAFETY: the destination has exactly the size returned by the first call.
        let written =
            unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(&mut destination)) };
        if written == 0 || written > required {
            value.to_owned()
        } else {
            utf16_buffer_to_string(&destination)
        }
    }

    pub(super) fn launch_uninstall(
        candidate: &UninstallCandidate,
    ) -> FileOperationResultValue<u32> {
        let mut command = match &candidate.launch_plan {
            UninstallLaunchPlan::Msi { product_code } => {
                let product_code = normalize_product_code(product_code).ok_or_else(|| {
                    unsafe_command("MSI launch plan contains an invalid product code")
                })?;
                if normalize_product_code(&candidate.entry.key_name).as_ref() != Some(&product_code)
                {
                    return Err(unsafe_command(
                        "MSI launch plan is not bound to the candidate registry product code",
                    ));
                }
                let mut command = Command::new(system_msiexec_path()?);
                command.arg("/x").arg(product_code);
                command
            }
            UninstallLaunchPlan::Exe {
                executable,
                arguments,
            } => {
                let executable_text = executable.as_os_str().to_string_lossy();
                validate_uninstall_executable_text(&executable_text)?;
                let metadata = std::fs::symlink_metadata(executable)
                    .map_err(|error| io_path_error(executable, error))?;
                if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                    return Err(unsafe_command(
                        "uninstaller is missing, not a file, or is a reparse point",
                    ));
                }
                let canonical = std::fs::canonicalize(executable)
                    .map_err(|error| io_path_error(executable, error))?;
                if !is_local_path(&canonical) {
                    return Err(unsafe_command("uninstaller is not on a local drive"));
                }
                let mut command = Command::new(&canonical);
                command.args(arguments);
                if let Some(parent) = canonical.parent() {
                    command.current_dir(parent);
                }
                command
            }
        };
        command.spawn().map(|child| child.id()).map_err(|error| {
            FileOperationError::new(
                FileOperationErrorKind::Io,
                format!("failed to start uninstaller: {error}"),
            )
        })
    }

    fn system_msiexec_path() -> FileOperationResultValue<PathBuf> {
        let mut buffer = vec![0_u16; MAX_WINDOWS_PATH];
        // SAFETY: the output buffer is writable for the duration of the call.
        let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
        if length == 0 || length >= buffer.len() {
            return Err(FileOperationError::new(
                FileOperationErrorKind::Io,
                "failed to locate the Windows system directory",
            ));
        }
        let mut path = PathBuf::from(OsString::from_wide(&buffer[..length]));
        path.push("msiexec.exe");
        Ok(path)
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn utf16_buffer_to_string(buffer: &[u16]) -> String {
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..length])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCT_CODE: &str = "{12345678-1234-ABCD-9876-1234567890AB}";

    fn uninstall_entry(
        key_name: &str,
        display_name: &str,
        install_location: Option<&str>,
        display_icon: Option<&str>,
        uninstall_string: &str,
    ) -> UninstallEntry {
        UninstallEntry {
            hive: RegistryHive::LocalMachine,
            view: RegistryView::Registry64,
            key_name: key_name.into(),
            display_name: display_name.into(),
            publisher: Some("Example Publisher".into()),
            install_location: install_location.map(PathBuf::from),
            display_icon: display_icon.map(str::to_owned),
            uninstall_string: uninstall_string.into(),
            windows_installer: false,
            system_component: false,
            no_remove: false,
            parent_key_name: None,
            release_type: None,
        }
    }

    fn application_identity() -> ApplicationIdentity {
        ApplicationIdentity {
            source_path: PathBuf::from(r"C:\Users\Alice\Desktop\Nova.lnk"),
            display_name_hint: "Nova".into(),
            target_executable: Some(PathBuf::from(r"C:\Program Files\Nova\nova.exe")),
            shortcut_arguments: None,
            msi_product_code: None,
        }
    }

    #[test]
    fn classifies_application_launchers_before_regular_files() {
        assert_eq!(
            classify_drop_shape(Path::new("Example.LNK"), false, false).unwrap(),
            DroppedItemKind::ApplicationShortcut
        );
        assert_eq!(
            classify_drop_shape(Path::new("Example.ExE"), false, false).unwrap(),
            DroppedItemKind::Executable
        );
        assert_eq!(
            classify_drop_shape(Path::new("Example.txt"), false, false).unwrap(),
            DroppedItemKind::RegularFile
        );
        for application_link in ["Example.url", "Example.WEBSITE", "Example.appref-ms"] {
            assert_eq!(
                classify_drop_shape(Path::new(application_link), false, false).unwrap(),
                DroppedItemKind::UnsupportedApplicationLink
            );
        }
        assert_eq!(
            classify_drop_shape(Path::new("folder.exe"), true, false).unwrap(),
            DroppedItemKind::Directory
        );
        assert_eq!(
            classify_drop_shape(Path::new("linked"), true, true)
                .unwrap_err()
                .kind,
            FileOperationErrorKind::ReparsePoint
        );
    }

    #[test]
    fn windows_path_comparison_observes_component_boundaries() {
        assert!(path_is_within(
            Path::new(r"C:\Program Files\Nova\nova.exe"),
            Path::new(r"c:/program files/NOVA")
        ));
        assert!(!path_is_within(
            Path::new(r"C:\Program Files\NovaPlus\nova.exe"),
            Path::new(r"C:\Program Files\Nova")
        ));
    }

    #[test]
    fn protection_rejects_roots_system_trees_and_running_application() {
        let restricted = vec![
            PathBuf::from(r"C:\Windows"),
            PathBuf::from(r"C:\Program Files"),
        ];
        let roots = vec![PathBuf::from(r"C:\Users\Alice")];
        let executable = Path::new(r"C:\Apps\Sunk\sunk.exe");

        assert!(target_intersects_protected_paths(
            Path::new(r"C:\"),
            &restricted,
            &roots,
            Some(executable)
        ));
        assert!(target_intersects_protected_paths(
            Path::new(r"C:\Windows\Temp\data.bin"),
            &restricted,
            &roots,
            Some(executable)
        ));
        assert!(target_intersects_protected_paths(
            Path::new(r"C:\Users\Alice"),
            &restricted,
            &roots,
            Some(executable)
        ));
        assert!(target_intersects_protected_paths(
            Path::new(r"C:\Apps\Sunk"),
            &restricted,
            &roots,
            Some(executable)
        ));
        assert!(!target_intersects_protected_paths(
            Path::new(r"C:\Users\Alice\Desktop\notes.txt"),
            &restricted,
            &roots,
            Some(executable)
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "manual smoke test moves one test-owned temporary file to the recycle bin"]
    fn real_recycle_bin_smoke_uses_only_a_test_owned_file() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sunk-recycle-smoke-{}-{nonce}.tmp",
            std::process::id()
        ));
        std::fs::write(&path, b"Sunk recycle-bin smoke test\n")
            .expect("test-owned temporary file should be created");

        move_to_recycle_bin(&path).expect("test-owned file should move to the recycle bin");
        assert!(
            !path.exists(),
            "recycled test file should leave its source path"
        );
    }

    #[test]
    fn product_codes_are_strict_and_normalized() {
        assert_eq!(
            normalize_product_code(PRODUCT_CODE).as_deref(),
            Some(PRODUCT_CODE)
        );
        assert_eq!(
            normalize_product_code("{12345678-1234-abcd-9876-1234567890ab}").as_deref(),
            Some(PRODUCT_CODE)
        );
        assert!(normalize_product_code("12345678-1234-ABCD-9876-1234567890AB").is_none());
        assert!(normalize_product_code("{12345678-1234-ABCD-9876-1234567890ZZ}").is_none());
    }

    #[test]
    fn display_icon_parser_removes_only_numeric_resource_index() {
        assert_eq!(
            parse_display_icon_path(r#""C:\Program Files\Nova\nova.exe", 2"#),
            Some(PathBuf::from(r"C:\Program Files\Nova\nova.exe"))
        );
        assert_eq!(
            parse_display_icon_path(r"C:\Nova\nova.exe,-1"),
            Some(PathBuf::from(r"C:\Nova\nova.exe"))
        );
    }

    #[test]
    fn candidate_selection_prefers_the_strongest_match() {
        let entries = vec![
            uninstall_entry(
                "Nova",
                "Nova",
                Some(r"C:\Program Files\Nova"),
                Some(r#""C:\Program Files\Nova\nova.exe",0"#),
                r#""C:\Program Files\Nova\uninstall.exe" /remove"#,
            ),
            uninstall_entry(
                "Other",
                "Other App",
                Some(r"C:\Program Files\Other"),
                None,
                r#""C:\Program Files\Other\uninstall.exe""#,
            ),
        ];
        let candidate = select_uninstall_candidate(&application_identity(), &entries).unwrap();
        assert_eq!(candidate.entry.display_name, "Nova");
        assert!(candidate.score >= MIN_UNINSTALL_MATCH_SCORE);
        assert!(
            candidate
                .evidence
                .contains(&MatchEvidence::ExactDisplayIcon)
        );
        assert!(candidate.evidence.iter().any(|evidence| matches!(
            evidence,
            MatchEvidence::ApplicationTarget(path)
                if path == Path::new(r"C:\Program Files\Nova\nova.exe")
        )));
    }

    #[test]
    fn tied_distinct_candidates_use_a_stable_registry_order() {
        let mut second = uninstall_entry(
            "NovaTwo",
            "Nova",
            Some(r"C:\Program Files\Nova"),
            Some(r"C:\Program Files\Nova\nova.exe,0"),
            r#""C:\Program Files\Nova\remove-two.exe""#,
        );
        second.publisher = Some("Different Publisher".into());
        let entries = vec![
            uninstall_entry(
                "NovaOne",
                "Nova",
                Some(r"C:\Program Files\Nova"),
                Some(r"C:\Program Files\Nova\nova.exe,0"),
                r#""C:\Program Files\Nova\remove-one.exe""#,
            ),
            second,
        ];
        let candidate = select_uninstall_candidate(&application_identity(), &entries).unwrap();
        assert_eq!(candidate.entry.key_name, "NovaOne");
    }

    #[test]
    fn shortcut_arguments_leave_the_registered_target_to_user_confirmation() {
        let identity = ApplicationIdentity {
            source_path: PathBuf::from(r"C:\Users\Alice\Desktop\YouTube.lnk"),
            display_name_hint: "YouTube".into(),
            target_executable: Some(PathBuf::from(
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            )),
            shortcut_arguments: Some("--app-id=example".into()),
            msi_product_code: None,
        };
        let entries = vec![uninstall_entry(
            "Chrome",
            "Google Chrome",
            Some(r"C:\Program Files\Google\Chrome"),
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe,0"),
            r#""C:\Program Files\Google\Chrome\Application\setup.exe" --uninstall"#,
        )];
        let candidate = select_uninstall_candidate(&identity, &entries).unwrap();
        assert_eq!(candidate.entry.display_name, "Google Chrome");
        assert!(candidate.score >= MIN_UNINSTALL_MATCH_SCORE);
    }

    #[test]
    fn advertised_msi_product_code_is_definitive() {
        let mut identity = application_identity();
        identity.target_executable = None;
        identity.msi_product_code = Some(PRODUCT_CODE.into());
        let mut entry = uninstall_entry(
            PRODUCT_CODE,
            "Nova MSI",
            None,
            None,
            "MsiExec.exe /I{12345678-1234-ABCD-9876-1234567890AB}",
        );
        entry.windows_installer = true;
        let candidate = select_uninstall_candidate(&identity, &[entry]).unwrap();
        assert_eq!(
            candidate.launch_plan,
            UninstallLaunchPlan::Msi {
                product_code: PRODUCT_CODE.into()
            }
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn msi_command_product_code_must_match_registry_key() {
        const OTHER_PRODUCT_CODE: &str = "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}";
        let entry = uninstall_entry(
            PRODUCT_CODE,
            "Nova MSI",
            None,
            None,
            &format!("MsiExec.exe /x{OTHER_PRODUCT_CODE}"),
        );

        assert_eq!(
            build_launch_plan(&entry, None).unwrap_err().kind,
            FileOperationErrorKind::UnsafeUninstallCommand
        );
    }

    #[test]
    fn registered_exe_uninstaller_may_live_outside_the_install_tree() {
        let identity = application_identity();
        let entry = uninstall_entry(
            "Nova",
            "Nova",
            Some(r"C:\Program Files\Nova"),
            Some(r"C:\Program Files\Nova\nova.exe,0"),
            r#""C:\Program Files\Nova\uninstall.exe""#,
        );
        let inside = UninstallLaunchPlan::Exe {
            executable: PathBuf::from(r"C:\Program Files\Nova\uninstall.exe"),
            arguments: Vec::new(),
        };
        let outside = UninstallLaunchPlan::Exe {
            executable: PathBuf::from(r"C:\Users\Alice\uninstall.exe"),
            arguments: Vec::new(),
        };

        assert!(launch_plan_is_bound_to_identity(&identity, &entry, &inside));
        assert!(launch_plan_is_bound_to_identity(
            &identity, &entry, &outside
        ));
    }

    #[test]
    fn exact_executable_stem_is_enough_for_user_confirmation() {
        let identity = ApplicationIdentity {
            source_path: PathBuf::from(r"C:\Users\Alice\Desktop\Shortcut.lnk"),
            display_name_hint: "Shortcut".into(),
            target_executable: Some(PathBuf::from(r"C:\Vendor\Nova.exe")),
            shortcut_arguments: None,
            msi_product_code: None,
        };
        let entry = uninstall_entry(
            "NovaEntry",
            "Nova",
            None,
            None,
            r#""C:\Vendor Tools\uninstall.exe" /remove"#,
        );

        let candidate = select_uninstall_candidate(&identity, &[entry]).unwrap();

        assert_eq!(candidate.score, MIN_UNINSTALL_MATCH_SCORE);
        assert!(
            candidate
                .evidence
                .contains(&MatchEvidence::ExactExecutableStem)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn system_parser_preserves_quoted_executable_and_arguments() {
        let plan =
            parse_uninstall_command(r#""C:\Program Files\Nova\uninstall.exe" /remove "user data""#)
                .unwrap();
        assert_eq!(
            plan,
            UninstallLaunchPlan::Exe {
                executable: PathBuf::from(r"C:\Program Files\Nova\uninstall.exe"),
                arguments: vec![OsString::from("/remove"), OsString::from("user data")],
            }
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn command_parser_normalizes_msi_maintenance_to_uninstall_plan() {
        assert_eq!(
            parse_uninstall_command(&format!("MsiExec.exe /I{PRODUCT_CODE}")).unwrap(),
            UninstallLaunchPlan::Msi {
                product_code: PRODUCT_CODE.into()
            }
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn command_parser_rejects_shell_hosts_and_recovers_unquoted_space_paths() {
        assert_eq!(
            parse_uninstall_command(r#"C:\Windows\System32\cmd.exe /c erase C:\data"#)
                .unwrap_err()
                .kind,
            FileOperationErrorKind::UnsafeUninstallCommand
        );
        assert_eq!(
            parse_uninstall_command(r#"C:\Program Files\Nova\uninstall.exe /remove "user data""#)
                .unwrap(),
            UninstallLaunchPlan::Exe {
                executable: PathBuf::from(r"C:\Program Files\Nova\uninstall.exe"),
                arguments: vec![OsString::from("/remove"), OsString::from("user data")],
            }
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_command_parsing_is_explicitly_unsupported() {
        assert_eq!(
            parse_uninstall_command(r"C:\App\uninstall.exe")
                .unwrap_err()
                .kind,
            FileOperationErrorKind::UnsupportedPlatform
        );
    }
}
