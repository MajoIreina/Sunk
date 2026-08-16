# Sunk Black Hole

Native Rust prototype for a physically motivated black-hole desktop renderer. It draws a Schwarzschild black hole and accretion disk into a borderless, always-on-top, transparent window. There is deliberately no skybox or star background: escaped rays are transparent unless they are used to lens captured desktop pixels.

## Run

Requirements: Windows 10 version 2004 / build 19041 or newer, a current GPU driver, Rust 1.95 or newer (the repository pins Rust 1.97.1), and the DXC 1.8.2502 runtime or newer.

```powershell
cargo run --release
```

The black-hole WGSL source is embedded in the executable at compile time; a deployed Release binary does not require an adjacent `assets` directory. On Windows the renderer explicitly restricts wgpu to DX12, requires modern DXC, and selects the DirectComposition Visual presentation path. It fails during renderer initialization instead of silently falling back to a backend with an unverified transparency contract.

For a deployed build, place `dxcompiler.dll` and its matching `dxil.dll` next to `sunk.exe`. Development runs also search installed Windows 10 SDK `x64` directories. `SUNK_DXCOMPILER_PATH` can point directly to another `dxcompiler.dll`, but `dxil.dll` must be beside it. Startup stops with an explicit error if a complete runtime pair cannot be found.

The separate settings window opens with the renderer and changes the result live. `F1` hides or shows it. Minimizing or closing that window sends it to the Windows notification area; left-click the Sunk tray icon or choose **显示设置** to restore it, and choose **退出 Sunk** to exit. The tray tooltip is also localized. The transparent primary HWND intentionally remains in the normal taskbar class because winit's tool-window style produces a black DirectComposition surface on the validated DX12 path.

Primary-window controls:

- Left drag on the visible black hole or disk: move the borderless renderer
- Right drag on the visible black hole or disk: orbit the observer
- Mouse wheel over the visible black hole or disk: change black-hole apparent size
- `Space`: pause disk animation
- `R`: reset the observer and all visual settings
- `P`: force full-window mouse passthrough on or off
- `F1`: hide or show settings
- `Esc`: quit from the renderer, or hide the focused settings window

## Settings window

The settings interface is Chinese and gives every editable option a short inline description as well as a hover explanation where useful. Its initial size is capped against the primary monitor work area and DPI; the scroll view keeps every control reachable on compact or high-scale displays. It is organized into four pages:

| Page | Controls and status |
| --- | --- |
| **通用** | Black-hole apparent size, disk animation speed, and interaction summary |
| **显示** | Disk tint, physical temperature, optical depth, thickness, emission, corona, turbulence, cloud layering, desktop-warp strength, lens influence, exposure, and capture status |
| **画质** | Geodesic integrator quality and an independent anti-aliasing selector |
| **关于** | Version, renderer, material, and desktop-compositing information |

Changes apply live. **恢复默认设置** restores the complete render configuration. Persistence and a system-wide hotkey remain later settings-product work.

## Window fitting and input

The transparent renderer window grows and shrinks with the apparent-size control. Its camera composition expands with the disk and lens-influence setting so the outer visible edge stays inside an 88% safe radius. Native resizing preserves the window center where possible and clamps the outer rectangle to the current monitor work area; if the requested maximum cannot fit, the effective black-hole size is reduced instead of clipping the render at the window or screen edge. The requested value is retained, so moving back to a larger monitor restores the intended size automatically.

Hit testing follows the global Windows cursor even after the overlay becomes click-through. Only the central shadow/photon ring and the projected emitting-disk ellipse claim pointer input. Transparent corners and areas that contain only refracted desktop pixels pass clicks to dialogs and applications underneath. Hit testing stays locked only when a press originated on visible primary-window content, so dragging a settings control cannot make the transparent renderer claim the desktop. A queued resize remains click-through until the native client size is confirmed; `P` remains an explicit full-window passthrough override.

## Rendering model

Spatial distances use Schwarzschild-radius units (`r_s = 1`). The shader integrates the null-orbit equation

```text
p'' = -(3/2) |p x p'|^2 p / |p|^5
```

with a curvature-aware adaptive step. Performance uses 128 midpoint steps, Balanced uses up to 256 RK4 steps, and Cinematic uses up to 384 RK4 steps. Integrator quality is independent of the selected anti-aliasing method. Segment/sphere tests prevent a large step from tunneling through the event horizon. A separate f64 CPU oracle checks the capture boundary and conservation drift.

The disk combines a porous photosphere at an exact ray/plane intersection with a vertically layered cloud volume. Domain-warped multi-scale noise forms clumps, lanes, and filaments; Keplerian `r^-3/2` differential rotation and height-dependent phase shear make those structures flow instead of rotating as a flat texture. Radiance uses front-to-back Beer-Lambert transfer,

