//! Coordinates drag visuals, recoverable file operations, and uninstall consent.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use bevy::prelude::*;

use crate::{
    file_interaction::{
        DropBatchRequested, DropVisualKind, FileInteractionSystems, VisualCommand,
        VisualOperationReady,
    },
    file_operations::{
        DropAnalysis, FileOperationCommand, FileOperationError, FileOperationErrorKind,
        FileOperationResult, FileOperationsWorker, UninstallCandidate, normalize_windows_path,
    },
    settings_ui::{
        FileActionStatusKind, FileActionUiState, ShowSettingsWindow, UninstallDecision,
        UninstallPrompt,
    },
};

const MAX_PENDING_ANALYSIS_BATCHES: usize = 8;

pub(crate) struct FileWorkflowPlugin;

impl Plugin for FileWorkflowPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FileOperationsBackend::new())
            .init_resource::<FileWorkflowState>()
            .configure_sets(
                Update,
                FileWorkflowSystems::Coordinate
                    .after(FileInteractionSystems::ObserveDrops)
                    .before(FileInteractionSystems::ReceiveCommands),
            )
            .add_systems(
                Update,
                (
                    report_backend_initialization,
                    accept_drop_batches,
                    poll_worker_results,
                    finalize_analyzed_batches,
                    handle_uninstall_decision,
                    handle_visual_operation_ready,
                )
                    .chain()
                    .in_set(FileWorkflowSystems::Coordinate),
            );
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FileWorkflowSystems {
    Coordinate,
}

#[derive(Resource)]
struct FileOperationsBackend {
    worker: Option<Mutex<FileOperationsWorker>>,
    initialization_error: Option<String>,
}

impl FileOperationsBackend {
    fn new() -> Self {
        match FileOperationsWorker::spawn() {
            Ok(worker) => Self {
                worker: Some(Mutex::new(worker)),
                initialization_error: None,
            },
            Err(error) => Self {
                worker: None,
                initialization_error: Some(error.to_string()),
            },
        }
    }

    fn send(&self, command: FileOperationCommand) -> Result<(), String> {
        let Some(worker) = &self.worker else {
            return Err(self
                .initialization_error
                .clone()
                .unwrap_or_else(|| "文件操作后台不可用".to_owned()));
        };
        worker
            .lock()
            .map_err(|_| "文件操作后台状态已损坏".to_owned())?
            .send(command)
            .map_err(|error| {
                warn!("file-operation command dispatch failed: {error}");
                match error.kind {
                    FileOperationErrorKind::WorkerDisconnected => {
                        "文件操作后台已停止，请重启 Sunk 后重试。".to_owned()
                    }
                    FileOperationErrorKind::Io => "文件操作等待队列已满，请稍后重试。".to_owned(),
                    _ => "文件操作后台拒绝了本次请求。".to_owned(),
                }
            })
    }

    fn drain_results(&mut self) -> Result<Vec<FileOperationResult>, String> {
        let Some(worker) = &self.worker else {
            return Ok(Vec::new());
        };
        let result = {
            let worker = worker
                .lock()
                .map_err(|_| "文件操作后台状态已损坏".to_owned())?;
            let mut results = Vec::new();
            loop {
                match worker.try_receive() {
                    Ok(Some(result)) => results.push(result),
                    Ok(None) => break Ok(results),
                    Err(error) => break Err(error.to_string()),
                }
            }
        };
        if let Err(error) = &result {
            self.initialization_error = Some(error.clone());
            self.worker = None;
        }
        result
    }
}

#[derive(Resource, Debug)]
struct FileWorkflowState {
    next_id: u64,
    next_batch_id: u64,
    analysis_batches: HashMap<u64, PendingAnalysisBatch>,
    request_batches: HashMap<u64, u64>,
    operations: HashMap<u64, PendingOperation>,
    revalidations: HashMap<u64, u64>,
    pending_uninstall_prompt: Option<u64>,
    backend_failed: bool,
}

impl Default for FileWorkflowState {
    fn default() -> Self {
        Self {
            next_id: 1,
            next_batch_id: 1,
            analysis_batches: HashMap::new(),
            request_batches: HashMap::new(),
            operations: HashMap::new(),
            revalidations: HashMap::new(),
            pending_uninstall_prompt: None,
            backend_failed: false,
        }
    }
}

impl FileWorkflowState {
    fn allocate_id(&mut self) -> Option<u64> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        Some(id)
    }

    fn allocate_batch_id(&mut self) -> Option<u64> {
        let id = self.next_batch_id;
        self.next_batch_id = self.next_batch_id.checked_add(1)?;
        Some(id)
    }
}

#[derive(Debug)]
struct PendingAnalysisBatch {
    drop_position: Vec2,
    items: Vec<PendingAnalysisItem>,
}

