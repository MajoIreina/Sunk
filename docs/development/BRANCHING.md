# Sunk Branch And Worktree Management

## Branch ownership

- `main` owns the Rust/wgpu implementation and is developed in `L:\item`.
- `unity` owns the Unity 6/URP implementation and is developed in `L:\Sunk-Unity`.
- The product name is `Sunk` on both branches. Technology-specific project files belong only to
  their owning branch.

The `unity` worktree is linked to the Git repository metadata under `L:\item\.git`. Do not move or
delete that metadata while the worktree exists. Use Git worktree commands when a worktree must be
relocated or removed.

## Unity project boundary

`L:\Sunk-Unity\unity\Sunk` is the only Unity project root. Unity Hub must open that directory, not
the repository root and not `L:\item`.

Commit:

- `Assets/**` and every corresponding `.meta` file;
- `Packages/manifest.json` and `Packages/packages-lock.json`;
- `ProjectSettings/**`;
- native source and intentional runtime plug-ins.

Never commit:

- `Library`, `Temp`, `Obj`, `Build`, `Builds`, `Logs`, or `UserSettings`;
- IDE-generated solutions and project files;
- native build directories or repository `artifacts`.

Do not globally ignore `.meta`, `.dll`, or `.bundle`. An intentional native plug-in under
`Assets/Sunk/Plugins` is part of the product and must be reviewable.

## Moving shared changes

The branches are alternative implementations. Do not merge `unity` wholesale into `main` or
`main` wholesale into `unity` unless the project owner explicitly approves replacing a route.

For a genuinely shared change, inspect the source commit and transfer only the relevant file or
commit. Never transfer Unity generated caches, Rust build output, or technology-specific CI into the
other branch.

Before every commit, verify the current directory, branch, and worktree status. Run
`tools/check-unity-layout.ps1` on `unity`, and confirm that `main` remains clean after Unity work.
