//! Cross-platform desktop window defaults and normalized input events.

use std::{collections::HashSet, path::PathBuf};

use winit::{
    dpi::LogicalSize,
    window::{Window, WindowAttributes, WindowLevel},
};

/// Window behavior shared by the Windows and macOS prototypes.
#[allow(
    clippy::struct_excessive_bools,
    reason = "these booleans map directly to independent native window flags"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopWindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub always_on_top: bool,
    pub transparent: bool,
    pub decorations: bool,
    pub resizable: bool,
}

impl Default for DesktopWindowConfig {
    fn default() -> Self {
        Self {
            title: "Sunk".to_owned(),
            width: 640,
            height: 640,
            always_on_top: true,
            transparent: true,
            decorations: false,
            resizable: true,
        }
    }
}

impl DesktopWindowConfig {
    #[must_use]
    pub fn attributes(&self) -> WindowAttributes {
        let level = if self.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };

        let attributes = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(self.width, self.height))
            .with_min_inner_size(LogicalSize::new(256_u32, 256_u32))
            .with_transparent(self.transparent)
            .with_decorations(self.decorations)
            .with_resizable(self.resizable)
            .with_window_level(level);

        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_no_redirection_bitmap(self.transparent)
        }

        #[cfg(not(target_os = "windows"))]
        attributes
    }
}

/// Platform drag-and-drop events normalized before they enter the core layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopInputEvent {
    DragEntered(PathBuf),
    DragLeft,
    Dropped(PathBuf),
}

/// File drag-and-drop events normalized from winit before aggregation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeFileDropEvent {
    Hovered(PathBuf),
    Dropped(PathBuf),
    Cancelled,
}

/// Current phase of a native file drag session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DragBatchState {
    #[default]
    Idle,
    Hovering,
    Dropping,
}

/// Observable changes produced while accumulating one drag session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragBatchUpdate {
    None,
    HoverStarted { count: usize },
    HoverChanged { count: usize },
    DropPending { count: usize },
    Cancelled,
    Rejected(FileDropRejection),
}

/// Structural input errors detected without inspecting the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDropRejection {
    EmptyPath,
}

/// A non-empty, de-duplicated set of files committed by one native drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDropBatch {
    paths: Vec<PathBuf>,
}

impl FileDropBatch {
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    #[must_use]
    pub fn into_paths(self) -> Vec<PathBuf> {
        self.paths
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.paths.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Collects the per-path events emitted by winit into one ordered drop batch.
///
/// This type performs no path resolution, metadata access, or file operation. Call
/// [`Self::flush_dropped`] after the current event-loop queue has been drained so every
/// `DroppedFile` event from the same native drop is included in one batch.
#[derive(Debug, Default)]
pub struct FileDropAggregator {
    state: DragBatchState,
    hovered_paths: Vec<PathBuf>,
    hovered_seen: HashSet<PathBuf>,
    dropped_paths: Vec<PathBuf>,
    dropped_seen: HashSet<PathBuf>,
}

impl FileDropAggregator {
    #[must_use]
    pub fn state(&self) -> DragBatchState {
        self.state
    }

    #[must_use]
    pub fn hovered_paths(&self) -> &[PathBuf] {
        &self.hovered_paths
    }

    #[must_use]
    pub fn pending_drop_paths(&self) -> &[PathBuf] {
        &self.dropped_paths
    }

    /// Accumulates one native event without touching the referenced path.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "native events transfer ownership of their path into the aggregator"
    )]
    pub fn push(&mut self, event: NativeFileDropEvent) -> DragBatchUpdate {
        match event {
            NativeFileDropEvent::Hovered(path) => self.push_hovered(path),
            NativeFileDropEvent::Dropped(path) => self.push_dropped(path),
            NativeFileDropEvent::Cancelled => self.cancel(),
        }
    }

    /// Clears the current drag session and any unflushed drop.
    pub fn cancel(&mut self) -> DragBatchUpdate {
        if self.state == DragBatchState::Idle {
            return DragBatchUpdate::None;
        }

        self.clear();
        DragBatchUpdate::Cancelled
    }

    /// Takes the completed drop exactly once while preserving first-seen path order.
    #[must_use]
    pub fn flush_dropped(&mut self) -> Option<FileDropBatch> {
        if self.dropped_paths.is_empty() {
            return None;
        }

        let paths = std::mem::take(&mut self.dropped_paths);
        self.clear();
        Some(FileDropBatch { paths })
    }

    fn push_hovered(&mut self, path: PathBuf) -> DragBatchUpdate {
        if path.as_os_str().is_empty() {
            return DragBatchUpdate::Rejected(FileDropRejection::EmptyPath);
        }

        // A pending drop is authoritative until the app flushes it at the event-loop boundary.
        if self.state == DragBatchState::Dropping {
            return DragBatchUpdate::DropPending {
                count: self.dropped_paths.len(),
            };
        }

        if !self.hovered_seen.insert(path.clone()) {
            return DragBatchUpdate::None;
        }

        self.hovered_paths.push(path);
        let count = self.hovered_paths.len();
        let update = if self.state == DragBatchState::Idle {
            DragBatchUpdate::HoverStarted { count }
        } else {
            DragBatchUpdate::HoverChanged { count }
        };
        self.state = DragBatchState::Hovering;
        update
    }

