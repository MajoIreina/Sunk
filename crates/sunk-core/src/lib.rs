//! Platform-independent domain state for Sunk.

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The lifecycle of one drag-and-consume interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionState {
    Idle,
    Hovering,
    Attracting,
    Capturing,
    Orbiting,
    EnteringEventHorizon,
    Consuming,
    Completed,
    Error,
}

/// Events accepted by [`InteractionStateMachine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionEvent {
    DragEntered(PathBuf),
    AttractionStarted,
    TargetCaptured(PathBuf),
    OrbitStarted,
    TargetReachedEventHorizon(PathBuf),
    ConsumptionStarted,
    ConsumptionCompleted(PathBuf),
    ConsumptionFailed(PathBuf),
    DragLeft,
    Reset,
}

impl InteractionEvent {
    #[must_use]
    pub fn kind(&self) -> InteractionEventKind {
        match self {
            Self::DragEntered(_) => InteractionEventKind::DragEntered,
            Self::AttractionStarted => InteractionEventKind::AttractionStarted,
            Self::TargetCaptured(_) => InteractionEventKind::TargetCaptured,
            Self::OrbitStarted => InteractionEventKind::OrbitStarted,
            Self::TargetReachedEventHorizon(_) => InteractionEventKind::TargetReachedEventHorizon,
            Self::ConsumptionStarted => InteractionEventKind::ConsumptionStarted,
            Self::ConsumptionCompleted(_) => InteractionEventKind::ConsumptionCompleted,
            Self::ConsumptionFailed(_) => InteractionEventKind::ConsumptionFailed,
            Self::DragLeft => InteractionEventKind::DragLeft,
            Self::Reset => InteractionEventKind::Reset,
        }
    }

    fn target(&self) -> Option<&Path> {
        match self {
            Self::DragEntered(path)
            | Self::TargetCaptured(path)
            | Self::TargetReachedEventHorizon(path)
            | Self::ConsumptionCompleted(path)
            | Self::ConsumptionFailed(path) => Some(path),
            _ => None,
        }
    }
}

/// Path-free event identity, suitable for logs, metrics, and errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionEventKind {
    DragEntered,
    AttractionStarted,
    TargetCaptured,
    OrbitStarted,
    TargetReachedEventHorizon,
    ConsumptionStarted,
    ConsumptionCompleted,
    ConsumptionFailed,
    DragLeft,
    Reset,
}

/// A successful and observable state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransition {
    pub from: InteractionState,
    pub to: InteractionState,
    pub event: InteractionEventKind,
    pub target: Option<PathBuf>,
}

/// Rejected input never mutates the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionError {
    EmptyTargetPath,
    MissingTarget {
        event: InteractionEventKind,
    },
    TargetMismatch {
        expected: PathBuf,
        received: PathBuf,
    },
    InvalidTransition {
        state: InteractionState,
        event: InteractionEventKind,
    },
}

impl fmt::Display for InteractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTargetPath => formatter.write_str("the interaction target path is empty"),
            Self::MissingTarget { event } => {
                write!(formatter, "event {event:?} requires an active target")
            }
            Self::TargetMismatch { expected, received } => write!(
                formatter,
                "event target '{}' does not match active target '{}'",
                received.display(),
                expected.display()
            ),
            Self::InvalidTransition { state, event } => {
                write!(formatter, "event {event:?} is invalid in state {state:?}")
            }
        }
    }
}

impl Error for InteractionError {}

/// Owns the state for a single target interaction.
#[derive(Debug, Clone)]
pub struct InteractionStateMachine {
    state: InteractionState,
    target: Option<PathBuf>,
}

