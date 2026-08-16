use std::{path::PathBuf, sync::Arc};

use bevy::{
    anti_alias::smaa::{Smaa, SmaaPreset},
    app::AppExit,
    camera::{ClearColorConfig, RenderTarget, visibility::RenderLayers},
    ecs::schedule::ScheduleLabel,
    prelude::*,
    window::{
        Monitor, PresentMode, PrimaryMonitor, PrimaryWindow, WindowCloseRequested, WindowLevel,
        WindowRef, WindowResolution,
    },
};
use bevy_egui::{EguiContext, EguiGlobalSettings, EguiMultipassSchedule, EguiPlugin, egui};

use crate::desktop_capture::DesktopCaptureState;
use crate::settings::{AntiAliasingMode, BlackHoleSettings, RenderQuality};

#[cfg(target_os = "windows")]
#[path = "system_tray.rs"]
mod system_tray;

pub const SETTINGS_WINDOW_TITLE: &str = "Sunk 设置";

pub struct SettingsUiPlugin;

#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct ShowSettingsWindow;

#[derive(Debug, Clone)]
pub(crate) struct UninstallPrompt {
    pub request_id: u64,
    pub application_name: String,
    pub publisher: Option<String>,
    pub install_location: Option<PathBuf>,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UninstallDecision {
    Confirm(u64),
    Cancel(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileActionStatusKind {
    Information,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub(crate) struct FileActionStatus {
    pub kind: FileActionStatusKind,
    pub title: String,
    pub detail: String,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct FileActionUiState {
    prompt: Option<UninstallPrompt>,
    decision: Option<UninstallDecision>,
    status: Option<FileActionStatus>,
    focus_cancel: bool,
}

impl FileActionUiState {
    pub(crate) fn request_uninstall(&mut self, prompt: UninstallPrompt) {
        self.prompt = Some(prompt);
        self.decision = None;
        self.focus_cancel = true;
    }

    pub(crate) fn take_decision(&mut self) -> Option<UninstallDecision> {
        self.decision.take()
    }

    fn cancel_pending_uninstall(&mut self) -> Option<u64> {
        let request_id = self.prompt.take()?.request_id;
        self.decision = Some(UninstallDecision::Cancel(request_id));
        self.focus_cancel = true;
        Some(request_id)
    }

    fn resolve_prompt(&mut self, decision: UninstallDecision) {
        let request_id = match decision {
            UninstallDecision::Confirm(request_id) | UninstallDecision::Cancel(request_id) => {
                request_id
            }
        };
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.request_id == request_id)
        {
            self.prompt = None;
            self.decision = Some(decision);
            self.focus_cancel = true;
        }
    }

    pub(crate) fn dismiss_prompt(&mut self, request_id: u64) {
        let prompt_matches = self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.request_id == request_id);
        let decision_matches = self.decision.is_some_and(|decision| {
            matches!(
                decision,
                UninstallDecision::Confirm(id) | UninstallDecision::Cancel(id)
                    if id == request_id
            )
        });
        if prompt_matches {
            self.prompt = None;
        }
        if decision_matches {
            self.decision = None;
        }
        if prompt_matches || decision_matches {
            self.focus_cancel = true;
        }
    }

    pub(crate) fn set_status(
        &mut self,
        kind: FileActionStatusKind,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.status = Some(FileActionStatus {
            kind,
            title: title.into(),
            detail: detail.into(),
        });
    }
}

impl Plugin for SettingsUiPlugin {
    fn build(&self, app: &mut App) {
        assert!(
            app.is_plugin_added::<EguiPlugin>(),
            "SettingsUiPlugin must be added after EguiPlugin"
        );

        // The primary transparent renderer does not host an Egui overlay. Keep
        // context creation explicit so input and output bind only to this window.
        app.world_mut()
            .resource_mut::<EguiGlobalSettings>()
            .auto_create_primary_context = false;

        #[cfg(target_os = "windows")]
        app.add_plugins(system_tray::SystemTrayPlugin);

        app.init_resource::<BlackHoleSettings>()
            .init_resource::<PendingSettingsFocus>()
            .init_resource::<ChineseFontState>()
            .init_resource::<FileActionUiState>()
            .add_message::<ShowSettingsWindow>()
            .add_systems(Startup, spawn_settings_window)
            .add_systems(
                Update,
                (
                    apply_pending_focus,
                    toggle_settings_window,
                    show_requested_settings_window,
                    hide_settings_on_close,
                    exit_on_primary_close,
                    sync_primary_camera_anti_aliasing,
                )
                    .chain(),
            )
            .add_systems(
                SettingsEguiPass,
                settings_panel.run_if(settings_window_is_visible),
            );
    }
}

/// Marker used by desktop capture to exclude the settings surface and by input
/// systems to distinguish it from the transparent primary window.
#[derive(Component, Debug)]
pub struct SettingsWindow;

#[derive(Component, Debug)]
pub struct SettingsCamera;

#[derive(Component)]
struct SettingsEguiContext;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct SettingsEguiPass;

#[derive(Resource, Default)]
struct PendingSettingsFocus(bool);

#[derive(Resource, Default)]
struct ChineseFontState {
    attempted: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SettingsTab {
    #[default]
    General,
    Display,
    Quality,
    About,
}

impl SettingsTab {
    const ALL: [Self; 4] = [Self::General, Self::Display, Self::Quality, Self::About];

    const fn label(self) -> &'static str {
        match self {
            Self::General => "通用",
            Self::Display => "显示",
            Self::Quality => "画质",
            Self::About => "关于",
        }
    }
}

fn spawn_settings_window(
    mut commands: Commands,
    primary_monitor: Option<Single<&Monitor, With<PrimaryMonitor>>>,
) {
    let work_area = primary_monitor
        .as_deref()
        .map(|monitor| primary_monitor_work_area().unwrap_or_else(|| monitor.physical_size()))
        .unwrap_or(UVec2::new(1_920, 1_080));
    let scale_factor = primary_monitor
        .as_deref()
        .map_or(1.0, |monitor| monitor.scale_factor);
    let settings_size = settings_window_logical_size(work_area, scale_factor);

    let window = commands
        .spawn((
            Window {
                title: SETTINGS_WINDOW_TITLE.into(),
                name: Some("sunk-settings".into()),
                resolution: WindowResolution::new(settings_size.x, settings_size.y),
                present_mode: PresentMode::AutoVsync,
                transparent: false,
                decorations: true,
                resizable: true,
                visible: true,
                focused: true,
                // The transparent renderer is also topmost. Keeping the settings
                // window in the same level allows it to stay above when focused.
                window_level: WindowLevel::AlwaysOnTop,
                skip_taskbar: false,
                ..default()
            },
            SettingsWindow,
        ))
        .id();

    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb(0.020, 0.024, 0.034)),
            ..default()
        },
        RenderTarget::Window(WindowRef::Entity(window)),
        // EguiMultipassSchedule requires and inserts EguiContext for this camera.
        EguiMultipassSchedule::new(SettingsEguiPass),
        SettingsEguiContext,
        SettingsCamera,
        RenderLayers::layer(31),
        Msaa::Off,
    ));
}

