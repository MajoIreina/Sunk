# Sunk

Sunk is a cross-platform Rust desktop application that renders a procedural, transparent black hole
as its primary interface. The project targets Windows (DX12) and macOS (Metal) through `wgpu` and
`winit`.

The current foundation provides:

- a transparent, borderless, always-on-top desktop window;
- a procedural WGSL black-hole and accretion-disk prototype;
- Windows DirectComposition presentation and transparent-surface capability checks;
- a platform-independent interaction state machine;
- ordered multi-file drop aggregation and a deterministic, path-free visual capture timeline;
- adaptive render-quality tiers with hysteresis;
- validated settings and a system-trash-only filesystem boundary;
- Windows and macOS CI with a 50 MiB release-binary gate.

The application does **not** delete or move files yet. A file drop only queues visual
attraction/capture/orbit/event-horizon state and temporarily increases the black hole's response.
The renderer never receives file paths. Permanent deletion is disabled by default and no
permanent-delete executor is exposed.

## Prerequisites

- Rust `1.97.1` (selected automatically by `rust-toolchain.toml`)
- Windows: Visual Studio 2022 Build Tools with the C++ workload and Windows SDK
- macOS: current Xcode Command Line Tools

## Develop

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p sunk-app --bin sunk
```

Press `Esc` to exit the prototype.

## Workspace

```text
crates/sunk-app         winit lifecycle and module orchestration
crates/sunk-core        interaction state and adaptive quality policy
crates/sunk-desktop     cross-platform window and input normalization
crates/sunk-renderer    wgpu surface, pipeline, and procedural renderer
crates/sunk-filesystem  inspection and move-to-trash boundary
crates/sunk-settings    validated TOML settings model
shaders/                     WGSL shader sources
docs/                        architecture notes and delivery plan
```

The product and technical baseline is
[`Sunk_Desktop_Development_Document_v0.1.docx`](Sunk_Desktop_Development_Document_v0.1.docx).

## Safety and scope

- The renderer never calls filesystem APIs.
- Native per-file drag events are coalesced at the event-loop boundary, de-duplicated in first-seen
  order, and validated as one atomic batch before visual work is queued.
- Permanent deletion and uninstall are outside Phase 0.
- Transparent composition requires a non-opaque swapchain mode; startup fails clearly if the
  platform cannot provide one.
- macOS transparency and Metal behavior must be smoke-tested on real macOS hardware or CI.

Licensed under GPL-3.0-only. See [`LICENSE`](LICENSE).
