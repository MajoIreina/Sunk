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

- [x] Accept Explorer and desktop `CF_HDROP` messages on the primary black-hole window (implemented this iteration; manual coverage pending)
- [x] Re-check the shaped drop target on release and isolate primary-window drops from the settings HWND
- [x] Preserve first-seen multi-file order, remove exact duplicates, and reject batches above 256 unique targets
- [x] Add hover, radial infall with near-horizon slowdown, progressive shadow occlusion, success, and failure visuals
- [x] Move validated ordinary files and directories only to the Windows Recycle Bin on a worker thread
- [x] Reject protected paths, drive roots, reparse points, symbolic links, UNC/network paths, application launchers, and the running Sunk path from ordinary disposal
- [x] Resolve classic `.lnk` and `.exe` application identities against unique high-confidence Win32/MSI uninstall records
- [x] Require a Chinese uninstall confirmation and launch a validated executable/MSI plan without a command shell
- [x] Reject UWP/MSIX/AppRef, `.url`/`.website`, unsafe, missing, and ambiguous uninstall targets
- [x] Record winit's fixed `DROPEFFECT_COPY` cursor as a known native UX limitation
- [ ] Complete manual coverage for 100%, 150%, and 200% DPI across mixed-monitor layouts
- [ ] Complete repeated tray restore, explorer/dialog click-through, sleep/wake, and display-change tests
- [ ] Persist settings with schema migration and corruption recovery
- [ ] Add an explicit opt-in global shortcut for settings recovery
- [ ] Replace the prototype capture rate policy with measured bandwidth and latency targets

## File-operation manual validation

These checks are intentionally open. Checked implementation items above do not claim that live shell behavior has been verified.

- [ ] Drag one disposable file from Explorer into the black-hole target, verify success feedback, confirm it appears in the Recycle Bin, and restore it
- [ ] Repeat with a disposable directory and confirm its contents remain recoverable
- [ ] Drop multiple disposable paths and verify first-seen ordering, de-duplication, per-item animation, and per-item results
- [ ] Enter the black-hole target, move into a transparent part of the same HWND, then release; verify no operation occurs and the background target remains usable
- [ ] Drop directly on a transparent corner and verify the underlying Explorer window or dialog receives the action
- [ ] Verify protected system/program paths, the Sunk executable tree, reparse points, symbolic links, UNC paths, and network shares are rejected without mutation
- [ ] Drag a known classic Win32 `.lnk` and `.exe`, verify the Chinese application/publisher/path prompt, then cancel and confirm no process starts
- [ ] Use a disposable Win32 test application to confirm the registered uninstaller starts only after explicit confirmation
- [ ] Use a disposable MSI package to confirm product-code matching and explicit uninstall launch behavior
- [ ] Verify UWP/MSIX/AppRef links, browser web-app shortcuts, unmatched executables, and ambiguous registry matches are rejected rather than guessed
- [ ] Force analysis, Recycle Bin, and launch failures and verify the event-horizon visual resolves to failure instead of a false success
- [ ] Confirm rendering and desktop click-through remain responsive while the worker performs file analysis or a Recycle Bin operation
- [ ] Record the Explorer copy cursor on accepted drags and decide whether a custom Windows `IDropTarget` is required before release
- [ ] Repeat accepted, rejected, and canceled drops at 100%, 150%, and 200% DPI on mixed-monitor layouts

## Release engineering

- [ ] Pin the distributable DXC runtime source, version, SHA-256 values, and redistribution notice
- [ ] Define whether the Visual C++ runtime is bundled or installed as a prerequisite
- [ ] Produce a signed Windows bundle and installer
- [ ] Add crash reporting and update behavior only after privacy and consent requirements are approved
- [ ] Establish reference GPU, resolution, frame-rate, memory, and bundle-size acceptance gates

`Sunk_Desktop_Development_Document_v0.1.docx` remains the historical product baseline. This roadmap and the current source describe the implemented renderer after the native Rust replacement.

Rollback point: `backup/pre-file-operations-20260816` -> `8864a41`.
