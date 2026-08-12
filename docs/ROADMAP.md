# Sunk Delivery Roadmap

## Phase 0: technical foundation (current)

- [x] Rust workspace and stable dependency baseline
- [x] Transparent, borderless, always-on-top window configuration
- [x] DirectComposition-backed DX12 surface on Windows
- [x] Procedural lensed-disk WGSL black-hole prototype
- [x] Core interaction state machine and adaptive quality policy
- [x] Safe settings and filesystem boundaries
- [x] Cross-platform CI and binary-size check
- [x] Windows DX12 transparency and visual smoke test
- [ ] Real macOS Metal transparency smoke test
- [ ] Documented macOS visual smoke-test results

### Windows smoke result (2026-08-13)

- Windows 11 build 26220, NVIDIA GeForce RTX 5080, 640 x 640 physical render size
- Procedural black hole is nonblank and remains responsive after hide/restore
- Window corners match the unobscured desktop exactly; no opaque rectangular background
- Captured-ray shadow is opaque black while all pixels outside emitted light remain transparent
- Lensed rear disk appears above and below the shadow; left/right luminance ratio is about 2.24:1
- Idle sample: about 0.13% GPU 3D engine usage and 0.05% total CPU usage
- Release executable: 2.57 MiB, below the 50 MiB binary gate

## Phase 1: interactive renderer

- Refine the cinematic Kerr-spin approximation and camera controls
- [x] Drag-session aggregation for multiple paths
- [x] Deterministic path-free capture/orbit/event-horizon timeline
- [ ] Per-target file-object capture/orbit/event-horizon rendering
- Off-screen HDR target, bloom, tonemapping, and final composite
- GPU timestamp queries with CPU fallback and explicit metric labels
- Resolution scaling and ray-step changes wired to the renderer

## Phase 2: desktop integration

- Explicit `Interactive` and `PassThrough` window modes
- Tray/menu recovery path for `Interactive` mode
- Explorer/Finder drag-and-drop acceptance tests (Windows implementation smoke-tested; macOS pending)
- DPI, multiple-monitor, sleep/wake, and display-change handling

## Phase 3: safe file operations

- Operation coordinator outside the render loop
- Move-to-trash preview, validation, cancellation, and completion events
- Symlink/alias/shortcut behavior specification
- No permanent deletion until confirmation UX and recovery policy are approved

## Release definition gaps

Before enforcing product acceptance, define:

- the reference GPU and physical render resolution for the 60 FPS target;
- whether the 50 MB limit applies to the executable, signed app bundle, installer, or archive;
- minimum supported Windows and macOS versions;
- signing, notarization, update, and crash-report privacy requirements.
