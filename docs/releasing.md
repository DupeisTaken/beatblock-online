# Reproducible release workflow

Compiled binaries are generated outputs and are not stored in Git. Release builds require
PowerShell, Node.js with pnpm, the pinned Rust toolchains, and Visual Studio Build Tools. Local
and hosted release builds use the same entry point:

```powershell
pnpm install --frozen-lockfile
pnpm build
```

The Windows build performs these steps:

1. Build the TypeScript protocol package.
2. Clone Lovely Injector `v0.9.0`, verify commit `91759da5702618c3b940fbbe8135954414c0ef34`, apply `third-party/lovely-no-console.patch`, and build `version.dll` with the pinned `nightly-2026-07-15` Rust compiler.
3. Download the official OBS Studio 32.0.4 source and Windows x64 archives, verify their published SHA-256 checksums, generate the `obs.dll` import library, and build the BBT source plugin.
4. Build the lean Rust runtime, embed the runtime, Lovely, and OBS payloads into the installer, and package both Lua distributions.
5. Verify that every native output is an x64 Portable Executable and write `release/SHA256SUMS.txt`.

Generated files are written under `artifacts/`, `release/`, and `mod/releases/`. These directories are ignored by Git.

## GitHub Actions

`.github/workflows/release.yml` runs on the pinned `windows-2022` hosted image:

- A manual **Run workflow** build validates and uploads a 14-day workflow artifact without publishing a release.
- Pushing a `v*` tag runs the same tests and build, uploads the workflow artifact, and creates a GitHub Release with the installer, OBS plugin, mod ZIPs, checksums, and generated release notes.
- Tags containing `-` (for example, `v0.3.0-alpha.2`) publish as prereleases.

The workflow grants `contents: write` only to the build job because GitHub requires that permission to create a release. All other CI jobs use read-only repository permissions.

Create and push a release tag only after the normal CI checks pass:

```powershell
git tag -a v0.3.0-alpha.2 -m "Beatblock Together v0.3.0-alpha.2"
git push origin v0.3.0-alpha.2
```

The GitHub Actions run is the source of published binaries. Do not commit generated executables or DLLs back into the repository.