impl Default for InteractionStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl InteractionStateMachine {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: InteractionState::Idle,
            target: None,
        }
    }

    #[must_use]
    pub const fn state(&self) -> InteractionState {
        self.state
    }

    #[must_use]
    pub fn target(&self) -> Option<&Path> {
        self.target.as_deref()
    }

    /// Applies one domain event without partially mutating state on failure.
    ///
    /// # Errors
    ///
    /// Returns an error when the event is invalid for the current state or its path does not match
    /// the active target.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "events represent consumed messages at the state-machine boundary"
    )]
    pub fn handle(&mut self, event: InteractionEvent) -> Result<StateTransition, InteractionError> {
        let from = self.state;
        let event_kind = event.kind();

        if event
            .target()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(InteractionError::EmptyTargetPath);
        }

        let mut next_target = self.target.clone();
        let next_state = match (self.state, &event) {
            (InteractionState::Idle, InteractionEvent::DragEntered(path)) => {
                next_target = Some(path.clone());
                InteractionState::Hovering
            }
            (InteractionState::Hovering, InteractionEvent::AttractionStarted) => {
                InteractionState::Attracting
            }
            (InteractionState::Attracting, InteractionEvent::TargetCaptured(path)) => {
                self.ensure_target(path, event_kind)?;
                InteractionState::Capturing
            }
            (InteractionState::Capturing, InteractionEvent::OrbitStarted) => {
                InteractionState::Orbiting
            }
            (InteractionState::Orbiting, InteractionEvent::TargetReachedEventHorizon(path)) => {
                self.ensure_target(path, event_kind)?;
                InteractionState::EnteringEventHorizon
            }
            (InteractionState::EnteringEventHorizon, InteractionEvent::ConsumptionStarted) => {
                InteractionState::Consuming
            }
            (InteractionState::Consuming, InteractionEvent::ConsumptionCompleted(path)) => {
                self.ensure_target(path, event_kind)?;
                InteractionState::Completed
            }
            (state, InteractionEvent::ConsumptionFailed(path))
                if state != InteractionState::Idle =>
            {
                self.ensure_target(path, event_kind)?;
                InteractionState::Error
            }
            (
                InteractionState::Hovering | InteractionState::Attracting,
                InteractionEvent::DragLeft,
            ) => {
                next_target = None;
                InteractionState::Idle
            }
            (state, InteractionEvent::Reset) if state != InteractionState::Idle => {
                next_target = None;
                InteractionState::Idle
            }
            _ => {
                return Err(InteractionError::InvalidTransition {
                    state: self.state,
                    event: event_kind,
                });
            }
        };

        self.state = next_state;
        self.target = next_target;

        Ok(StateTransition {
            from,
            to: next_state,
            event: event_kind,
            target: self.target.clone(),
        })
    }

    fn ensure_target(
        &self,
        received: &Path,
        event: InteractionEventKind,
    ) -> Result<(), InteractionError> {
        let Some(expected) = self.target.as_deref() else {
            return Err(InteractionError::MissingTarget { event });
        };

        if expected != received {
            return Err(InteractionError::TargetMismatch {
                expected: expected.to_path_buf(),
                received: received.to_path_buf(),
            });
        }

        Ok(())
    }
}

/// Maximum number of unique targets accepted in one atomic drop batch.
pub const MAX_DROP_BATCH_TARGETS: usize = 256;

/// A validated, ordered set of paths received from one operating-system drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropBatch {
    paths: Vec<PathBuf>,
}

impl DropBatch {
    /// Validates a drop and removes exact duplicate paths without changing their first-seen order.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty batch, an empty path, or more than 256 unique targets. The
    /// whole batch is rejected instead of being truncated.
    pub fn try_new(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, DropBatchError> {
        let mut unique_paths = Vec::new();
        let mut seen = HashSet::new();

        for (index, path) in paths.into_iter().enumerate() {
            if path.as_os_str().is_empty() {
                return Err(DropBatchError::EmptyPath { index });
            }
            if seen.insert(path.clone()) {
                unique_paths.push(path);
                if unique_paths.len() > MAX_DROP_BATCH_TARGETS {
                    return Err(DropBatchError::TooManyUniqueTargets {
                        maximum: MAX_DROP_BATCH_TARGETS,
                        received: unique_paths.len(),
                    });
                }
            }
        }

        if unique_paths.is_empty() {
            return Err(DropBatchError::EmptyBatch);
        }

        Ok(Self {
            paths: unique_paths,
        })
    }

    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
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

/// Validation failures reject a complete [`DropBatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropBatchError {
    EmptyBatch,
    EmptyPath { index: usize },
    TooManyUniqueTargets { maximum: usize, received: usize },
}

impl fmt::Display for DropBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBatch => formatter.write_str("a drop batch must contain at least one path"),
            Self::EmptyPath { index } => write!(formatter, "drop path at index {index} is empty"),
            Self::TooManyUniqueTargets { maximum, received } => write!(
                formatter,
                "drop batch has {received} unique targets; the maximum is {maximum}"
            ),
        }
    }
}

impl Error for DropBatchError {}

/// Renderer-facing phase for a path-free visual target snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualCapturePhase {
    Attracting,
    Capturing,
    Orbiting,
    EnteringEventHorizon,
}

/// A stable, path-free description of one visual target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualCaptureSnapshot {
    pub visual_id: u64,
    pub phase: VisualCapturePhase,
    /// Phase-local progress in the inclusive range `0.0..=1.0`.
    pub progress: f32,
    pub orbit_lane: u16,
}

/// Errors produced without discarding any batch already owned by the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualCaptureError {
    PendingQueueFull { capacity: usize },
    VisualIdExhausted,
    Interaction(InteractionError),
}

