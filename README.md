# Sunk - Unity implementation

This branch is the Unity implementation of **Sunk**, a cross-platform desktop application whose
primary interface is a procedural Gargantua-inspired black hole. It is an alternative to the Rust
implementation on `main`; it is not a mixed Rust/Unity workspace.

## Status

- Phase 1 Gargantua visual prototype
- Unity `6000.0.30f1` (Unity 6 LTS)
- Universal Render Pipeline `17.0.1`
- Windows IL2CPP module installed locally
- Product renderer, prototype scene, parameter asset, and EditMode tests are implemented
- Native window integration and file operations are not implemented yet

The first visual milestone now includes a full-screen mathematical Gargantua renderer: black-hole
shadow, photon ring, compact accretion disk and lensed secondary image, Kerr-inspired lensing,
Doppler beaming, redshift, and a procedural star field. See
[the prototype guide](docs/unity/GARGANTUA_PROTOTYPE.md).

## Worktree isolation

| Branch | Local worktree | Active implementation |
| --- | --- | --- |
| `main` | `L:\item` | Rust / wgpu |
| `unity` | `L:\Sunk-Unity` | Unity 6 / URP |

Open only `L:\Sunk-Unity\unity\Sunk` in Unity Hub. Unity-generated folders such as `Library`,
`Temp`, and `Logs` remain inside the Unity worktree and are ignored. Do not routinely merge the
entire `unity` branch into `main`; shared changes must be reviewed and selected explicitly.

See [the branch management rules](docs/development/BRANCHING.md) before moving files between the
two implementations.

## Prerequisites

- Unity Editor `6000.0.30f1`
- Windows: Windows Build Support (IL2CPP), Visual Studio 2022 Build Tools, and Windows SDK
- macOS: a matching Unity Editor installation and current Xcode command-line tools

## Develop

1. In Unity Hub, add and open `unity/Sunk`.
2. Open `Assets/Sunk/Scenes/GargantuaPrototype.unity`.
3. Keep all product-owned assets under `Assets/Sunk`.
4. Run the repository layout check before committing:

```powershell
pwsh -NoProfile -File tools/check-unity-layout.ps1
```

After Unity Personal is activated, verify the project headlessly:

```powershell
& 'C:\Program Files\Unity\Hub\Editor\6000.0.30f1\Editor\Unity.exe' `
  -batchmode -quit -projectPath 'L:\Sunk-Unity\unity\Sunk' `
  -logFile 'L:\Sunk-Unity\artifacts\unity-import.log'
```

## Repository layout

```text
unity/Sunk/                  Unity Hub project root
  Assets/Sunk/               product runtime, shaders, scenes, settings, and tests
  Packages/                  pinned Unity package manifest and lock file
  ProjectSettings/           versioned Unity project settings
docs/unity/                  Unity implementation specification
docs/development/            branch and repository management rules
native/windows/              future Windows native integration source
native/macos/                future macOS native integration source
tools/                       repository validation scripts
```

## Naming

- Product name: `Sunk`
- C# root namespace: `Sunk`
- Future assemblies: `Sunk.Core`, `Sunk.Rendering`, `Sunk.Interaction`, and other `Sunk.*` modules
- Windows output: `sunk.exe`
- macOS output: `Sunk.app`
- Bundle identifier: pending publisher identifier confirmation; do not invent one

Domain names such as `BlackHoleModel`, Gargantua, Accretion Disk, Event Horizon, and Kerr remain
part of the rendering model and are not legacy product names.

## Safety and licensing

File deletion and uninstall behavior are outside this foundation. Future file removal must default
to the operating-system trash, permanent deletion must require explicit confirmation, and rendering
code must never receive full filesystem paths.

Licensed under GPL-3.0-only. See [LICENSE](LICENSE). Before distribution, review compatibility
between the project license, Unity runtime terms, native plug-ins, and third-party packages.

The Unity technical baseline is
[Sunk Desktop Unity Development Document v0.2](docs/unity/Sunk_Desktop_Unity_Development_Document_v0.2.md).