fn settings_window_logical_size(work_area: UVec2, scale_factor: f64) -> UVec2 {
    const DESIRED: UVec2 = UVec2::new(520, 720);
    // rcWork already excludes the taskbar. Reserve physical pixels for the title
    // bar and resize frame, then convert the remaining client area to logical UI
    // units using the monitor selected by winit.
    const NON_CLIENT_RESERVE: UVec2 = UVec2::new(32, 96);
    let available_client = work_area.saturating_sub(NON_CLIENT_RESERVE);
    let scale = scale_factor.clamp(0.5, 4.0) as f32;
    let maximum_logical = (available_client.as_vec2() / scale)
        .floor()
        .max(Vec2::ONE)
        .as_uvec2();
    UVec2::new(
        DESIRED.x.min(maximum_logical.x),
        DESIRED.y.min(maximum_logical.y),
    )
}

#[cfg(target_os = "windows")]
fn primary_monitor_work_area() -> Option<UVec2> {
    use std::mem::size_of;

    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint},
    };

    // SAFETY: MONITOR_DEFAULTTOPRIMARY guarantees a monitor for any point and
    // `info` advertises the correct writable structure size.
    let monitor = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) };
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    Some(UVec2::new(
        (info.rcWork.right - info.rcWork.left).max(1) as u32,
        (info.rcWork.bottom - info.rcWork.top).max(1) as u32,
    ))
}