#[derive(Debug)]
struct PendingAnalysisItem {
    id: u64,
    path: PathBuf,
    outcome: Option<AnalysisOutcome>,
}

#[derive(Debug)]
enum AnalysisOutcome {
    Success(DropAnalysis),
    Failure(FileOperationError),
    DispatchFailure(String),
}

#[derive(Debug)]
struct PendingOperation {
    intent: OperationIntent,
    phase: OperationPhase,
}

#[derive(Debug)]
enum OperationIntent {
    Trash {
        path: PathBuf,
    },
    Uninstall {
        source_path: PathBuf,
        candidate: Box<UninstallCandidate>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationPhase {
    AwaitingConfirmation,
    Animating,
    Revalidating,
    Submitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalysisMix {
    TrashOnly,
    SingleUninstall,
    UnsafeMix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecisionOutcome {
    Ignored,
    Cancelled(u64),
    Authorized(u64),
    InvalidState(u64),
}

impl FileWorkflowState {
    fn apply_uninstall_decision(&mut self, decision: UninstallDecision) -> DecisionOutcome {
        let id = match decision {
            UninstallDecision::Confirm(id) | UninstallDecision::Cancel(id) => id,
        };
        if self.pending_uninstall_prompt != Some(id) {
            return DecisionOutcome::Ignored;
        }

        match decision {
            UninstallDecision::Cancel(_) => {
                self.pending_uninstall_prompt = None;
                self.operations.remove(&id);
                DecisionOutcome::Cancelled(id)
            }
            UninstallDecision::Confirm(_) => {
                let authorized = self.operations.get_mut(&id).is_some_and(|operation| {
                    if operation.phase != OperationPhase::AwaitingConfirmation
                        || !matches!(&operation.intent, OperationIntent::Uninstall { .. })
                    {
                        return false;
                    }
                    operation.phase = OperationPhase::Animating;
                    true
                });
                self.pending_uninstall_prompt = None;
                if authorized {
                    DecisionOutcome::Authorized(id)
                } else {
                    self.operations.remove(&id);
                    DecisionOutcome::InvalidState(id)
                }
            }
        }
    }
}

fn report_backend_initialization(
    backend: Res<FileOperationsBackend>,
    mut ui: ResMut<FileActionUiState>,
    mut reported: Local<bool>,
) {
    if *reported {
        return;
    }
    *reported = true;
    if backend.worker.is_none() {
        ui.set_status(
            FileActionStatusKind::Error,
            "文件操作不可用",
            "后台工作线程启动失败。黑洞渲染仍可使用，但不会处理拖入目标。",
        );
    }
}

fn accept_drop_batches(
    mut requests: MessageReader<DropBatchRequested>,
    backend: Res<FileOperationsBackend>,
    mut state: ResMut<FileWorkflowState>,
    mut ui: ResMut<FileActionUiState>,
    mut visuals: MessageWriter<VisualCommand>,
) {
    for request in requests.read() {
        if request.paths.is_empty() {
            continue;
        }
        if backend.worker.is_none() || state.backend_failed {
            ui.set_status(
                FileActionStatusKind::Error,
                "文件操作不可用",
                "后台工作线程当前不可用，本次拖入未执行任何操作。",
            );
            pulse_rejected_paths(
                &mut state,
                &request.paths,
                request.drop_position,
                &mut visuals,
            );
            continue;
        }
        if state.analysis_batches.len() >= MAX_PENDING_ANALYSIS_BATCHES {
            ui.set_status(
                FileActionStatusKind::Warning,
                "等待队列已满",
                "已有多个拖放批次正在检查，请稍后重试。本次未执行任何操作。",
            );
            pulse_rejected_paths(
                &mut state,
                &request.paths,
                request.drop_position,
                &mut visuals,
            );
            continue;
        }

        let Some(batch_id) = state.allocate_batch_id() else {
            reject_for_identifier_exhaustion(
                &mut state,
                &request.paths,
                request.drop_position,
                &mut ui,
                &mut visuals,
            );
            continue;
        };

        let mut items = Vec::with_capacity(request.paths.len());
        let mut exhausted = false;
        for path in &request.paths {
            let Some(id) = state.allocate_id() else {
                exhausted = true;
                break;
            };
            let dispatch = backend.send(FileOperationCommand::AnalyzeDrop {
                request_id: id,
                path: path.clone(),
            });
            let outcome = dispatch.err().map(AnalysisOutcome::DispatchFailure);
            if outcome.is_none() {
                state.request_batches.insert(id, batch_id);
            }
            items.push(PendingAnalysisItem {
                id,
                path: path.clone(),
                outcome,
            });
        }

        if exhausted {
            for item in &items {
                state.request_batches.remove(&item.id);
            }
            reject_for_identifier_exhaustion(
                &mut state,
                &request.paths,
                request.drop_position,
                &mut ui,
                &mut visuals,
            );
            continue;
        }

        state.analysis_batches.insert(
            batch_id,
            PendingAnalysisBatch {
                drop_position: request.drop_position,
                items,
            },
        );
        ui.set_status(
            FileActionStatusKind::Information,
            "正在检查拖入目标",
            format!(
                "正在安全检查 {} 个目标，检查完成前不会修改文件。",
                request.paths.len()
            ),
        );
    }
}

fn poll_worker_results(
    mut backend: ResMut<FileOperationsBackend>,
    mut state: ResMut<FileWorkflowState>,
    mut ui: ResMut<FileActionUiState>,
    mut visuals: MessageWriter<VisualCommand>,
) {
    let results = match backend.drain_results() {
        Ok(results) => results,
        Err(error) => {
            error!("file-operation worker disconnected: {error}");
            fail_all_work(&mut state, &mut ui, &mut visuals);
            return;
        }
    };

    for result in results {
        match result {
            FileOperationResult::DropAnalyzed { request_id, result } => {
                if let Some(visual_id) = state.revalidations.remove(&request_id) {
                    finish_uninstall_revalidation(
                        visual_id,
                        result,
                        &backend,
                        &mut state,
                        &mut ui,
                        &mut visuals,
                    );
                    continue;
                }

                let Some(batch_id) = state.request_batches.remove(&request_id) else {
                    continue;
                };
                let Some(batch) = state.analysis_batches.get_mut(&batch_id) else {
                    continue;
                };
                if let Some(item) = batch.items.iter_mut().find(|item| item.id == request_id) {
                    item.outcome = Some(match result {
                        Ok(analysis) => AnalysisOutcome::Success(analysis),
                        Err(error) => AnalysisOutcome::Failure(error),
                    });
                }
            }
            FileOperationResult::MovedToTrash {
                request_id,
                path,
                result,
            } => {
                state.operations.remove(&request_id);
                match result {
                    Ok(()) => {
                        visuals.write(VisualCommand::Complete {
                            id: request_id,
                            success: true,
                        });
                        ui.set_status(
                            FileActionStatusKind::Success,
                            "已移入回收站",
                            format!(
                                "“{}”已移入 Windows 回收站，可从回收站恢复。",
                                target_name(&path)
                            ),
                        );
                    }
                    Err(error) => {
                        error!("move to recycle bin failed: {error}");
                        visuals.write(VisualCommand::Complete {
                            id: request_id,
                            success: false,
                        });
                        ui.set_status(
                            FileActionStatusKind::Error,
                            "无法移入回收站",
                            analysis_error_detail(&error),
                        );
                    }
                }
            }
            FileOperationResult::UninstallStarted { request_id, result } => {
                state.operations.remove(&request_id);
                match result {
                    Ok(_) => {
                        visuals.write(VisualCommand::Complete {
                            id: request_id,
                            success: true,
                        });
                        ui.set_status(
                            FileActionStatusKind::Success,
                            "卸载程序已启动",
                            "请在 Windows 卸载程序中完成后续步骤。启动成功不代表软件已经卸载完成。",
                        );
                    }
                    Err(error) => {
                        error!("uninstaller launch failed: {error}");
                        visuals.write(VisualCommand::Complete {
                            id: request_id,
                            success: false,
                        });
                        ui.set_status(
                            FileActionStatusKind::Error,
                            "卸载程序启动失败",
                            analysis_error_detail(&error),
                        );
                    }
                }
            }
        }
    }
}

fn finalize_analyzed_batches(
    mut state: ResMut<FileWorkflowState>,
    mut ui: ResMut<FileActionUiState>,
    mut show_settings: MessageWriter<ShowSettingsWindow>,
    mut visuals: MessageWriter<VisualCommand>,
) {
    let ready = state
        .analysis_batches
        .iter()
        .filter(|(_, batch)| batch.items.iter().all(|item| item.outcome.is_some()))
        .map(|(batch_id, _)| *batch_id)
        .collect::<Vec<_>>();

    for batch_id in ready {
        let Some(batch) = state.analysis_batches.remove(&batch_id) else {
            continue;
        };
        for item in &batch.items {
            state.request_batches.remove(&item.id);
        }

        if let Some(detail) = first_analysis_failure(&batch) {
            reject_analysis_batch(&mut state, batch, detail, &mut ui, &mut visuals);
            continue;
        }

        let mix = analysis_mix(&batch);
        match mix {
            AnalysisMix::TrashOnly => {
                if trash_batch_has_overlapping_targets(&batch) {
                    reject_analysis_batch(
                        &mut state,
                        batch,
                        "批次中包含重复目标或互相包含的父子路径，请只保留彼此独立的目标。"
                            .to_owned(),
                        &mut ui,
                        &mut visuals,
                    );
                    continue;
                }
                let count = batch.items.len();
                for item in batch.items {
                    let Some(AnalysisOutcome::Success(DropAnalysis::Trashable(target))) =
                        item.outcome
                    else {
                        continue;
                    };
                    state.operations.insert(
                        item.id,
                        PendingOperation {
                            intent: OperationIntent::Trash { path: target.path },
                            phase: OperationPhase::Animating,
                        },
                    );
                    visuals.write(VisualCommand::Begin {
                        id: item.id,
                        kind: DropVisualKind::File,
                        start_position: batch.drop_position,
                    });
                }
                ui.set_status(
                    FileActionStatusKind::Information,
                    "目标已通过安全检查",
                    format!(
                        "{} 个目标正在被黑洞捕获，到达事件视界后将移入回收站。",
                        count
                    ),
                );
            }
            AnalysisMix::SingleUninstall => {
                let mut items = batch.items.into_iter();
                let Some(item) = items.next() else {
                    continue;
                };
                let Some(AnalysisOutcome::Success(DropAnalysis::Uninstall(candidate))) =
                    item.outcome
                else {
                    continue;
                };

                if state.pending_uninstall_prompt.is_some() {
                    reject_items(
                        &mut state,
                        [item.id],
                        [item.path.as_path()],
                        batch.drop_position,
                        &mut visuals,
                    );
                    ui.set_status(
                        FileActionStatusKind::Warning,
                        "已有卸载确认待处理",
                        "每次只能确认一个软件。本次拖入未启动任何卸载程序。",
                    );
                    continue;
                }

                let prompt = UninstallPrompt {
                    request_id: item.id,
                    application_name: candidate.entry.display_name.clone(),
                    publisher: candidate.entry.publisher.clone(),
                    install_location: candidate.entry.install_location.clone(),
                    source_path: item.path.clone(),
                };
                state.pending_uninstall_prompt = Some(item.id);
                state.operations.insert(
                    item.id,
                    PendingOperation {
                        intent: OperationIntent::Uninstall {
                            source_path: item.path,
                            candidate,
                        },
                        phase: OperationPhase::AwaitingConfirmation,
                    },
                );
                visuals.write(VisualCommand::Stage {
                    id: item.id,
                    kind: DropVisualKind::Application,
                    start_position: batch.drop_position,
                });
                ui.request_uninstall(prompt);
                ui.set_status(
                    FileActionStatusKind::Warning,
                    "等待卸载确认",
                    "已找到唯一的高置信软件记录。确认前不会启动任何程序，也不会删除快捷方式。",
                );
                show_settings.write(ShowSettingsWindow);
            }
            AnalysisMix::UnsafeMix => {
                reject_analysis_batch(
                    &mut state,
                    batch,
                    "一个批次不能混合文件和软件，也不能同时卸载多个软件。请分开拖入。".to_owned(),
                    &mut ui,
                    &mut visuals,
                );
            }
        }
    }
}

fn handle_uninstall_decision(
    mut state: ResMut<FileWorkflowState>,
    mut ui: ResMut<FileActionUiState>,
    mut visuals: MessageWriter<VisualCommand>,
) {
    let Some(decision) = ui.take_decision() else {
        return;
    };
    match state.apply_uninstall_decision(decision) {
        DecisionOutcome::Ignored => {}
        DecisionOutcome::Cancelled(id) => {
            visuals.write(VisualCommand::Reject { id });
            ui.set_status(
                FileActionStatusKind::Information,
                "已取消卸载",
                "没有启动任何程序，也没有修改或删除拖入的快捷方式。",
            );
        }
        DecisionOutcome::Authorized(id) => {
            visuals.write(VisualCommand::Authorize { id });
            ui.set_status(
                FileActionStatusKind::Information,
                "卸载已确认",
                "软件图标正在进入事件视界；启动前会再次核对 Windows 中的卸载记录。",
            );
        }
        DecisionOutcome::InvalidState(id) => {
            visuals.write(VisualCommand::Reject { id });
            ui.set_status(
                FileActionStatusKind::Error,
                "卸载确认已失效",
                "等待中的软件记录状态不一致，已恢复拖入图标且未启动任何程序。",
            );
        }
    }
}

fn handle_visual_operation_ready(
    mut ready: MessageReader<VisualOperationReady>,
    backend: Res<FileOperationsBackend>,
    mut state: ResMut<FileWorkflowState>,
    mut ui: ResMut<FileActionUiState>,
    mut visuals: MessageWriter<VisualCommand>,
) {
    for event in ready.read().copied() {
        let action = state.operations.get(&event.id).and_then(|operation| {
            (operation.phase == OperationPhase::Animating).then(|| match &operation.intent {
                OperationIntent::Trash { path } => ReadyAction::Trash(path.clone()),
                OperationIntent::Uninstall { source_path, .. } => {
                    ReadyAction::Revalidate(source_path.clone())
                }
            })
        });
        let Some(action) = action else {
            continue;
        };

        match action {
            ReadyAction::Trash(path) => {
                let command = FileOperationCommand::MoveToTrash {
                    request_id: event.id,
                    path,
                };
                if let Err(error) = backend.send(command) {
                    fail_operation(event.id, error, &mut state, &mut ui, &mut visuals);
                } else if let Some(operation) = state.operations.get_mut(&event.id) {
                    operation.phase = OperationPhase::Submitted;
                }
            }
            ReadyAction::Revalidate(source_path) => {
                let Some(revalidation_id) = state.allocate_id() else {
                    fail_operation(
                        event.id,
                        "内部请求编号已耗尽".to_owned(),
                        &mut state,
                        &mut ui,
                        &mut visuals,
                    );
                    continue;
                };
                let command = FileOperationCommand::AnalyzeDrop {
                    request_id: revalidation_id,
                    path: source_path,
                };
                if let Err(error) = backend.send(command) {
                    fail_operation(event.id, error, &mut state, &mut ui, &mut visuals);
                } else {
                    state.revalidations.insert(revalidation_id, event.id);
                    if let Some(operation) = state.operations.get_mut(&event.id) {
                        operation.phase = OperationPhase::Revalidating;
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
enum ReadyAction {
    Trash(PathBuf),
    Revalidate(PathBuf),
}

fn finish_uninstall_revalidation(
    visual_id: u64,
    result: Result<DropAnalysis, FileOperationError>,
    backend: &FileOperationsBackend,
    state: &mut FileWorkflowState,
    ui: &mut FileActionUiState,
    visuals: &mut MessageWriter<VisualCommand>,
) {
    let expected = state.operations.get(&visual_id).and_then(|operation| {
        if operation.phase != OperationPhase::Revalidating {
            return None;
        }
        match &operation.intent {
            OperationIntent::Uninstall { candidate, .. } => Some(candidate.clone()),
            OperationIntent::Trash { .. } => None,
        }
    });
    let Some(expected) = expected else {
        return;
    };

    let current = match result {
        Ok(DropAnalysis::Uninstall(candidate)) if candidate == expected => candidate,
        Ok(DropAnalysis::Uninstall(_)) => {
            fail_operation(
                visual_id,
                "确认后软件的卸载记录发生了变化".to_owned(),
                state,
                ui,
                visuals,
            );
            return;
        }
        Ok(DropAnalysis::Trashable(_)) => {
            fail_operation(
                visual_id,
                "拖入目标不再对应同一个软件".to_owned(),
                state,
                ui,
                visuals,
            );
            return;
        }
        Err(error) => {
            error!("uninstall revalidation failed: {error}");
            fail_operation(visual_id, analysis_error_detail(&error), state, ui, visuals);
            return;
        }
    };

    let command = FileOperationCommand::LaunchUninstall {
        request_id: visual_id,
        candidate: current,
    };
    if let Err(error) = backend.send(command) {
        fail_operation(visual_id, error, state, ui, visuals);
    } else if let Some(operation) = state.operations.get_mut(&visual_id) {
        operation.phase = OperationPhase::Submitted;
    }
}

fn analysis_mix(batch: &PendingAnalysisBatch) -> AnalysisMix {
    let mut trash = 0;
    let mut uninstall = 0;
    for item in &batch.items {
        match item.outcome.as_ref() {
            Some(AnalysisOutcome::Success(DropAnalysis::Trashable(_))) => trash += 1,
            Some(AnalysisOutcome::Success(DropAnalysis::Uninstall(_))) => uninstall += 1,
            _ => return AnalysisMix::UnsafeMix,
        }
    }
    match (trash, uninstall, batch.items.len()) {
        (_, 0, total) if trash == total && total > 0 => AnalysisMix::TrashOnly,
        (0, 1, 1) => AnalysisMix::SingleUninstall,
        _ => AnalysisMix::UnsafeMix,
    }
}

fn trash_batch_has_overlapping_targets(batch: &PendingAnalysisBatch) -> bool {
    let paths = batch
        .items
        .iter()
        .filter_map(|item| match item.outcome.as_ref() {
            Some(AnalysisOutcome::Success(DropAnalysis::Trashable(target))) => {
                Some(target.path.as_path())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    paths.len() != batch.items.len() || normalized_paths_overlap(&paths)
}

fn normalized_paths_overlap(paths: &[&Path]) -> bool {
    let normalized = paths
        .iter()
        .map(|path| normalize_windows_path(path))
        .collect::<Option<Vec<_>>>();
    let Some(normalized) = normalized else {
        return true;
    };

    for left_index in 0..normalized.len() {
        for right_index in left_index + 1..normalized.len() {
            let left = &normalized[left_index];
            let right = &normalized[right_index];
            if normalized_path_is_within(left, right) || normalized_path_is_within(right, left) {
                return true;
            }
        }
    }
    false
}

fn normalized_path_is_within(path: &str, directory: &str) -> bool {
    path == directory
        || path
            .strip_prefix(directory)
            .is_some_and(|remainder| remainder.starts_with('\\'))
}

fn first_analysis_failure(batch: &PendingAnalysisBatch) -> Option<String> {
    batch
        .items
        .iter()
        .find_map(|item| match item.outcome.as_ref() {
            Some(AnalysisOutcome::Failure(error)) => {
                error!("dropped target was rejected: {error}");
                Some(analysis_error_detail(error))
            }
            Some(AnalysisOutcome::DispatchFailure(error)) => Some(error.clone()),
            Some(AnalysisOutcome::Success(_)) | None => None,
        })
}

fn reject_analysis_batch(
    state: &mut FileWorkflowState,
    batch: PendingAnalysisBatch,
    detail: String,
    ui: &mut FileActionUiState,
    visuals: &mut MessageWriter<VisualCommand>,
) {
    let ids = batch.items.iter().map(|item| item.id).collect::<Vec<_>>();
    let paths = batch
        .items
        .iter()
        .map(|item| item.path.as_path())
        .collect::<Vec<_>>();
    reject_items(state, ids, paths, batch.drop_position, visuals);
    ui.set_status(
        FileActionStatusKind::Error,
        "拖入目标已拒绝",
        format!("{detail} 本次批次未执行任何操作。"),
    );
}

fn pulse_rejected_paths(
    state: &mut FileWorkflowState,
    paths: &[PathBuf],
    position: Vec2,
    visuals: &mut MessageWriter<VisualCommand>,
) {
    let mut ids = Vec::new();
    let mut valid_paths = Vec::new();
    for path in paths {
        let Some(id) = state.allocate_id() else {
            break;
        };
        ids.push(id);
        valid_paths.push(path.as_path());
    }
    reject_items(state, ids, valid_paths, position, visuals);
}

fn reject_items<'a>(
    _state: &mut FileWorkflowState,
    ids: impl IntoIterator<Item = u64>,
    paths: impl IntoIterator<Item = &'a Path>,
    position: Vec2,
    visuals: &mut MessageWriter<VisualCommand>,
) {
    for (id, path) in ids.into_iter().zip(paths) {
        visuals.write(VisualCommand::Stage {
            id,
            kind: visual_kind_hint(path),
            start_position: position,
        });
        visuals.write(VisualCommand::Reject { id });
    }
}

fn reject_for_identifier_exhaustion(
    state: &mut FileWorkflowState,
    paths: &[PathBuf],
    position: Vec2,
    ui: &mut FileActionUiState,
    visuals: &mut MessageWriter<VisualCommand>,
) {
    pulse_rejected_paths(state, paths, position, visuals);
    ui.set_status(
        FileActionStatusKind::Error,
        "内部请求编号已耗尽",
        "为避免无法追踪操作，本次拖入未执行任何操作。请重启 Sunk。",
    );
}

fn fail_operation(
    id: u64,
    detail: String,
    state: &mut FileWorkflowState,
    ui: &mut FileActionUiState,
    visuals: &mut MessageWriter<VisualCommand>,
) {
    state.operations.remove(&id);
    state.revalidations.retain(|_, visual_id| *visual_id != id);
    if state.pending_uninstall_prompt == Some(id) {
        state.pending_uninstall_prompt = None;
        ui.dismiss_prompt(id);
    }
    visuals.write(VisualCommand::Complete { id, success: false });
    ui.set_status(
        FileActionStatusKind::Error,
        "操作未完成",
        format!("{detail} 未执行不可恢复删除。"),
    );
}

fn fail_all_work(
    state: &mut FileWorkflowState,
    ui: &mut FileActionUiState,
    visuals: &mut MessageWriter<VisualCommand>,
) {
    state.backend_failed = true;
    let batches = std::mem::take(&mut state.analysis_batches);
    for (_, batch) in batches {
        let ids = batch.items.iter().map(|item| item.id).collect::<Vec<_>>();
        let paths = batch
            .items
            .iter()
            .map(|item| item.path.as_path())
            .collect::<Vec<_>>();
        reject_items(state, ids, paths, batch.drop_position, visuals);
    }
    state.request_batches.clear();
    state.revalidations.clear();

    let operations = std::mem::take(&mut state.operations);
    for (id, operation) in operations {
        if operation.phase == OperationPhase::AwaitingConfirmation {
            visuals.write(VisualCommand::Reject { id });
        } else {
            visuals.write(VisualCommand::Complete { id, success: false });
        }
    }
    if let Some(id) = state.pending_uninstall_prompt.take() {
        ui.dismiss_prompt(id);
    }
    ui.set_status(
        FileActionStatusKind::Error,
        "文件操作后台已停止",
        "所有等待中的操作均已取消，没有执行不可恢复删除。请重启 Sunk 后重试。",
    );
}

fn visual_kind_hint(path: &Path) -> DropVisualKind {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension)
            if extension.eq_ignore_ascii_case("lnk")
                || extension.eq_ignore_ascii_case("exe")
                || extension.eq_ignore_ascii_case("appref-ms")
                || extension.eq_ignore_ascii_case("url")
                || extension.eq_ignore_ascii_case("website") =>
        {
            DropVisualKind::Application
        }
        _ => DropVisualKind::File,
    }
}

fn analysis_error_detail(error: &FileOperationError) -> String {
    match error.kind {
        FileOperationErrorKind::UnsupportedPlatform => {
            "当前文件操作后端仅支持 Windows。".to_owned()
        }
        FileOperationErrorKind::InvalidPath => "路径无效或不是本地绝对路径。".to_owned(),
        FileOperationErrorKind::NotFound => "拖入目标不存在或已经被移动。".to_owned(),
        FileOperationErrorKind::ProtectedPath => {
            "该目标位于系统、程序或 Sunk 自身的保护范围内。".to_owned()
        }
        FileOperationErrorKind::ReparsePoint => {
            "为避免跟随链接处理错误目标，暂不接受符号链接或重解析点。".to_owned()
        }
        FileOperationErrorKind::NotTrashable => {
            "该位置不支持可靠的 Windows 回收站操作。".to_owned()
        }
        FileOperationErrorKind::ShortcutResolution => {
            "无法把快捷方式可靠解析为本地软件。".to_owned()
        }
        FileOperationErrorKind::RegistryRead => {
            "无法读取 Windows 中登记的软件卸载信息。".to_owned()
        }
        FileOperationErrorKind::NoUninstallCandidate => {
            "没有找到唯一且高置信的 Windows 卸载记录，未执行任何操作。".to_owned()
        }
        FileOperationErrorKind::AmbiguousUninstallCandidate => {
            "找到多个可能的软件记录，无法安全确定卸载对象。".to_owned()
        }
        FileOperationErrorKind::UnsafeUninstallCommand => {
            "软件登记的卸载入口不符合安全启动规则。".to_owned()
        }
        FileOperationErrorKind::Io => "Windows 拒绝了本次文件或进程操作。".to_owned(),
        FileOperationErrorKind::WorkerDisconnected => {
            "文件操作后台意外停止，请重启 Sunk。".to_owned()
        }
    }
}

fn target_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::file_operations::{
        DroppedItemKind, RegistryHive, RegistryView, UninstallEntry, UninstallLaunchPlan,
        ValidatedTrashTarget,
    };

    fn trash(path: &str) -> AnalysisOutcome {
        AnalysisOutcome::Success(DropAnalysis::Trashable(ValidatedTrashTarget {
            path: PathBuf::from(path),
            kind: DroppedItemKind::RegularFile,
        }))
    }

    fn uninstall_candidate() -> Box<UninstallCandidate> {
        Box::new(UninstallCandidate {
            entry: UninstallEntry {
                hive: RegistryHive::CurrentUser,
                view: RegistryView::Registry64,
                key_name: "Example".to_owned(),
                display_name: "Example".to_owned(),
                publisher: None,
                install_location: Some(PathBuf::from(r"C:\Program Files\Example")),
                display_icon: None,
                uninstall_string: r#""C:\Program Files\Example\uninstall.exe""#.to_owned(),
                windows_installer: false,
                system_component: false,
                no_remove: false,
                parent_key_name: None,
                release_type: None,
            },
            score: 500,
            evidence: Vec::new(),
            launch_plan: UninstallLaunchPlan::Exe {
                executable: PathBuf::from(r"C:\Program Files\Example\uninstall.exe"),
                arguments: Vec::<OsString>::new(),
            },
        })
    }

    fn uninstall() -> AnalysisOutcome {
        AnalysisOutcome::Success(DropAnalysis::Uninstall(uninstall_candidate()))
    }

    fn awaiting_uninstall_state(id: u64) -> FileWorkflowState {
        let mut state = FileWorkflowState {
            pending_uninstall_prompt: Some(id),
            ..default()
        };
        state.operations.insert(
            id,
            PendingOperation {
                intent: OperationIntent::Uninstall {
                    source_path: PathBuf::from(r"C:\Desktop\Example.lnk"),
                    candidate: uninstall_candidate(),
                },
                phase: OperationPhase::AwaitingConfirmation,
            },
        );
        state
    }

    fn analyzed_batch(outcomes: Vec<AnalysisOutcome>) -> PendingAnalysisBatch {
        PendingAnalysisBatch {
            drop_position: Vec2::ZERO,
            items: outcomes
                .into_iter()
                .enumerate()
                .map(|(index, outcome)| PendingAnalysisItem {
                    id: index as u64 + 1,
                    path: PathBuf::from(format!(r"C:\drop\{index}")),
                    outcome: Some(outcome),
                })
                .collect(),
        }
    }

    #[test]
    fn ordinary_files_can_share_one_batch() {
        let batch = analyzed_batch(vec![trash(r"C:\drop\a"), trash(r"C:\drop\b")]);
        assert_eq!(analysis_mix(&batch), AnalysisMix::TrashOnly);
    }

    #[test]
    fn one_application_requires_the_confirmation_path() {
        let batch = analyzed_batch(vec![uninstall()]);
        assert_eq!(analysis_mix(&batch), AnalysisMix::SingleUninstall);
    }

    #[test]
    fn mixed_or_multiple_application_batches_are_rejected() {
        let mixed = analyzed_batch(vec![trash(r"C:\drop\a"), uninstall()]);
        let multiple = analyzed_batch(vec![uninstall(), uninstall()]);
        assert_eq!(analysis_mix(&mixed), AnalysisMix::UnsafeMix);
        assert_eq!(analysis_mix(&multiple), AnalysisMix::UnsafeMix);
    }

    #[test]
    fn cancelling_uninstall_clears_the_prompt_and_operation() {
        let mut state = awaiting_uninstall_state(17);

        assert_eq!(
            state.apply_uninstall_decision(UninstallDecision::Cancel(17)),
            DecisionOutcome::Cancelled(17)
        );
        assert_eq!(state.pending_uninstall_prompt, None);
        assert!(!state.operations.contains_key(&17));
    }

    #[test]
    fn stale_uninstall_decision_does_not_affect_the_current_prompt() {
        let mut state = awaiting_uninstall_state(17);

        assert_eq!(
            state.apply_uninstall_decision(UninstallDecision::Cancel(16)),
            DecisionOutcome::Ignored
        );
        assert_eq!(state.pending_uninstall_prompt, Some(17));
        assert_eq!(
            state.operations.get(&17).map(|operation| operation.phase),
            Some(OperationPhase::AwaitingConfirmation)
        );
    }

    #[test]
    fn confirming_uninstall_authorizes_the_capture_animation() {
        let mut state = awaiting_uninstall_state(17);

        assert_eq!(
            state.apply_uninstall_decision(UninstallDecision::Confirm(17)),
            DecisionOutcome::Authorized(17)
        );
        assert_eq!(state.pending_uninstall_prompt, None);
        assert_eq!(
            state.operations.get(&17).map(|operation| operation.phase),
            Some(OperationPhase::Animating)
        );
    }

    #[test]
    fn trash_batch_rejects_case_insensitive_duplicate_paths() {
        assert!(normalized_paths_overlap(&[
            Path::new(r"C:\drop\Report.txt"),
            Path::new(r"c:\DROP\report.TXT"),
        ]));
    }

    #[test]
    fn trash_batch_rejects_parent_and_child_paths() {
        assert!(normalized_paths_overlap(&[
            Path::new(r"C:\drop\folder"),
            Path::new(r"C:\drop\folder\child.txt"),
        ]));
    }

    #[test]
    fn trash_batch_allows_independent_sibling_paths() {
        assert!(!normalized_paths_overlap(&[
            Path::new(r"C:\drop\folder-a"),
            Path::new(r"C:\drop\folder-b"),
        ]));
    }

    #[test]
    fn visual_kind_hint_uses_application_feedback_for_application_links() {
        for path in [
            r"C:\Desktop\Example.LNK",
            r"C:\Desktop\Example.URL",
            r"C:\Desktop\Example.website",
        ] {
            assert_eq!(
                visual_kind_hint(Path::new(path)),
                DropVisualKind::Application
            );
        }
        assert_eq!(
            visual_kind_hint(Path::new(r"C:\Desktop\notes.txt")),
            DropVisualKind::File
        );
    }
}