```text
a = 1 - exp(-tau)
L = L + T * a * source
T = T * (1 - a)
```

so density is not accidentally applied twice. The radial temperature follows a zero-torque thin-disk profile. A Planckian-locus approximation supplies black-body chromaticity; Schwarzschild gravitational shift and circular-orbit Doppler shift alter observed temperature and brightness. The user tint is applied afterward in linear RGB.

Captured rays keep foreground disk radiance and make the untransmitted event horizon opaque black. Escaped rays add no synthetic background. Output is premultiplied RGBA exactly once and is presented through wgpu's D3D12 DirectComposition path.

## Anti-aliasing

The **画质** page exposes only paths that currently preserve the transparent premultiplied-RGBA contract:

- **关闭** uses one geodesic sample with no edge post-process.
- **SMAA（推荐）** applies Bevy's high-preset morphological edge pass to the primary transparent camera.
- **SSAA 2x2（高开销）** traces four deterministic subpixel geodesics as separate additive draw passes. Each pass contributes one quarter of its premultiplied color and optical coverage, avoiding the large single-fragment shader loop that previously caused slow DX12 pipeline compilation.

TAA and MSAA remain visible but disabled in the selector so their status is explicit. TAA needs stable motion vectors, depth, and history reprojection; the transparent full-screen ray marcher does not yet provide reliable inputs, and separately accumulated color/opacity would ghost desktop edges. MSAA samples triangle coverage, while the black-hole, photon-ring, disk, and lensed-desktop edges are calculated inside a full-screen fragment shader, so it does not meaningfully smooth them.

## Desktop lensing

The Windows proof backend captures a physical client-area region with up to 45% overscan on every side and uploads an sRGB desktop texture at 20 FPS. GDI capture is constrained to the monitor containing the renderer because a single `BitBlt` rectangle spanning multiple displays or adapters is not reliable; uncovered areas remain transparent. If the expanded copy fails, it temporarily falls back to the client region and periodically retries overscan.

Every escaped ray intersects a finite virtual background plane. The shader compares the integrated bent-ray projection with the corresponding straight-ray projection, then combines the measured pixel displacement with impact parameter, closest approach, and excess path length. The requested view-plane influence extent is converted back into the ray's conserved physical impact parameter; a broad terminal falloff and a perceptual subpixel threshold return the window edges to transparency without replacing the geodesic deformation with a fixed circular UV mask. Weak far-field bending fades naturally, while strong and asymmetric photon-ring arcs retain their shape. Only pixels with visible displacement replace the live transparent desktop, which prevents a delayed unwarped copy from covering the real desktop. The apparent-size and lens-influence controls update both the ray composition and native window fit, so the desktop effect stays aligned with the black hole.

This establishes that the rendered black hole can visibly bend real desktop content, including multiple images close to the photon ring. It is still a prototype capture path: GDI readback and CPU upload introduce latency and bandwidth cost. The next performance gate is Windows Graphics Capture plus a proven D3D11-to-D3D12/wgpu shared-texture bridge behind the existing `DesktopCaptureState` interface.

The transparent renderer HWND uses `WDA_EXCLUDEFROMCAPTURE` to prevent recursive feedback. The opaque settings window remains ordinary capturable desktop content; it never samples the capture texture itself and therefore cannot recurse. Consequently, normal third-party screen recorders omit the black-hole overlay. Recording the complete effect will require an internal compositor/encoder path.

## Scope and validation

- Current spacetime: Schwarzschild, not Kerr. Rotation controls disk pattern motion; it does not claim frame dragging.
- No synthetic star field or opaque rectangular background.
- TAA and MSAA are deliberately unavailable for the technical reasons described above; SMAA is the default and SSAA 2x2 is the high-cost spatial option.
- Run `cargo test` for characteristic radii, critical capture/escape rays, invariant drift, capture-coordinate mapping, and settings sanitization.
- Set `SUNK_CAPTURE_FRAME` to an absolute PNG path before launch for an internal framebuffer QA capture. `SUNK_CAPTURE_AFTER_DESKTOP_FRAME` optionally delays it until a chosen desktop-capture frame.

This is an independent implementation of published physical equations. See `THIRD_PARTY_NOTICES.md` for research references and licensing notes.

## Project documents

- `docs/ARCHITECTURE.md` describes the current native renderer and desktop-compositing boundaries.
- `docs/ROADMAP.md` records the remaining renderer, integration, and release work.
- `Sunk_Desktop_Development_Document_v0.1.docx` is retained as the historical product baseline; where it differs from the running implementation, the current source and Markdown documentation take precedence.

The CI release gate limits `sunk.exe` to 128 MiB. The separately deployed DXC runtime pair is tracked as packaging work rather than being hidden inside that executable limit.

## License

Sunk is distributed under GPL-3.0-only. See `LICENSE`.
