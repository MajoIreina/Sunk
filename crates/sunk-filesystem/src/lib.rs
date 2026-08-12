//! File inspection and recoverable disposal behind a platform-neutral boundary.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileItemType {
    File,
    Directory,
    WindowsShortcut,
    MacAlias,
    Application,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub item_type: FileItemType,
    pub size_bytes: u64,
    pub read_only: bool,
    pub is_symlink: bool,
}

/// The application-facing filesystem API deliberately has no permanent-delete method.
pub trait FileSystemService: Send + Sync {
    /// Inspects a path without following symbolic links.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is empty, missing, or inaccessible.
    fn inspect(&self, path: &Path) -> Result<FileInfo, FileSystemError>;

    /// Moves an existing path to the operating system's recoverable trash location.
    ///
    /// # Errors
    ///
    /// Returns an error when the path cannot be inspected or the platform trash operation fails.
    fn move_to_trash(&self, path: &Path) -> Result<(), FileSystemError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFileSystem;

impl LocalFileSystem {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl FileSystemService for LocalFileSystem {
    fn inspect(&self, path: &Path) -> Result<FileInfo, FileSystemError> {
        if path.as_os_str().is_empty() {
            return Err(FileSystemError::EmptyPath);
        }

        let metadata = fs::symlink_metadata(path).map_err(|source| FileSystemError::Io {
            operation: FileSystemOperation::Inspect,
            path: path.to_path_buf(),
            source,
        })?;

        Ok(FileInfo {
            path: path.to_path_buf(),
            item_type: classify(path, &metadata),
            size_bytes: metadata.len(),
            read_only: metadata.permissions().readonly(),
            is_symlink: metadata.file_type().is_symlink(),
        })
    }

    fn move_to_trash(&self, path: &Path) -> Result<(), FileSystemError> {
        if path.as_os_str().is_empty() {
            return Err(FileSystemError::EmptyPath);
        }

        // Inspect first so callers get a stable missing/inaccessible-path error.
        self.inspect(path)?;
        trash::delete(path).map_err(|source| FileSystemError::Trash {
            path: path.to_path_buf(),
            message: source.to_string(),
        })
    }
}

fn classify(path: &Path, metadata: &fs::Metadata) -> FileItemType {
    if metadata.file_type().is_symlink() {
        return FileItemType::Unknown;
    }

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);

    if metadata.is_dir() {
        if extension.as_deref() == Some("app") {
            FileItemType::Application
        } else {
            FileItemType::Directory
        }
    } else if metadata.is_file() {
        match extension.as_deref() {
            Some("lnk") => FileItemType::WindowsShortcut,
            Some("alias") => FileItemType::MacAlias,
            _ => FileItemType::File,
        }
    } else {
        FileItemType::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystemOperation {
    Inspect,
}

#[derive(Debug)]
pub enum FileSystemError {
    EmptyPath,
    Io {
        operation: FileSystemOperation,
        path: PathBuf,
        source: std::io::Error,
    },
    Trash {
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for FileSystemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("file path is empty"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "filesystem {operation:?} failed for '{}': {source}",
                path.display()
            ),
            Self::Trash { path, message } => {
                write!(
                    formatter,
                    "could not move '{}' to trash: {message}",
                    path.display()
                )
            }
        }
    }
}

impl Error for FileSystemError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::EmptyPath | Self::Trash { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOperation {
    MoveToTrash,
    PermanentDelete { confirmed_by_user: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermanentDeletePolicy {
    Disabled,
    RequireExplicitConfirmation,
}

/// Authorization policy only; permanent deletion is implemented outside this crate's safe API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileOperationPolicy {
    permanent_delete: PermanentDeletePolicy,
}

impl Default for FileOperationPolicy {
    fn default() -> Self {
        Self::trash_only()
    }
}

impl FileOperationPolicy {
    #[must_use]
    pub const fn trash_only() -> Self {
        Self {
            permanent_delete: PermanentDeletePolicy::Disabled,
        }
    }

    #[must_use]
    pub const fn allowing_confirmed_permanent_delete() -> Self {
        Self {
            permanent_delete: PermanentDeletePolicy::RequireExplicitConfirmation,
        }
    }

    #[must_use]
    pub const fn permanent_delete_policy(&self) -> PermanentDeletePolicy {
        self.permanent_delete
    }

    /// Checks whether an operation is permitted by the current policy.
    ///
    /// # Errors
    ///
    /// Returns an error when permanent deletion is disabled or lacks explicit confirmation.
    pub fn authorize(&self, operation: FileOperation) -> Result<(), PolicyViolation> {
        match operation {
            FileOperation::PermanentDelete { .. }
                if self.permanent_delete == PermanentDeletePolicy::Disabled =>
            {
                Err(PolicyViolation::PermanentDeleteDisabled)
            }
            FileOperation::PermanentDelete {
                confirmed_by_user: false,
            } => Err(PolicyViolation::ExplicitConfirmationRequired),
            FileOperation::MoveToTrash
            | FileOperation::PermanentDelete {
                confirmed_by_user: true,
            } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyViolation {
    PermanentDeleteDisabled,
    ExplicitConfirmationRequired,
}

impl fmt::Display for PolicyViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermanentDeleteDisabled => {
                formatter.write_str("permanent deletion is disabled by policy")
            }
            Self::ExplicitConfirmationRequired => {
                formatter.write_str("permanent deletion requires explicit user confirmation")
            }
        }
    }
}

impl Error for PolicyViolation {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_reports_a_known_repository_file() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let info = LocalFileSystem::new().inspect(&manifest).unwrap();

        assert_eq!(info.path, manifest);
        assert_eq!(info.item_type, FileItemType::File);
        assert!(!info.is_symlink);
        assert!(info.size_bytes > 0);
    }

    #[test]
    fn inspect_reports_a_known_repository_directory() {
        let source_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let info = LocalFileSystem::new().inspect(&source_directory).unwrap();

        assert_eq!(info.item_type, FileItemType::Directory);
        assert!(!info.is_symlink);
    }

    #[test]
    fn inspect_rejects_an_empty_path_without_side_effects() {
        let error = LocalFileSystem::new().inspect(Path::new("")).unwrap_err();
        assert!(matches!(error, FileSystemError::EmptyPath));
    }

    #[test]
    fn default_policy_allows_only_recoverable_disposal() {
        let policy = FileOperationPolicy::default();
        assert_eq!(policy.authorize(FileOperation::MoveToTrash), Ok(()));
        assert_eq!(
            policy.authorize(FileOperation::PermanentDelete {
                confirmed_by_user: true,
            }),
            Err(PolicyViolation::PermanentDeleteDisabled)
        );
    }

    #[test]
    fn permanent_delete_needs_policy_and_per_action_confirmation() {
        let policy = FileOperationPolicy::allowing_confirmed_permanent_delete();
        assert_eq!(
            policy.authorize(FileOperation::PermanentDelete {
                confirmed_by_user: false,
            }),
            Err(PolicyViolation::ExplicitConfirmationRequired)
        );
        assert_eq!(
            policy.authorize(FileOperation::PermanentDelete {
                confirmed_by_user: true,
            }),
            Ok(())
        );
    }
}
