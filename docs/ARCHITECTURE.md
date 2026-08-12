# Architecture Baseline

## Phase 0 vertical slice

The initial workspace intentionally implements six cohesive modules instead of creating every
future crate as an empty placeholder. New modules should be extracted only when they own real
behavior and tests.

```text
sunk-app
  |-- sunk-core
  |-- sunk-desktop
  |-- sunk-renderer
  |-- sunk-settings
  `-- sunk-filesystem (not invoked by the prototype)
```

`sunk-core` has no dependency on `winit`, `wgpu`, or platform APIs. The renderer accepts only
renderer-facing values such as `RenderQuality`. The filesystem crate owns OS trash integration and
is not reachable from the renderer.

## Transparency contract

Windows uses the DX12 DirectComposition presentation path (`DxgiFromVisual`) together with a
no-redirection-bitmap window and prefers `PreMultiplied` alpha, which DirectComposition accepts
reliably. macOS uses Metal and prefers `PostMultiplied` alpha. An opaque-only surface is treated as
an initialization error rather than silently showing a black rectangle.

The final pass writes RGB in the alpha representation selected for the platform and clears to fully
transparent. Future bloom and ray passes must render to off-screen textures, followed by a final
compositing render pass. Compute shaders must not write directly to the swapchain.

## Gargantua visual contract

The Phase 0 shader now traces an inexpensive Schwarzschild-style light path through a tilted,
procedural accretion disk. Multiple disk-plane crossings produce the recognizable upper and lower
lensed images instead of a flat Saturn-like ellipse. Orbital velocity drives Doppler beaming and
color shift, while gravitational redshift, a narrow photon ring, and an opaque captured-ray shadow
complete the visual hierarchy. This is a cinematic approximation rather than a scientific Kerr
simulation; a future renderer may add spin and a physically derived camera model without changing
the transparent-window contract.

Pixels outside the black hole and its emitted light remain fully transparent. The shader cannot
lens the live desktop behind the window because that desktop is not available as a sampled texture.

## Runtime policy

- Interaction refresh target: approximately 60 Hz.
- Idle refresh target: approximately 10 Hz.
- Quality changes use wall-clock sustained thresholds to avoid oscillation.
- Ray integration uses five distinct budgets: 64, 72, 80, 88, and 96 steps. The 64-step floor
  preserves both lensed disk images; idle savings primarily come from the 10 Hz frame rate.
- The current timing sample measures CPU submission time, not GPU time. True GPU timing requires
  checking `TIMESTAMP_QUERY` support and is deferred to the performance milestone.

## File safety

The default policy is trash-only. `FileSystemService` exposes inspection and move-to-trash; it does
not expose permanent deletion. The Phase 0 application does not call either operation. A future
operation coordinator must perform validation outside the renderer and report completion through
domain events.

## Drag-and-capture boundary

`winit` emits one hover or drop event per path and does not expose a public end-of-drop event. The
desktop layer therefore collects paths in first-seen order and the application flushes them when the
event loop is about to wait. On the supported Windows and macOS backends, all paths from one native
drop are emitted together, so this preserves a multi-file drop as one event-loop-coalesced batch. It
is not described as a platform-level transaction because two unrealistically close native drops
could theoretically be coalesced.

The core validates each batch atomically, rejects empty paths and batches above 256 unique targets,
and drives a bounded deterministic visual queue. Each target retains its own single-path state
machine. Renderer-facing snapshots contain only a generated visual identifier, phase, progress, and
orbit lane; paths never cross into the renderer. The visual timeline stops at the event horizon and
does not emit consumption-complete events or invoke the filesystem service.

## Deferred platform work

- Per-target file-object rendering and visual orbit animation
- Interactive and click-through window modes
- Native application discovery and uninstall
- Audio and update services
- Signing, notarization, packaging, and installer-size gates