#[cfg(not(target_os = "windows"))]
fn primary_monitor_work_area() -> Option<UVec2> {
    None
}

fn settings_window_is_visible(windows: Query<&Window, With<SettingsWindow>>) -> bool {
    windows.iter().any(|window| window.visible)
}

/// Focus is applied on the frame after a hidden window becomes visible. Bevy's
/// winit synchronization focuses before changing visibility in a single frame,
/// which is unreliable for a previously hidden native window on Windows.
fn apply_pending_focus(
    mut pending: ResMut<PendingSettingsFocus>,
    mut windows: Query<&mut Window, With<SettingsWindow>>,
) {
    if !pending.0 {
        return;
    }

    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if window.visible {
        window.focused = true;
        pending.0 = false;
    }
}

fn toggle_settings_window(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut pending: ResMut<PendingSettingsFocus>,
    mut file_actions: ResMut<FileActionUiState>,
    mut windows: Query<&mut Window, With<SettingsWindow>>,
) {
    let Ok(mut window) = windows.single_mut() else {
        return;
    };

    if keyboard.just_pressed(KeyCode::F1) {
        if window.visible {
            hide_settings_window(&mut window, &mut pending, &mut file_actions);
        } else {
            file_actions.cancel_pending_uninstall();
            window.set_minimized(false);
            window.visible = true;
            pending.0 = true;
        }
    } else if window.focused && keyboard.just_pressed(KeyCode::Escape) {
        hide_settings_window(&mut window, &mut pending, &mut file_actions);
    }
}

fn hide_settings_window(
    window: &mut Window,
    pending: &mut PendingSettingsFocus,
    file_actions: &mut FileActionUiState,
) {
    file_actions.cancel_pending_uninstall();
    window.set_minimized(false);
    window.visible = false;
    window.focused = false;
    pending.0 = false;
}

fn show_requested_settings_window(
    mut requests: MessageReader<ShowSettingsWindow>,
    mut pending: ResMut<PendingSettingsFocus>,
    mut windows: Query<&mut Window, With<SettingsWindow>>,
) {
    if requests.read().next().is_none() {
        return;
    }

    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    window.set_minimized(false);
    window.visible = true;
    // `apply_pending_focus` already ran this frame. The next frame focuses the
    // now-visible native HWND after winit has applied the visibility change.
    pending.0 = true;
}

/// `WindowPlugin::close_when_requested` must be disabled by the app so Bevy's
/// default Last-stage handler does not despawn this entity after it is hidden.
fn hide_settings_on_close(
    mut close_requests: MessageReader<WindowCloseRequested>,
    mut pending: ResMut<PendingSettingsFocus>,
    mut file_actions: ResMut<FileActionUiState>,
    mut windows: Query<(Entity, &mut Window), With<SettingsWindow>>,
) {
    let Ok((entity, mut window)) = windows.single_mut() else {
        return;
    };

    if close_requests.read().any(|event| event.window == entity) {
        hide_settings_window(&mut window, &mut pending, &mut file_actions);
    }
}

fn exit_on_primary_close(
    mut close_requests: MessageReader<WindowCloseRequested>,
    primary_window: Single<Entity, With<PrimaryWindow>>,
    mut app_exit: MessageWriter<AppExit>,
) {
    if close_requests
        .read()
        .any(|event| event.window == *primary_window)
    {
        app_exit.write(AppExit::Success);
    }
}