impl fmt::Display for VisualCaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PendingQueueFull { capacity } => {
                write!(
                    formatter,
                    "visual capture queue is full ({capacity} pending batches)"
                )
            }
            Self::VisualIdExhausted => {
                formatter.write_str("visual target identifier space is exhausted")
            }
            Self::Interaction(error) => {
                write!(formatter, "visual state transition failed: {error}")
            }
        }
    }
}

impl Error for VisualCaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Interaction(error) => Some(error),
            Self::PendingQueueFull { .. } | Self::VisualIdExhausted => None,
        }
    }
}

impl From<InteractionError> for VisualCaptureError {
    fn from(error: InteractionError) -> Self {
        Self::Interaction(error)
    }
}

#[derive(Debug, Clone)]
struct VisualTarget {
    visual_id: u64,
    orbit_lane: u16,
    start_delay: Duration,
    path: PathBuf,
    interaction: InteractionStateMachine,
}

impl VisualTarget {
    fn new(
        visual_id: u64,
        orbit_lane: u16,
        start_delay: Duration,
        path: PathBuf,
    ) -> Result<Self, InteractionError> {
        let mut interaction = InteractionStateMachine::new();
        interaction.handle(InteractionEvent::DragEntered(path.clone()))?;
        interaction.handle(InteractionEvent::AttractionStarted)?;
        Ok(Self {
            visual_id,
            orbit_lane,
            start_delay,
            path,
            interaction,
        })
    }

    fn synchronize(&mut self, batch_elapsed: Duration) -> Result<(), InteractionError> {
        let local_elapsed = batch_elapsed.saturating_sub(self.start_delay);
        if batch_elapsed < self.start_delay {
            return Ok(());
        }

        if local_elapsed >= VisualCaptureController::ATTRACTION_DURATION
            && self.interaction.state() == InteractionState::Attracting
        {
            self.interaction
                .handle(InteractionEvent::TargetCaptured(self.path.clone()))?;
        }
        if local_elapsed
            >= VisualCaptureController::ATTRACTION_DURATION
                + VisualCaptureController::CAPTURING_DURATION
            && self.interaction.state() == InteractionState::Capturing
        {
            self.interaction.handle(InteractionEvent::OrbitStarted)?;
        }
        if local_elapsed
            >= VisualCaptureController::ATTRACTION_DURATION
                + VisualCaptureController::CAPTURING_DURATION
                + VisualCaptureController::ORBITING_DURATION
            && self.interaction.state() == InteractionState::Orbiting
        {
            self.interaction
                .handle(InteractionEvent::TargetReachedEventHorizon(
                    self.path.clone(),
                ))?;
        }
        Ok(())
    }

