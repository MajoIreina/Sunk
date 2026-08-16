#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod black_hole;
mod desktop_capture;
pub mod physics;
mod settings;
mod settings_ui;
mod window_interaction;

use bevy::{
    prelude::*,
    render::{
        RenderPlugin,
        settings::{Backends, WgpuSettings},
    },
    window::{CompositeAlphaMode, MonitorSelection, WindowLevel, WindowPosition, WindowResolution},
    winit::WinitSettings,
};
use bevy_egui::EguiPlugin;

use black_hole::BlackHolePlugin;
use desktop_capture::DesktopCapturePlugin;
use settings_ui::SettingsUiPlugin;
use window_interaction::WindowInteractionPlugin;

fn main() {
    configure_windows_transparency();

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(RenderPlugin {
                    render_creation: platform_wgpu_settings().into(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Sunk Black Hole".into(),
                        name: Some("sunk-black-hole".into()),
                        resolution: WindowResolution::new(900, 700),
                        position: WindowPosition::Centered(MonitorSelection::Primary),
                        transparent: true,
                        decorations: false,
                        // The client surface follows the requested black-hole size.
                        // User-driven edge resizing would break that composition.
                        resizable: false,
                        window_level: WindowLevel::AlwaysOnTop,
                        #[cfg(target_os = "windows")]
                        composite_alpha_mode: CompositeAlphaMode::PreMultiplied,
                        // Keep the DComp Visual HWND in the normal taskbar class.
                        // On the validated Windows/DX12 path, winit's tool-window
                        // style (`skip_taskbar: true`) makes this transparent
                        // surface present black. Settings still minimize to the
                        // independent notification-area icon.
                        skip_taskbar: false,
                        ..default()
                    }),
                    close_when_requested: false,
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::NONE))
        // A desktop object must keep animating while another application has focus.
        .insert_resource(WinitSettings::continuous())
        .add_plugins(EguiPlugin::default())
        .add_plugins((
            SettingsUiPlugin,
            DesktopCapturePlugin,
            BlackHolePlugin,
            WindowInteractionPlugin,
        ))
        .run();
}

#[cfg(target_os = "windows")]
fn configure_windows_transparency() {
    // SAFETY: This is the first operation in `main`, before Bevy starts its task
    // pools or any other thread. No concurrent environment access can occur.
    unsafe {
        std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "Visual");
    }
}

#[cfg(not(target_os = "windows"))]
fn configure_windows_transparency() {}

fn platform_wgpu_settings() -> WgpuSettings {
    #[cfg(target_os = "windows")]
    {
        WgpuSettings {
            // Do not rely on wgpu's backend preference order. The transparent
            // surface contract is validated specifically on DX12 + DComp Visual.
            backends: Some(Backends::DX12),
            // FXC takes minutes to optimize this large geodesic ray marcher.
            // Require modern DXC explicitly instead of silently falling back.
            dx12_shader_compiler: bevy::render::settings::Dx12Compiler::DynamicDxc {
                dxc_path: windows_dxcompiler_path(),
            },
            ..default()
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        WgpuSettings::default()
    }
}

#[cfg(target_os = "windows")]
fn windows_dxcompiler_path() -> String {
    use std::path::{Path, PathBuf};

    fn valid_runtime(compiler: &Path) -> bool {
        compiler.is_file() && compiler.with_file_name("dxil.dll").is_file()
    }

    if let Some(path) = std::env::var_os("SUNK_DXCOMPILER_PATH").map(PathBuf::from)
        && valid_runtime(&path)
    {
        return path.to_string_lossy().into_owned();
    }

    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let adjacent = directory.join("dxcompiler.dll");
        if valid_runtime(&adjacent) {
            return adjacent.to_string_lossy().into_owned();
        }
    }

    let program_files_x86 = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let sdk_bin = program_files_x86.join(r"Windows Kits\10\bin");
    let mut sdk_versions = std::fs::read_dir(&sdk_bin)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    sdk_versions.sort_unstable();

    for version in sdk_versions.into_iter().rev() {
        let compiler = version.join(r"x64\dxcompiler.dll");
        if valid_runtime(&compiler) {
            return compiler.to_string_lossy().into_owned();
        }
    }

    panic!(
        "DXC 1.8.2502 or newer is required: place dxcompiler.dll and dxil.dll next to the executable, or set SUNK_DXCOMPILER_PATH"
    );
}