/// Keep post-process AA attached only to the camera that renders the primary
/// transparent window. SSAA is evaluated inside the ray marcher, so it must not
/// be combined with SMAA implicitly.
fn sync_primary_camera_anti_aliasing(
    settings: Res<BlackHoleSettings>,
    cameras: Query<(Entity, &RenderTarget, Option<&Smaa>), Without<SettingsCamera>>,
    mut commands: Commands,
) {
    for (entity, render_target, current_smaa) in &cameras {
        if !matches!(render_target, RenderTarget::Window(WindowRef::Primary)) {
            continue;
        }

        if settings.anti_aliasing.uses_smaa() {
            if current_smaa.map(|smaa| smaa.preset) != Some(SmaaPreset::High) {
                commands.entity(entity).insert(Smaa {
                    preset: SmaaPreset::High,
                });
            }
        } else if current_smaa.is_some() {
            commands.entity(entity).remove::<Smaa>();
        }
    }
}

fn settings_panel(
    mut context: Single<&mut EguiContext, With<SettingsEguiContext>>,
    mut settings: ResMut<BlackHoleSettings>,
    desktop_capture: Res<DesktopCaptureState>,
    mut file_actions: ResMut<FileActionUiState>,
    mut selected_tab: Local<SettingsTab>,
    mut chinese_font: ResMut<ChineseFontState>,
) {
    let context = context.get_mut();
    configure_chinese_font(context, &mut chinese_font);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(18, 20, 26);
    visuals.extreme_bg_color = egui::Color32::from_rgb(11, 13, 18);
    context.set_visuals(visuals);
    let mut viewport_ui = egui::Ui::new(
        context.clone(),
        "settings_viewport".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(context.viewport_rect()),
    );

    egui::CentralPanel::default().show(&mut viewport_ui, |ui| {
        ui.heading("Sunk 黑洞");
        ui.label("桌面黑洞实时渲染设置");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            for tab in SettingsTab::ALL {
                if ui
                    .selectable_label(*selected_tab == tab, tab.label())
                    .clicked()
                {
                    *selected_tab = tab;
                }
            }
        });
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            match *selected_tab {
                SettingsTab::General => {
                    general_tab(ui, &mut settings, file_actions.status.as_ref())
                }
                SettingsTab::Display => display_tab(ui, &mut settings, &desktop_capture),
                SettingsTab::Quality => quality_tab(ui, &mut settings),
                SettingsTab::About => about_tab(ui),
            }

            ui.add_space(14.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("恢复默认设置").clicked() {
                    *settings = BlackHoleSettings::default();
                }
                ui.weak("按 F1 显示或隐藏设置窗口");
            });
            setting_description(ui, "恢复操作会立即重置本页及其他页面中的所有渲染参数。");
        });
    });

    uninstall_confirmation_modal(context, &mut file_actions);

    settings.sanitize();
}

fn general_tab(
    ui: &mut egui::Ui,
    settings: &mut BlackHoleSettings,
    file_action_status: Option<&FileActionStatus>,
) {
    section_heading(ui, "黑洞行为");
    described_slider(
        ui,
        egui::Slider::new(&mut settings.apparent_size, 0.25..=3.0)
            .logarithmic(true)
            .text("黑洞大小"),
        "调整黑洞、吸积盘和引力透镜的整体表观尺寸。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.animation_speed, 0.0..=4.0).text("流动速度"),
        "控制吸积盘物质绕黑洞旋转的时间倍率；设为 0 可冻结流动。",
    );

    section_heading(ui, "快捷操作");
    ui.label("右键拖动：改变观察方向");
    ui.label("滚轮：缩放黑洞");
    ui.label("空格：暂停或继续动画");
    ui.label("R：恢复默认参数");
    ui.label("F1：显示或隐藏设置窗口");

    section_heading(ui, "文件交互状态");
    if let Some(status) = file_action_status {
        let color = match status.kind {
            FileActionStatusKind::Information => egui::Color32::from_rgb(135, 185, 235),
            FileActionStatusKind::Success => egui::Color32::from_rgb(120, 205, 160),
            FileActionStatusKind::Warning => egui::Color32::from_rgb(225, 185, 105),
            FileActionStatusKind::Error => egui::Color32::from_rgb(235, 115, 115),
        };
        ui.label(egui::RichText::new(&status.title).strong().color(color));
        setting_description(ui, &status.detail);
    } else {
        setting_description(ui, "尚未处理文件或软件图标。拖入目标后，结果会显示在这里。");
    }
}

