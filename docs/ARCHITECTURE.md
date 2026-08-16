# Current Architecture

## Product boundary

Sunk is a native Rust Windows desktop renderer. It presents a physically motivated black hole in a borderless, always-on-top transparent window, bends a captured copy of the desktop through the integrated light path, and exposes live controls in a separate Chinese settings window.

The current implementation is one executable package. The previous six-crate Phase 0 workspace is retained in Git history and in `backup/pre-rust-refactor-20260816`; it is not the runtime architecture of `main`.

```text
src/main.rs
  |-- black_hole.rs          Bevy material, render passes, and embedded WGSL
  |-- desktop_capture.rs     Windows desktop capture and GPU texture upload
  |-- file_interaction.rs    CF_HDROP intake and file-object visual lifecycle
  |-- file_operations.rs     Validation, Recycle Bin, and uninstall boundary
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

The Windows backend captures an overscanned physical region of the monitor through GDI at 30 FPS and uploads it as an sRGB texture. Every render frame remaps the last capture's absolute desktop rectangle against the overlay's current client rectangle, avoiding a one-capture-period offset while the window moves. The renderer HWND is excluded from capture to prevent recursive feedback. Failed overscan falls back to the client region and is retried later.

An escaped integrated ray and its straight reference ray intersect the same finite background plane. Their displacement is combined with conserved impact parameter, closest approach, and path-length response. A physical far-field falloff returns the window edge to transparency without replacing geodesic deformation with a fixed circular UV mask.

The capture implementation is intentionally isolated behind `DesktopCaptureState`. A future Windows Graphics Capture and shared-texture backend can replace the GDI readback without changing the shader or settings contract.

## Windows, input, and settings

The primary window tracks apparent black-hole size and lens influence while keeping the outer visible edge inside an 88% safe radius of the current monitor work area. A resize remains click-through until native client dimensions confirm the requested geometry.

Dynamic hit testing claims pointer input only over the central shadow/photon ring and projected emitting disk. Transparent corners and desktop-only lens pixels pass clicks to underlying applications. A drag remains owned only if its press began on interactive primary-window content.

Windows Explorer and desktop file drags arrive through winit's OLE `CF_HDROP` target as Bevy `FileDragAndDrop` messages. Only messages for the primary window are considered. Global left-button state and the physical lens extent let the click-through overlay acquire an incoming OLE drag before its first hover message; this path does not grant ordinary pointer ownership. During the drag, the native cursor drives a radial tidal echo through the same geodesic, capture mapping, and edge support used by desktop lensing. Because the drop message does not contain an authoritative black-hole-local release point, the current cursor is tested against the narrower rendered target again when `DroppedFile` is received. The external drag lock also defers native resizing, while a release outside the shadow or emitting disk is rejected. The settings HWND is never a file-operation source.

winit reports `DROPEFFECT_COPY` for its built-in Windows drop target. Explorer therefore displays a copy cursor even though Sunk performs a separately authorized Recycle Bin or uninstall action after the drop. A truthful native cursor would require replacing that layer with a custom `IDropTarget`; this is an explicit interaction limitation, not a change to the operation policy.

The independent egui window contains General, Display, Quality, and About pages in Chinese. Quality includes a 30/60/120 FPS frame pacer while both DX12 surfaces retain VSync presentation. A native window subclass consumes minimize before winit can reconfigure the settings surface to zero size, hides the HWND, and mirrors that state back into Bevy. This guard is created even if notification-area initialization fails. Closing follows the same hide behavior. When available, the tray icon restores settings or exits the application without ending the renderer process.

## File interaction lifecycle

`file_interaction.rs` owns no filesystem or process APIs. It coalesces all paths delivered for one update, preserves first-seen order, removes exact duplicates, and rejects more than 256 unique targets. Hover feedback is driven by the live cursor rather than by a one-time OLE enter event.

Each accepted path receives a stable visual identifier. Ordinary files can begin immediately after validation; application visuals are staged and pause before infall until confirmation. The visual phases are radial attraction, capture, infall with near-horizon slowdown, apparent-horizon entry, and explicit success or failure. Position is constrained to a zero-angular-momentum radial path: no orbit, spiral, or rigid-body spin is added. A monotonic tidal proxy stretches the radial axis and compresses the tangential axis. The apparent horizon is derived from the renderer's critical-impact shadow radius rather than a fixed pixel ratio; the wider pointer hit tolerance is kept separate. An external visual redshifts, emits an operation-ready message at that boundary, and waits before success lets it cross. The icon body is split into `4x4` tiles whose visibility follows the same shadow geometry, producing progressive occlusion after entry. A release already overlapping the shadow starts at its exact initial position and moves only inward, avoiding both foreground bleed and an artificial outward projection. Cancellation, validation errors, Recycle Bin failures, and launch failures take the rejection path.

The hand-off is intentionally narrow:

```text
FileDragAndDrop
  -> ordered DropBatchRequested
  -> worker analysis and stable operation intent
  -> VisualCommand::Begin or VisualCommand::Stage
  -> VisualOperationReady
  -> MoveToTrash or LaunchUninstall worker command
  -> VisualCommand::Complete(success/failure) and Chinese status
