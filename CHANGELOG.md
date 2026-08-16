# Changelog

All notable public changes to Sunk are recorded in this file.

## [0.0.1] - 2026-08-17

First public Beta release.

### Added

- Native Rust DX12 and DirectComposition transparent desktop renderer.
- Embedded WGSL Schwarzschild ray tracing with a layered, differentially rotating accretion disk.
- Live desktop gravitational lensing with adjustable influence radius and softened compositing boundary.
- Chinese General, Display, Quality, and About settings pages.
- 30, 60, and 120 FPS limits plus SMAA and SSAA 2x2 anti-aliasing paths.
- Notification-area restore and exit controls.
- Explorer file and directory drops that move validated targets to the Recycle Bin.
- Classic Win32 and MSI uninstall discovery with an explicit Chinese confirmation dialog.
- Radial attraction, tidal deformation, horizon waiting, and progressive horizon occlusion visuals.

### Known limitations

- Windows 10 version 2004 or newer, x64, DX12, and the Microsoft Visual C++ 2015-2022 Redistributable (x64) are required.
- The portable build is unsigned and has no installer or automatic updater.
- Desktop capture currently uses a 30 FPS GDI readback path and can show capture latency.
- Settings do not persist between launches.
- Explorer shows its standard copy cursor during accepted drags.
- Application uninstall supports classic Win32 and MSI records; UWP, MSIX, AppRef, and web shortcuts are unsupported.
- Manual mixed-DPI, Recycle Bin, disposable-uninstaller, sleep/wake, and display-change coverage remains open in the roadmap.
