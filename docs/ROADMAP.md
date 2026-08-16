# Sunk Delivery Roadmap

## Native renderer prototype (complete)

- [x] Rust 2024 single-package application with locked dependencies
- [x] Windows DX12 and DirectComposition transparent presentation
- [x] WGSL embedded into the executable and explicit Dynamic DXC selection
- [x] Adaptive Schwarzschild null-ray integration with CPU reference tests
- [x] Soft, layered, differentially rotating accretion-disk clouds
- [x] Doppler, gravitational shift, Beer-Lambert transfer, and photon-ring response
- [x] Live desktop capture with geodesic displacement and recursive-capture prevention
- [x] Window size following apparent black-hole size with monitor work-area limits
- [x] Content-shaped hit testing and transparent click-through
- [x] Chinese General, Display, Quality, and About settings pages
- [x] Notification-area restore and exit controls
- [x] SMAA and four-pass SSAA 2x2 paths with explicit TAA/MSAA status
- [x] Windows CI for formatting, strict Clippy, tests, locked Release build, and size gate

## Rendering and capture hardening

- [ ] Replace GDI readback with Windows Graphics Capture and a proven shared-texture bridge
- [ ] Add GPU timestamp queries and sustained adaptive-quality decisions
- [ ] Add a Kerr spacetime option with validated spin and frame-dragging behavior
- [ ] Investigate temporal reconstruction only after motion, depth, alpha-history, and desktop-latency inputs are reliable
- [ ] Build automated transparent-surface and nonblank canvas smoke checks for CI-capable hardware

## Desktop integration

- [ ] Complete manual coverage for 100%, 150%, and 200% DPI across mixed-monitor layouts
- [ ] Complete repeated tray restore, explorer/dialog click-through, sleep/wake, and display-change tests
- [ ] Persist settings with schema migration and corruption recovery
- [ ] Add an explicit opt-in global shortcut for settings recovery
- [ ] Replace the prototype capture rate policy with measured bandwidth and latency targets

## Release engineering

- [ ] Pin the distributable DXC runtime source, version, SHA-256 values, and redistribution notice
- [ ] Define whether the Visual C++ runtime is bundled or installed as a prerequisite
- [ ] Produce a signed Windows bundle and installer
- [ ] Add crash reporting and update behavior only after privacy and consent requirements are approved
- [ ] Establish reference GPU, resolution, frame-rate, memory, and bundle-size acceptance gates

`Sunk_Desktop_Development_Document_v0.1.docx` remains the historical product baseline. This roadmap and the current source describe the implemented renderer after the native Rust replacement.