```

Paths remain in the operation coordinator. The renderer and visual entities receive only identifiers, a file/application kind, a start position, and lifecycle commands.

## Recycle Bin boundary

Blocking inspection, COM shell work, and process launch run on a named file-operation worker rather than the Bevy render thread. A normal file or directory is canonicalized and validated immediately before a native `IFileOperation` request with `FOFX_RECYCLEONDELETE` and early-failure flags. Aborted or unsupported recycle operations fail visibly; there is no call to `remove_file`, `remove_dir_all`, or a permanent-delete fallback.

The validator rejects empty or missing targets, drive roots, the Windows directory, Program Files, Program Files (x86), ProgramData, a directory containing any of those protected trees, the running Sunk executable or a directory containing it, reparse points and symbolic links, UNC paths, and other paths that are not local drive-letter paths. `.lnk`, `.exe`, `.appref-ms`, `.url`, and `.website` inputs are never treated as ordinary trashable files.

Validation makes each requested operation auditable but does not turn a multi-file batch into a filesystem transaction. Results are tracked per visual identifier so a partial batch can show individual success and failure accurately.

## Application uninstall boundary

The uninstall backend supports classic Win32 and MSI identities from `.lnk` or `.exe` inputs. A shortcut is resolved with the Windows Shell Link COM interfaces. Candidates are enumerated from current-user and local-machine uninstall records in both 32-bit and 64-bit registry views. System components, update/patch records, entries marked as non-removable, entries without a display name, and entries without an uninstall command are excluded.

MSI product identity is definitive. Other candidates are ranked using exact display icon, install location, display name, and executable-name evidence; one exact executable-stem match is sufficient to present a user confirmation. Ties use stable registry-hive, registry-view, key-name, and uninstall-command ordering instead of blocking the requested uninstall. Shortcut arguments do not veto an otherwise eligible record: a browser-hosted shortcut may therefore present its registered host application, but the dialog names that application and its exact uninstall source before the user decides. A missing eligible candidate is rejected.

Before authorization, the Chinese egui modal shows the application name, unverified registry publisher, install location, registered uninstall source, and dropped source. Cancel is the initially focused action. Confirmation retains that exact launch plan; when the visual reaches the operation boundary it is submitted directly rather than being blocked by a second registry enumeration. MSI uses the system `msiexec.exe` with a validated product GUID; traditional uninstall strings are parsed with Windows command-line rules into an absolute local executable plus arguments. Legacy unquoted paths containing spaces are recovered only through an explicit `.exe` boundary, after which Windows parses the remaining argument tail. The executable no longer has to live inside the application's install directory. Shell and script hosts, network or relative paths, missing executables, and reparse-point uninstallers remain rejected. No command is passed through a shell, and Sunk never deletes an application directory itself.

An `UninstallStarted` result means only that Windows accepted creation of the registered uninstaller process. It must not be presented as proof that the product was fully removed. UWP, MSIX, AppRef, Store applications, and other ambiguous shell identities remain unsupported in this backend.

The rollback branch `backup/pre-file-operations-20260816` fixes the pre-feature boundary at commit `8864a41`. Implementation claims in this document do not replace the outstanding manual Explorer and Windows shell checks recorded in the roadmap.

## Deployment boundary

The v0.0.1 Windows x64 portable bundle consists of `sunk.exe`, the official Microsoft DirectXShaderCompiler v1.9.2607 x64 runtime pair, project, DXC, and resolved Rust dependency license files, release notes, and SHA-256 manifests. WGSL remains embedded and no source `assets` directory is deployed. The Microsoft Visual C++ 2015-2022 Redistributable (x64) is a system prerequisite rather than a bundled component.

`THIRD_PARTY_NOTICES.md` records the pinned DXC source archive and binary hashes. The packaging script rejects a version mismatch, missing runtime pair, missing notices, an oversized executable, or a non-empty destination. Code signing, installer generation, and update delivery remain future release work.