    fn push_dropped(&mut self, path: PathBuf) -> DragBatchUpdate {
        if path.as_os_str().is_empty() {
            return DragBatchUpdate::Rejected(FileDropRejection::EmptyPath);
        }

        self.state = DragBatchState::Dropping;
        if self.dropped_seen.insert(path.clone()) {
            self.dropped_paths.push(path);
        }
        DragBatchUpdate::DropPending {
            count: self.dropped_paths.len(),
        }
    }

    fn clear(&mut self) {
        self.state = DragBatchState::Idle;
        self.hovered_paths.clear();
        self.hovered_seen.clear();
        self.dropped_paths.clear();
        self.dropped_seen.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn path(name: &str) -> PathBuf {
        Path::new("virtual-drop-targets").join(name)
    }

    #[test]
    fn defaults_match_the_phase_zero_window_contract() {
        let config = DesktopWindowConfig::default();
        assert!(config.transparent);
        assert!(config.always_on_top);
        assert!(!config.decorations);
        assert_eq!((config.width, config.height), (640, 640));
    }

    #[test]
    fn multiple_files_flush_as_one_ordered_batch() {
        let first = path("first.txt");
        let second = path("second.txt");
        let mut aggregator = FileDropAggregator::default();

        assert_eq!(
            aggregator.push(NativeFileDropEvent::Hovered(first.clone())),
            DragBatchUpdate::HoverStarted { count: 1 }
        );
        assert_eq!(
            aggregator.push(NativeFileDropEvent::Hovered(second.clone())),
            DragBatchUpdate::HoverChanged { count: 2 }
        );
        assert_eq!(
            aggregator.push(NativeFileDropEvent::Dropped(first.clone())),
            DragBatchUpdate::DropPending { count: 1 }
        );
        assert_eq!(
            aggregator.push(NativeFileDropEvent::Dropped(second.clone())),
            DragBatchUpdate::DropPending { count: 2 }
        );

        let batch = aggregator.flush_dropped().unwrap();
        assert_eq!(batch.paths(), [first, second]);
        assert_eq!(aggregator.state(), DragBatchState::Idle);
        assert_eq!(aggregator.flush_dropped(), None);
    }

    #[test]
    fn duplicate_paths_keep_first_seen_order() {
        let first = path("first.txt");
        let second = path("second.txt");
        let mut aggregator = FileDropAggregator::default();

        aggregator.push(NativeFileDropEvent::Dropped(first.clone()));
        aggregator.push(NativeFileDropEvent::Dropped(first.clone()));
        aggregator.push(NativeFileDropEvent::Dropped(second.clone()));
        aggregator.push(NativeFileDropEvent::Dropped(first.clone()));

        let batch = aggregator.flush_dropped().unwrap();
        assert_eq!(batch.into_paths(), vec![first, second]);
    }

    #[test]
    fn cancellation_clears_hover_and_pending_drop() {
        let mut aggregator = FileDropAggregator::default();
        aggregator.push(NativeFileDropEvent::Hovered(path("hovered.txt")));
        aggregator.push(NativeFileDropEvent::Dropped(path("dropped.txt")));

        assert_eq!(
            aggregator.push(NativeFileDropEvent::Cancelled),
            DragBatchUpdate::Cancelled
        );
        assert_eq!(aggregator.state(), DragBatchState::Idle);
        assert!(aggregator.hovered_paths().is_empty());
        assert!(aggregator.pending_drop_paths().is_empty());
        assert_eq!(aggregator.flush_dropped(), None);
        assert_eq!(
            aggregator.push(NativeFileDropEvent::Cancelled),
            DragBatchUpdate::None
        );
    }

    #[test]
    fn hovering_alone_never_creates_a_drop_batch() {
        let hovered = path("hovered.txt");
        let mut aggregator = FileDropAggregator::default();
        aggregator.push(NativeFileDropEvent::Hovered(hovered.clone()));

        assert_eq!(aggregator.flush_dropped(), None);
        assert_eq!(aggregator.state(), DragBatchState::Hovering);
        assert_eq!(aggregator.hovered_paths(), [hovered]);
    }

    #[test]
    fn drop_without_hover_is_accepted() {
        let dropped = path("direct.txt");
        let mut aggregator = FileDropAggregator::default();
        aggregator.push(NativeFileDropEvent::Dropped(dropped.clone()));

        assert_eq!(
            aggregator.flush_dropped().unwrap().into_paths(),
            vec![dropped]
        );
    }

    #[test]
    fn separate_flushes_create_separate_batches() {
        let first = path("first.txt");
        let second = path("second.txt");
        let mut aggregator = FileDropAggregator::default();

        aggregator.push(NativeFileDropEvent::Dropped(first.clone()));
        assert_eq!(
            aggregator.flush_dropped().unwrap().into_paths(),
            vec![first]
        );
        aggregator.push(NativeFileDropEvent::Dropped(second.clone()));
        assert_eq!(
            aggregator.flush_dropped().unwrap().into_paths(),
            vec![second]
        );
    }

    #[test]
    fn empty_paths_are_rejected_without_polluting_the_session() {
        let valid = path("valid.txt");
        let mut aggregator = FileDropAggregator::default();

        assert_eq!(
            aggregator.push(NativeFileDropEvent::Dropped(PathBuf::new())),
            DragBatchUpdate::Rejected(FileDropRejection::EmptyPath)
        );
        assert_eq!(aggregator.state(), DragBatchState::Idle);
        assert_eq!(aggregator.flush_dropped(), None);

        aggregator.push(NativeFileDropEvent::Dropped(valid.clone()));
        assert_eq!(
            aggregator.flush_dropped().unwrap().into_paths(),
            vec![valid]
        );
    }
}