fn uninstall_confirmation_modal(context: &egui::Context, state: &mut FileActionUiState) {
    let Some(prompt) = state.prompt.clone() else {
        return;
    };

    let mut decision = None;
    let response = egui::Modal::new(egui::Id::new("sunk-uninstall-confirmation")).show(
        context,
        |ui| {
            ui.set_max_width(440.0);
            ui.heading("确认卸载软件");
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&prompt.application_name).strong());
            if let Some(publisher) = prompt
                .publisher
                .as_deref()
                .filter(|publisher| !publisher.trim().is_empty())
            {
                ui.label(format!("发布者（注册信息，未验证）：{publisher}"));
            } else {
                ui.label("发布者（未验证）：未提供");
            }
            if let Some(location) = &prompt.install_location {
                ui.add(
                    egui::Label::new(format!("安装位置：{}", location.display())).wrap(),
                );
            }
            ui.add(
                egui::Label::new(format!("拖入来源：{}", prompt.source_path.display())).wrap(),
            );
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "确认后将启动该软件在 Windows 中登记的卸载程序。Sunk 不会删除程序目录；后续步骤可能触发系统权限确认。",
                    )
                    .color(egui::Color32::from_rgb(225, 185, 105)),
                )
                .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let cancel = ui.button("取消");
                if state.focus_cancel {
                    cancel.request_focus();
                    state.focus_cancel = false;
                }
                if cancel.clicked() {
                    decision = Some(UninstallDecision::Cancel(prompt.request_id));
                }
                if ui.button("确认并启动卸载").clicked() {
                    decision = Some(UninstallDecision::Confirm(prompt.request_id));
                }
            });
        },
    );

    if decision.is_none() && response.should_close() {
        decision = Some(UninstallDecision::Cancel(prompt.request_id));
    }
    if let Some(decision) = decision {
        state.resolve_prompt(decision);
    }
}

fn display_tab(
    ui: &mut egui::Ui,
    settings: &mut BlackHoleSettings,
    desktop_capture: &DesktopCaptureState,
) {
    section_heading(ui, "吸积盘材质");
    let tint_description = "在物理黑体色温计算之后乘入的线性 RGB 色调，用于整体调色。";
    ui.horizontal(|ui| {
        ui.label("吸积盘色调");
        ui.color_edit_button_rgb(&mut settings.disk_tint_linear)
            .on_hover_text(tint_description);
    });
    setting_description(ui, tint_description);

    described_slider(
        ui,
        egui::Slider::new(&mut settings.disk_temperature_kelvin, 2_000.0..=25_000.0)
            .logarithmic(true)
            .suffix(" K")
            .text("内缘温度"),
        "决定吸积盘内缘的黑体辐射颜色；温度越高，光线越偏白蓝。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.disk_density, 0.25..=16.0).text("光学厚度"),
        "控制物质吸收与遮挡强度；数值越高，云层越厚实且不透明。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.disk_thickness, 0.25..=3.0).text("盘体厚度"),
        "控制吸积盘大气层的垂直厚度，不改变内外半径。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.emission_strength, 0.1..=10.0)
            .logarithmic(true)
            .text("辐射强度"),
        "调节物质在曝光和色调映射之前发出的辐射亮度。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.corona_opacity, 0.0..=1.5).text("云层体积"),
        "控制盘面上下稀薄高温云层的光学深度和可见程度。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.turbulence, 0.0..=1.0).text("湍流强度"),
        "增加多尺度密度扰动，使绕行光带产生自然的云状层次。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.cloudiness, 0.0..=1.0).text("云化程度"),
        "在薄盘光球与体积云之间混合；数值越高，结构越柔和、立体。",
    );

    section_heading(ui, "背景与光学");
    described_slider(
        ui,
        egui::Slider::new(&mut settings.background_warp, 0.0..=1.5).text("桌面扭曲强度"),
        "控制逃逸光线对桌面采样位置的位移倍率，模拟引力透镜弯曲。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.lens_radius, 0.4..=2.0).text("背景影响范围"),
        "缩放由黑洞表观半径推导出的透镜影响范围，会随黑洞大小同步变化。",
    );
    described_slider(
        ui,
        egui::Slider::new(&mut settings.exposure, 0.25..=4.0)
            .logarithmic(true)
            .text("画面曝光"),
        "统一调整最终辐射亮度；过高会损失吸积盘高光层次。",
    );

    if desktop_capture.is_ready() {
        ui.label(
            egui::RichText::new(format!(
                "桌面透镜已连接：{} × {}，捕获帧 {}",
                desktop_capture.frame_size.x,
                desktop_capture.frame_size.y,
                desktop_capture.frame_index
            ))
            .small()
            .color(egui::Color32::from_rgb(120, 205, 160)),
        );
    } else {
        ui.label(
            egui::RichText::new("桌面透镜正在等待安全捕获帧")
                .small()
                .color(egui::Color32::from_rgb(220, 180, 105)),
        );
    }
}