    fn snapshot(&self, batch_elapsed: Duration) -> VisualCaptureSnapshot {
        let local_elapsed = batch_elapsed.saturating_sub(self.start_delay);
        let (phase, phase_elapsed, phase_duration) = match self.interaction.state() {
            InteractionState::Attracting => (
                VisualCapturePhase::Attracting,
                local_elapsed,
                VisualCaptureController::ATTRACTION_DURATION,
            ),
            InteractionState::Capturing => (
                VisualCapturePhase::Capturing,
                local_elapsed.saturating_sub(VisualCaptureController::ATTRACTION_DURATION),
                VisualCaptureController::CAPTURING_DURATION,
            ),
            InteractionState::Orbiting => (
                VisualCapturePhase::Orbiting,
                local_elapsed.saturating_sub(
                    VisualCaptureController::ATTRACTION_DURATION
                        + VisualCaptureController::CAPTURING_DURATION,
                ),
                VisualCaptureController::ORBITING_DURATION,
            ),
            InteractionState::EnteringEventHorizon => (
                VisualCapturePhase::EnteringEventHorizon,
                local_elapsed.saturating_sub(
                    VisualCaptureController::ATTRACTION_DURATION
                        + VisualCaptureController::CAPTURING_DURATION
                        + VisualCaptureController::ORBITING_DURATION,
                ),
                VisualCaptureController::EVENT_HORIZON_DURATION,
            ),
            state => unreachable!("visual target entered non-visual state {state:?}"),
        };

        VisualCaptureSnapshot {
            visual_id: self.visual_id,
            phase,
            progress: duration_progress(phase_elapsed, phase_duration),
            orbit_lane: self.orbit_lane,
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveCaptureBatch {
    targets: Vec<VisualTarget>,
    elapsed: Duration,
    total_duration: Duration,
}

impl ActiveCaptureBatch {
    fn try_new(batch: DropBatch, first_visual_id: u64) -> Result<Self, InteractionError> {
        let mut targets = Vec::with_capacity(batch.len());
        for (index, path) in batch.paths.into_iter().enumerate() {
            let ordinal = u64::try_from(index).expect("drop batches contain at most 256 targets");
            let stagger_index =
                u32::try_from(index).expect("drop batches contain at most 256 targets");
            let orbit_lane = u16::try_from(index % VisualCaptureController::ORBIT_LANE_COUNT)
                .expect("orbit lane count fits in u16");
            targets.push(VisualTarget::new(
                first_visual_id + ordinal,
                orbit_lane,
                VisualCaptureController::TARGET_STAGGER.saturating_mul(stagger_index),
                path,
            )?);
        }

        let last_start_delay = targets
            .last()
            .map_or(Duration::ZERO, |target| target.start_delay);
        Ok(Self {
            targets,
            elapsed: Duration::ZERO,
            total_duration: last_start_delay + VisualCaptureController::TARGET_DURATION,
        })
    }

    fn advance(&mut self, delta: Duration) -> Result<Duration, InteractionError> {
        let remaining_in_batch = self.total_duration.saturating_sub(self.elapsed);
        let consumed = delta.min(remaining_in_batch);
        self.elapsed += consumed;
        for target in &mut self.targets {
            target.synchronize(self.elapsed)?;
        }
        Ok(delta.saturating_sub(consumed))
    }

    fn is_complete(&self) -> bool {
        self.elapsed >= self.total_duration
    }
}

/// Advances queued multi-target capture animations without invoking file operations.
#[derive(Debug, Clone)]
pub struct VisualCaptureController {
    active: Option<ActiveCaptureBatch>,
    pending: VecDeque<ActiveCaptureBatch>,
    next_visual_id: u64,
}

impl Default for VisualCaptureController {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualCaptureController {
    pub const MAX_PENDING_BATCHES: usize = 8;
    pub const ORBIT_LANE_COUNT: usize = 8;
    pub const TARGET_STAGGER: Duration = Duration::from_millis(120);
    pub const ATTRACTION_DURATION: Duration = Duration::from_millis(320);
    pub const CAPTURING_DURATION: Duration = Duration::from_millis(240);
    pub const ORBITING_DURATION: Duration = Duration::from_millis(960);
    pub const EVENT_HORIZON_DURATION: Duration = Duration::from_millis(320);
    pub const TARGET_DURATION: Duration = Self::ATTRACTION_DURATION
        .saturating_add(Self::CAPTURING_DURATION)
        .saturating_add(Self::ORBITING_DURATION)
        .saturating_add(Self::EVENT_HORIZON_DURATION);

    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: None,
            pending: VecDeque::new(),
            next_visual_id: 1,
        }
    }

    /// Accepts one atomic drop batch or returns an error without evicting queued work.
    ///
    /// # Errors
    ///
    /// Returns an error when eight batches are already waiting, visual identifiers are exhausted,
    /// or an internal single-target transition rejects the validated batch.
    pub fn submit_batch(&mut self, batch: DropBatch) -> Result<(), VisualCaptureError> {
        if self.active.is_some() && self.pending.len() >= Self::MAX_PENDING_BATCHES {
            return Err(VisualCaptureError::PendingQueueFull {
                capacity: Self::MAX_PENDING_BATCHES,
            });
        }

        let target_count = batch.len() as u64;
        let next_visual_id = self
            .next_visual_id
            .checked_add(target_count)
            .ok_or(VisualCaptureError::VisualIdExhausted)?;
        let capture_batch = ActiveCaptureBatch::try_new(batch, self.next_visual_id)?;

        if self.active.is_none() {
            self.active = Some(capture_batch);
        } else {
            self.pending.push_back(capture_batch);
        }
        self.next_visual_id = next_visual_id;
        Ok(())
    }

    /// Advances visual time. Excess time crosses phase and batch boundaries deterministically.
    ///
    /// # Errors
    ///
    /// Returns an error only if an internal single-target state transition is rejected.
    pub fn advance(&mut self, mut delta: Duration) -> Result<(), VisualCaptureError> {
        if delta.is_zero() {
            return Ok(());
        }

        while let Some(active) = self.active.as_mut() {
            let remaining = active.advance(delta)?;
            if !active.is_complete() {
                break;
            }

            self.active = self.pending.pop_front();
            delta = remaining;
            if delta.is_zero() {
                break;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn has_work(&self) -> bool {
        self.active.is_some() || !self.pending.is_empty()
    }

    #[must_use]
    pub fn pending_batch_count(&self) -> usize {
        self.pending.len()
    }

    /// Returns only renderer-safe values; paths remain private to the domain controller.
    #[must_use]
    pub fn snapshots(&self) -> Vec<VisualCaptureSnapshot> {
        self.active.as_ref().map_or_else(Vec::new, |active| {
            active
                .targets
                .iter()
                .filter(|target| {
                    active.elapsed >= target.start_delay
                        && active.elapsed < target.start_delay + Self::TARGET_DURATION
                })
                .map(|target| target.snapshot(active.elapsed))
                .collect()
        })
    }
}

fn duration_progress(elapsed: Duration, duration: Duration) -> f32 {
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

/// Named quality tiers ordered from least to most expensive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RenderQualityLevel {
    Background,
    Performance,
    Balanced,
    High,
    Cinematic,
}

impl RenderQualityLevel {
    const fn step_up(self) -> Self {
        match self {
            Self::Background => Self::Performance,
            Self::Performance => Self::Balanced,
            Self::Balanced => Self::High,
            Self::High | Self::Cinematic => Self::Cinematic,
        }
    }

    const fn step_down(self) -> Self {
        match self {
            Self::Cinematic => Self::High,
            Self::High => Self::Balanced,
            Self::Balanced => Self::Performance,
            Self::Performance | Self::Background => Self::Background,
        }
    }
}

/// Renderer-facing controls changed together as one quality tier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderQuality {
    pub resolution_scale: f32,
    pub ray_steps: u32,
    pub disk_steps: u32,
    pub particle_budget: u32,
    pub bloom_enabled: bool,
}

impl RenderQuality {
    #[must_use]
    pub const fn for_level(level: RenderQualityLevel) -> Self {
        match level {
            RenderQualityLevel::Cinematic => Self {
                resolution_scale: 1.0,
                ray_steps: 96,
                disk_steps: 96,
                particle_budget: 50_000,
                bloom_enabled: true,
            },
            RenderQualityLevel::High => Self {
                resolution_scale: 1.0,
                ray_steps: 88,
                disk_steps: 72,
                particle_budget: 35_000,
                bloom_enabled: true,
            },
            RenderQualityLevel::Balanced => Self {
                resolution_scale: 0.75,
                ray_steps: 80,
                disk_steps: 48,
                particle_budget: 20_000,
                bloom_enabled: true,
            },
            RenderQualityLevel::Performance => Self {
                resolution_scale: 0.5,
                ray_steps: 72,
                disk_steps: 24,
                particle_budget: 10_000,
                bloom_enabled: false,
            },
            RenderQualityLevel::Background => Self {
                resolution_scale: 0.3,
                ray_steps: 64,
                disk_steps: 12,
                particle_budget: 2_500,
                bloom_enabled: false,
            },
        }
    }
}

impl Default for RenderQuality {
    fn default() -> Self {
        Self::for_level(RenderQualityLevel::Balanced)
    }
}

/// Adjusts renderer quality only after sustained load or headroom.
#[derive(Debug, Clone)]
pub struct PerformanceController {
    current_level: RenderQualityLevel,
    slow_for: Duration,
    fast_for: Duration,
    idle_for: Duration,
    was_interactive: bool,
}

impl Default for PerformanceController {
    fn default() -> Self {
        Self::with_level(RenderQualityLevel::Balanced)
    }
}

impl PerformanceController {
    const DOWNGRADE_FRAME_TIME: Duration = Duration::from_millis(20);
    const UPGRADE_FRAME_TIME: Duration = Duration::from_millis(12);
    const DOWNGRADE_HOLD: Duration = Duration::from_millis(750);
    const UPGRADE_HOLD: Duration = Duration::from_secs(3);
    const BACKGROUND_HOLD: Duration = Duration::from_secs(5);

    #[must_use]
    pub const fn with_level(level: RenderQualityLevel) -> Self {
        Self {
            current_level: level,
            slow_for: Duration::ZERO,
            fast_for: Duration::ZERO,
            idle_for: Duration::ZERO,
            was_interactive: false,
        }
    }

    #[must_use]
    pub const fn current_level(&self) -> RenderQualityLevel {
        self.current_level
    }

    #[must_use]
    pub const fn current_quality(&self) -> RenderQuality {
        RenderQuality::for_level(self.current_level)
    }

    /// Records one frame and returns a new quality only when the tier changes.
    pub fn update(
        &mut self,
        frame_time: Duration,
        sample_period: Duration,
        interactive: bool,
    ) -> Option<RenderQuality> {
        if !interactive {
            self.was_interactive = false;
            self.fast_for = Duration::ZERO;
            self.slow_for = Duration::ZERO;
            self.idle_for = self.idle_for.saturating_add(sample_period);

            if self.idle_for >= Self::BACKGROUND_HOLD
                && self.current_level != RenderQualityLevel::Background
            {
                return self.change_level(RenderQualityLevel::Background);
            }
            return None;
        }

        self.idle_for = Duration::ZERO;
        if !self.was_interactive {
            self.was_interactive = true;
            if self.current_level < RenderQualityLevel::Balanced {
                return self.change_level(RenderQualityLevel::Balanced);
            }
        }

        if frame_time > Self::DOWNGRADE_FRAME_TIME {
            self.fast_for = Duration::ZERO;
            self.slow_for = self.slow_for.saturating_add(sample_period);
            if self.slow_for >= Self::DOWNGRADE_HOLD {
                return self.change_level(self.current_level.step_down());
            }
        } else if frame_time < Self::UPGRADE_FRAME_TIME {
            self.slow_for = Duration::ZERO;
            self.fast_for = self.fast_for.saturating_add(sample_period);
            if self.fast_for >= Self::UPGRADE_HOLD {
                return self.change_level(self.current_level.step_up());
            }
        } else {
            self.fast_for = Duration::ZERO;
            self.slow_for = Duration::ZERO;
        }

        None
    }

    fn change_level(&mut self, level: RenderQualityLevel) -> Option<RenderQuality> {
        self.fast_for = Duration::ZERO;
        self.slow_for = Duration::ZERO;
        if level == self.current_level {
            return None;
        }

        self.current_level = level;
        Some(self.current_quality())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PathBuf {
        PathBuf::from("drop/target.txt")
    }

    #[test]
    fn interaction_completes_the_happy_path() {
        let path = target();
        let mut machine = InteractionStateMachine::new();
        let events = [
            InteractionEvent::DragEntered(path.clone()),
            InteractionEvent::AttractionStarted,
            InteractionEvent::TargetCaptured(path.clone()),
            InteractionEvent::OrbitStarted,
            InteractionEvent::TargetReachedEventHorizon(path.clone()),
            InteractionEvent::ConsumptionStarted,
            InteractionEvent::ConsumptionCompleted(path.clone()),
        ];

        for event in events {
            machine.handle(event).unwrap();
        }

        assert_eq!(machine.state(), InteractionState::Completed);
        assert_eq!(machine.target(), Some(path.as_path()));

        machine.handle(InteractionEvent::Reset).unwrap();
        assert_eq!(machine.state(), InteractionState::Idle);
        assert_eq!(machine.target(), None);
    }

    #[test]
    fn rejected_event_does_not_mutate_state() {
        let mut machine = InteractionStateMachine::new();
        let error = machine
            .handle(InteractionEvent::ConsumptionStarted)
            .unwrap_err();

        assert_eq!(
            error,
            InteractionError::InvalidTransition {
                state: InteractionState::Idle,
                event: InteractionEventKind::ConsumptionStarted,
            }
        );
        assert_eq!(machine.state(), InteractionState::Idle);
        assert_eq!(machine.target(), None);
    }

    #[test]
    fn target_mismatch_is_explicit_and_preserves_state() {
        let path = target();
        let mut machine = InteractionStateMachine::new();
        machine
            .handle(InteractionEvent::DragEntered(path.clone()))
            .unwrap();
        machine.handle(InteractionEvent::AttractionStarted).unwrap();

        let other = PathBuf::from("drop/other.txt");
        let error = machine
            .handle(InteractionEvent::TargetCaptured(other.clone()))
            .unwrap_err();

        assert_eq!(
            error,
            InteractionError::TargetMismatch {
                expected: path.clone(),
                received: other,
            }
        );
        assert_eq!(machine.state(), InteractionState::Attracting);
        assert_eq!(machine.target(), Some(path.as_path()));
    }

    #[test]
    fn operation_failure_enters_error_and_can_reset() {
        let path = target();
        let mut machine = InteractionStateMachine::new();
        machine
            .handle(InteractionEvent::DragEntered(path.clone()))
            .unwrap();
        machine
            .handle(InteractionEvent::ConsumptionFailed(path))
            .unwrap();
        assert_eq!(machine.state(), InteractionState::Error);

        machine.handle(InteractionEvent::Reset).unwrap();
        assert_eq!(machine.state(), InteractionState::Idle);
    }

    fn drop_batch(paths: &[&str]) -> DropBatch {
        DropBatch::try_new(paths.iter().map(PathBuf::from)).unwrap()
    }

    #[test]
    fn drop_batch_validates_and_stably_deduplicates_paths() {
        assert_eq!(
            DropBatch::try_new(Vec::<PathBuf>::new()),
            Err(DropBatchError::EmptyBatch)
        );
        assert_eq!(
            DropBatch::try_new([PathBuf::from("drop/a"), PathBuf::new()]),
            Err(DropBatchError::EmptyPath { index: 1 })
        );

        let batch = drop_batch(&["drop/b", "drop/a", "drop/b", "drop/c", "drop/a"]);
        assert_eq!(
            batch.paths(),
            [
                PathBuf::from("drop/b"),
                PathBuf::from("drop/a"),
                PathBuf::from("drop/c")
            ]
        );
    }

    #[test]
    fn oversized_drop_batch_is_rejected_atomically() {
        let paths = (0..=MAX_DROP_BATCH_TARGETS)
            .map(|index| PathBuf::from(format!("drop/{index}")))
            .collect::<Vec<_>>();
        assert_eq!(
            DropBatch::try_new(paths),
            Err(DropBatchError::TooManyUniqueTargets {
                maximum: MAX_DROP_BATCH_TARGETS,
                received: MAX_DROP_BATCH_TARGETS + 1,
            })
        );
    }

    #[test]
    fn capture_timeline_is_deterministic_and_snapshots_are_path_free() {
        let batch = drop_batch(&["private/alpha", "private/beta"]);
        let mut first = VisualCaptureController::new();
        let mut second = VisualCaptureController::new();
        first.submit_batch(batch.clone()).unwrap();
        second.submit_batch(batch).unwrap();

        for delta in [
            Duration::from_millis(100),
            Duration::from_millis(400),
            Duration::from_millis(700),
        ] {
            first.advance(delta).unwrap();
            second.advance(delta).unwrap();
            assert_eq!(first.snapshots(), second.snapshots());
        }

        let snapshots = first.snapshots();
        assert_eq!(snapshots.len(), 2);
        assert_eq!((snapshots[0].visual_id, snapshots[0].orbit_lane), (1, 0));
        assert_eq!((snapshots[1].visual_id, snapshots[1].orbit_lane), (2, 1));
        assert!(snapshots.iter().all(|snapshot| {
            (0.0..=1.0).contains(&snapshot.progress)
                && snapshot.phase == VisualCapturePhase::Orbiting
        }));

        let debug_snapshot = format!("{snapshots:?}");
        assert!(!debug_snapshot.contains("private"));
        assert!(!debug_snapshot.contains("alpha"));
    }

    #[test]
    fn zero_delta_does_not_change_capture_state() {
        let mut controller = VisualCaptureController::new();
        controller.submit_batch(drop_batch(&["drop/a"])).unwrap();
        let before = controller.snapshots();
        controller.advance(Duration::ZERO).unwrap();
        assert_eq!(controller.snapshots(), before);
        assert!(controller.has_work());
    }

    #[test]
    fn snapshots_only_include_targets_inside_their_visual_window() {
        let mut controller = VisualCaptureController::new();
        controller
            .submit_batch(drop_batch(&["drop/a", "drop/b", "drop/c"]))
            .unwrap();

        assert_eq!(
            controller
                .snapshots()
                .iter()
                .map(|snapshot| snapshot.visual_id)
                .collect::<Vec<_>>(),
            [1]
        );

        controller
            .advance(VisualCaptureController::TARGET_STAGGER * 2)
            .unwrap();
        assert_eq!(
            controller
                .snapshots()
                .iter()
                .map(|snapshot| snapshot.visual_id)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );

        let until_first_finishes = VisualCaptureController::TARGET_DURATION
            .checked_sub(VisualCaptureController::TARGET_STAGGER * 2)
            .unwrap();
        controller.advance(until_first_finishes).unwrap();
        assert_eq!(
            controller
                .snapshots()
                .iter()
                .map(|snapshot| snapshot.visual_id)
                .collect::<Vec<_>>(),
            [2, 3]
        );

        controller
            .advance(VisualCaptureController::TARGET_STAGGER * 2)
            .unwrap();
        assert!(!controller.has_work());
        assert!(controller.snapshots().is_empty());
    }

    #[test]
    fn large_delta_crosses_all_visual_phases_and_returns_to_idle() {
        let mut controller = VisualCaptureController::new();
        controller
            .submit_batch(drop_batch(&["drop/a", "drop/b", "drop/c"]))
            .unwrap();

        controller.advance(Duration::from_secs(30)).unwrap();
        assert!(!controller.has_work());
        assert!(controller.snapshots().is_empty());
    }

    #[test]
    fn queued_batches_run_fifo_and_keep_visual_ids_stable() {
        let mut controller = VisualCaptureController::new();
        controller.submit_batch(drop_batch(&["drop/a"])).unwrap();
        controller
            .submit_batch(drop_batch(&["drop/b", "drop/c"]))
            .unwrap();
        assert_eq!(controller.pending_batch_count(), 1);
        assert_eq!(controller.snapshots()[0].visual_id, 1);

        controller
            .advance(VisualCaptureController::TARGET_DURATION)
            .unwrap();
        let next = controller.snapshots();
        assert_eq!(controller.pending_batch_count(), 0);
        assert_eq!(
            next.iter()
                .map(|snapshot| snapshot.visual_id)
                .collect::<Vec<_>>(),
            [2]
        );

        controller
            .advance(VisualCaptureController::TARGET_STAGGER)
            .unwrap();
        assert_eq!(
            controller
                .snapshots()
                .iter()
                .map(|snapshot| snapshot.visual_id)
                .collect::<Vec<_>>(),
            [2, 3]
        );

        controller.advance(Duration::from_secs(30)).unwrap();
        assert!(!controller.has_work());
    }

    #[test]
    fn full_pending_queue_rejects_new_batch_without_losing_work() {
        let mut controller = VisualCaptureController::new();
        controller
            .submit_batch(drop_batch(&["drop/active"]))
            .unwrap();
        for index in 0..VisualCaptureController::MAX_PENDING_BATCHES {
            controller
                .submit_batch(drop_batch(&[&format!("drop/queued-{index}")]))
                .unwrap();
        }
        let snapshots_before = controller.snapshots();

        assert_eq!(
            controller.submit_batch(drop_batch(&["drop/rejected"])),
            Err(VisualCaptureError::PendingQueueFull {
                capacity: VisualCaptureController::MAX_PENDING_BATCHES,
            })
        );
        assert_eq!(controller.pending_batch_count(), 8);
        assert_eq!(controller.snapshots(), snapshots_before);
    }

    #[test]
    fn visual_capture_stops_before_consumption_states() {
        let mut controller = VisualCaptureController::new();
        controller.submit_batch(drop_batch(&["drop/a"])).unwrap();

        let before_finish = VisualCaptureController::TARGET_DURATION
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        controller.advance(before_finish).unwrap();
        assert_eq!(
            controller.snapshots()[0].phase,
            VisualCapturePhase::EnteringEventHorizon
        );
        controller.advance(Duration::from_millis(1)).unwrap();
        assert!(!controller.has_work());

        let mut single = InteractionStateMachine::new();
        let path = PathBuf::from("drop/a");
        for event in [
            InteractionEvent::DragEntered(path.clone()),
            InteractionEvent::AttractionStarted,
            InteractionEvent::TargetCaptured(path.clone()),
            InteractionEvent::OrbitStarted,
            InteractionEvent::TargetReachedEventHorizon(path),
        ] {
            single.handle(event).unwrap();
        }
        assert_eq!(single.state(), InteractionState::EnteringEventHorizon);
    }

    #[test]
    fn one_slow_frame_does_not_change_quality() {
        let mut controller = PerformanceController::default();
        assert_eq!(
            controller.update(Duration::from_millis(25), Duration::from_millis(16), true),
            None
        );
        assert_eq!(controller.current_level(), RenderQualityLevel::Balanced);
    }

    #[test]
    fn sustained_slow_frames_downgrade_one_tier() {
        let mut controller = PerformanceController::default();
        let mut changed = None;
        for _ in 0..30 {
            changed = changed.or(controller.update(
                Duration::from_millis(25),
                Duration::from_millis(25),
                true,
            ));
        }

        assert_eq!(controller.current_level(), RenderQualityLevel::Performance);
        assert_eq!(
            changed,
            Some(RenderQuality::for_level(RenderQualityLevel::Performance))
        );
    }

    #[test]
    fn sustained_fast_frames_upgrade_one_tier() {
        let mut controller = PerformanceController::default();
        let mut changed = None;
        for _ in 0..300 {
            changed = changed.or(controller.update(
                Duration::from_millis(10),
                Duration::from_millis(10),
                true,
            ));
        }

        assert_eq!(controller.current_level(), RenderQualityLevel::High);
        assert_eq!(
            changed,
            Some(RenderQuality::for_level(RenderQualityLevel::High))
        );
    }

    #[test]
    fn idle_uses_background_quality_and_interaction_restores_balanced() {
        let mut controller = PerformanceController::with_level(RenderQualityLevel::High);
        let background = controller.update(Duration::from_millis(1), Duration::from_secs(5), false);
        assert_eq!(
            background,
            Some(RenderQuality::for_level(RenderQualityLevel::Background))
        );

        let active = controller.update(Duration::from_millis(16), Duration::from_millis(16), true);
        assert_eq!(
            active,
            Some(RenderQuality::for_level(RenderQualityLevel::Balanced))
        );
    }

    #[test]
    fn render_quality_uses_five_distinct_ray_budgets() {
        let budgets = [
            RenderQualityLevel::Background,
            RenderQualityLevel::Performance,
            RenderQualityLevel::Balanced,
            RenderQualityLevel::High,
            RenderQualityLevel::Cinematic,
        ]
        .map(|level| RenderQuality::for_level(level).ray_steps);

        assert_eq!(budgets, [64, 72, 80, 88, 96]);
    }
}
