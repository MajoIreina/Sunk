# Current Architecture

## Product boundary

Sunk is a native Rust Windows desktop renderer. It presents a physically motivated black hole in a borderless, always-on-top transparent window, bends a captured copy of the desktop through the integrated light path, and exposes live controls in a separate Chinese settings window.

The current implementation is one executable package. The previous six-crate Phase 0 workspace is retained in Git history and in `backup/pre-rust-refactor-20260816`; it is not the runtime architecture of `main`.

```text
src/main.rs
  |-- black_hole.rs          Bevy material, render passes, and embedded WGSL
  |-- desktop_capture.rs     Windows desktop capture and GPU texture upload
  |-- physics.rs             CPU reference equations and invariants
  |-- settings.rs            Sanitized render and interaction settings
  |-- settings_ui.rs         Chinese egui settings window
  |-- system_tray.rs         Notification-area recovery and exit commands
  `-- window_interaction.rs  Window fitting, dragging, orbit, and hit testing

assets/shaders/black_hole.wgsl
  `-- Embedded into sunk.exe at compile time
```

## Graphics and transparency contract

The validated production path is Windows DX12 with wgpu's DirectComposition Visual presentation system. The primary surface uses premultiplied RGBA and clears to transparent. Escaped rays add no synthetic sky or star field; pixels are visible only when they contain emitted black-hole light, the opaque captured-ray shadow, or meaningfully displaced desktop content.

The application explicitly selects DX12 and Dynamic DXC. Startup requires a matching `dxcompiler.dll` and `dxil.dll` pair, discovered beside `sunk.exe`, through `SUNK_DXCOMPILER_PATH`, or in an installed Windows SDK. It does not silently fall back to FXC or another presentation backend.

The WGSL source is registered as a Bevy internal asset with `embedded_asset!`. A deployed executable therefore does not require the source `assets` directory. The source remains tracked for review, validation, and reproducible builds.

## Light integration and disk material

The fragment shader integrates Schwarzschild null paths in Schwarzschild-radius units. Quality presets use a midpoint performance path or adaptive RK4 with progressively larger integration budgets. Segment-to-horizon tests prevent a large adaptive step from tunneling through the captured region. CPU f64 tests independently check characteristic radii, the critical impact boundary, and invariant drift.

The accretion disk combines a porous ray-plane photosphere with a layered cloud volume. Domain-warped noise is advected with Keplerian `r^-3/2` differential rotation. Beer-Lambert front-to-back transfer, a zero-torque temperature profile, Doppler beaming, and gravitational frequency shift produce the visible material and color response.

SMAA operates as a Bevy post-process on the transparent primary camera. SSAA 2x2 renders four deterministic subpixel geodesic passes with additive premultiplied blending. TAA and MSAA remain visible but unavailable because the ray marcher does not provide reliable motion/depth history and its important edges are generated inside the fragment shader rather than at triangle coverage boundaries.

## Desktop lensing

The Windows backend captures an overscanned physical region of the monitor through GDI and uploads it as an sRGB texture. The renderer HWND is temporarily excluded from capture to prevent recursive feedback. Failed overscan falls back to the client region and is retried later.

An escaped integrated ray and its straight reference ray intersect the same finite background plane. Their displacement is combined with conserved impact parameter, closest approach, and path-length response. A physical far-field falloff returns the window edge to transparency without replacing geodesic deformation with a fixed circular UV mask.

The capture implementation is intentionally isolated behind `DesktopCaptureState`. A future Windows Graphics Capture and shared-texture backend can replace the GDI readback without changing the shader or settings contract.

## Windows, input, and settings

The primary window tracks apparent black-hole size and lens influence while keeping the outer visible edge inside an 88% safe radius of the current monitor work area. A resize remains click-through until native client dimensions confirm the requested geometry.

Dynamic hit testing claims pointer input only over the central shadow/photon ring and projected emitting disk. Transparent corners and desktop-only lens pixels pass clicks to underlying applications. A drag remains owned only if its press began on interactive primary-window content.

The independent egui window contains General, Display, Quality, and About pages in Chinese. Closing or minimizing it hides it to the Windows notification area. The tray icon restores settings or exits the application.

## Deployment boundary

A runnable Windows bundle currently consists of `sunk.exe`, a matching DXC runtime pair, and the applicable Microsoft Visual C++ runtime. Packaging must record the DXC source, version, hashes, and redistribution terms. Signing, installer generation, and update delivery remain release work.