fn quality_tab(ui: &mut egui::Ui, settings: &mut BlackHoleSettings) {
    section_heading(ui, "光线积分");
    egui::ComboBox::from_label("渲染质量")
        .selected_text(settings.render_quality.label())
        .show_ui(ui, |ui| {
            for quality in RenderQuality::ALL {
                ui.selectable_value(&mut settings.render_quality, quality, quality.label());
            }
        });
    setting_description(
        ui,
        match settings.render_quality {
            RenderQuality::Performance => "使用较少积分步数，适合低功耗设备或较大的黑洞窗口。",
            RenderQuality::Balanced => "使用自适应 RK4 积分，在光线精度与实时性能之间平衡。",
            RenderQuality::Cinematic => "使用最高积分步数和更严格误差阈值，适合高端 GPU。",
        },
    );

    ui.label(format!(
        "每像素最多 {} 个自适应积分步 × {} 条空间光线",
        settings.render_quality.integration_steps(),
        settings.anti_aliasing.spatial_samples(),
    ));
    setting_description(
        ui,
        "渲染质量决定单条光线的积分精度；抗锯齿决定每像素的光线数量或后处理方式。",
    );

    section_heading(ui, "抗锯齿");
    egui::ComboBox::from_label("抗锯齿类型")
        .selected_text(settings.anti_aliasing.label())
        .show_ui(ui, |ui| {
            for mode in AntiAliasingMode::ALL {
                ui.selectable_value(&mut settings.anti_aliasing, mode, mode.label());
            }

            ui.separator();
            ui.add_enabled(false, egui::Button::selectable(false, "TAA（当前不适用）"))
                .on_disabled_hover_text(
                    "当前渲染没有可靠的运动矢量和逐像素深度，透明边缘还会产生历史残影。",
                );
            ui.add_enabled(false, egui::Button::selectable(false, "MSAA（当前不适用）"))
                .on_disabled_hover_text(
                    "MSAA 只能处理几何覆盖边缘，无法平滑全屏着色器内部计算出的黑洞轮廓。",
                );
        });
    setting_description(ui, settings.anti_aliasing.description());

    ui.add_space(6.0);
    ui.label(egui::RichText::new("方案说明").strong());
    setting_description(
        ui,
        "TAA 需要稳定的运动矢量、深度和历史重投影；本项目的透明全屏光线追踪不具备可靠输入，启用会造成拖影。",
    );
    setting_description(
        ui,
        "MSAA 只在三角形覆盖边界采样，而黑洞与吸积盘边缘生成于片元着色器内部，因此不会获得有效改善。",
    );
}

fn about_tab(ui: &mut egui::Ui) {
    section_heading(ui, "关于 Sunk");
    ui.label(format!("版本 {}", env!("CARGO_PKG_VERSION")));
    setting_description(
        ui,
        "使用原生 Rust、Bevy 0.19 和 WGSL 构建的透明桌面黑洞渲染器。",
    );

    section_heading(ui, "渲染技术");
    ui.label("Windows：DirectX 12 + DirectComposition");
    ui.label("光线：自适应测地线积分");
    ui.label("材质：黑体辐射 + 体积吸积盘");
    ui.label("背景：实时桌面捕获 + 引力透镜采样");

    section_heading(ui, "项目状态");
    setting_description(
        ui,
        "当前处于原型阶段。设置参数实时生效，后续可直接复用为正式设置窗口。",
    );
}

fn described_slider(ui: &mut egui::Ui, slider: egui::Slider<'_>, description: &'static str) {
    ui.add(slider).on_hover_text(description);
    setting_description(ui, description);
}

fn setting_description(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .small()
                .color(egui::Color32::from_gray(155)),
        )
        .wrap(),
    );
    ui.add_space(5.0);
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).strong());
    ui.add_space(2.0);
}

fn configure_chinese_font(context: &egui::Context, state: &mut ChineseFontState) {
    if state.attempted {
        return;
    }
    state.attempted = true;

    for path in chinese_font_candidates() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };

        let family_name = "sunk-cjk".to_owned();
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            family_name.clone(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(font_names) = fonts.families.get_mut(&family) {
                font_names.insert(0, family_name.clone());
            }
        }
        context.set_fonts(fonts);
        info!("loaded Chinese UI font from {}", path.display());
        return;
    }

    warn!("no CJK system font found; Chinese settings text may use fallback glyphs");
}

#[cfg(target_os = "windows")]
fn chinese_font_candidates() -> Vec<PathBuf> {
    let windows_dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let fonts = windows_dir.join("Fonts");
    ["msyh.ttc", "msyhbd.ttc", "simhei.ttf", "simsun.ttc"]
        .into_iter()
        .map(|name| fonts.join(name))
        .collect()
}

#[cfg(target_os = "macos")]
fn chinese_font_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
        PathBuf::from("/System/Library/Fonts/STHeiti Medium.ttc"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uninstall_prompt(request_id: u64) -> UninstallPrompt {
        UninstallPrompt {
            request_id,
            application_name: "Example".to_owned(),
            publisher: None,
            install_location: None,
            source_path: PathBuf::from(r"C:\Desktop\Example.lnk"),
        }
    }

    #[test]
    fn settings_window_uses_desired_size_when_it_fits() {
        assert_eq!(
            settings_window_logical_size(UVec2::new(1_920, 1_040), 1.0),
            UVec2::new(520, 720)
        );
    }

    #[test]
    fn settings_window_height_is_limited_at_high_dpi() {
        let size = settings_window_logical_size(UVec2::new(1_920, 1_040), 2.0);
        assert_eq!(size, UVec2::new(520, 472));
        assert!(size.y * 2 + 96 <= 1_040);
    }

    #[test]
    fn hiding_settings_cancels_the_prompt_and_restores_cancel_focus() {
        let mut window = Window {
            visible: true,
            focused: true,
            ..default()
        };
        let mut pending = PendingSettingsFocus(true);
        let mut state = FileActionUiState::default();
        state.request_uninstall(uninstall_prompt(42));
        state.focus_cancel = false;

        hide_settings_window(&mut window, &mut pending, &mut state);

        assert!(!window.visible);
        assert!(!window.focused);
        assert!(!pending.0);
        assert!(state.prompt.is_none());
        assert_eq!(state.take_decision(), Some(UninstallDecision::Cancel(42)));
        assert!(state.focus_cancel);
    }

    #[test]
    fn resolving_a_stale_prompt_does_not_replace_the_current_request() {
        let mut state = FileActionUiState::default();
        state.request_uninstall(uninstall_prompt(42));

        state.resolve_prompt(UninstallDecision::Confirm(41));

        assert_eq!(
            state.prompt.as_ref().map(|prompt| prompt.request_id),
            Some(42)
        );
        assert_eq!(state.take_decision(), None);
    }

    #[test]
    fn dismissing_a_resolved_prompt_clears_its_pending_decision() {
        let mut state = FileActionUiState::default();
        state.request_uninstall(uninstall_prompt(42));
        state.resolve_prompt(UninstallDecision::Confirm(42));

        state.dismiss_prompt(42);

        assert!(state.prompt.is_none());
        assert_eq!(state.take_decision(), None);
        assert!(state.focus_cancel);
    }
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn chinese_font_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/usr/share/fonts/opentype/source-han-sans/SourceHanSansSC-Regular.otf"),
    ]
}
