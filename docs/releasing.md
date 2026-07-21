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
3. Download the official OBS Studio 32.0.4 source and Windows x64 archives, verify their published SHA-256 checksums, generate the `obs.dll` import library, and build the BBT source plugin. The build writes a manifest containing the exact `plugin.c` and DLL SHA-256 digests.
4. Verify that the OBS build manifest still matches both the reviewed C source and generated DLL, then build the lean Rust runtime, embed the runtime, Lovely, and OBS payloads into the installer, and package both Lua distributions. A direct `node scripts/build-windows.mjs` invocation fails closed and requests `pnpm build:obs` when the ignored native artifact is missing, stale, or modified.
5. Verify that every native output is an x64 Portable Executable and write `release/SHA256SUMS.txt`.

Generated files are written under `artifacts/`, `release/`, and `mod/releases/`. These directories are ignored by Git. `release/BeatblockOnlineInstaller.exe` is both the GitHub Actions staging artifact and the single stable local review copy.

The checked-in Online menu and Windows installer icons share one deterministic
generator in `scripts/generate-icons.py`. Their canonical globe silhouette is
the cleaned, transparent `scripts/assets/globe-template.png`; keep the source
free of checkerboard pixels. The approved native menu trace is retained
separately as `scripts/assets/globe-template-menu.png` so high-resolution
installer changes cannot resample it. Regenerate both icons after a design
change, then verify that no stale binary asset remains:

```powershell
python scripts/generate-icons.py
python scripts/generate-icons.py --check
```

The script requires Pillow. The generated 72 px PNG is copied into both mod
distributions and embedded in the installer payload. The installer PNG is used
by the Slint window, while its multi-resolution ICO is linked into the Windows
executable for Explorer and taskbar rendering.

## GitHub Actions

`.github/workflows/release.yml` runs on the pinned `windows-2022` hosted image:

- A manual **Run workflow** build validates and uploads a 14-day workflow artifact without publishing a release.
- Pushing the exact `v<package.json version>` tag runs the same tests and build, uploads the workflow artifact, and creates a GitHub Release with the installer, OBS plugin, mod ZIPs, checksums, and generated release notes. A mismatched tag fails before the native build.
- Tags containing `-` (for example, `v0.3.0-beta.1`) publish as prereleases.

Third-party actions are pinned to full commit SHAs. The build job has read-only repository access plus the OIDC permissions needed to produce GitHub artifact attestations. A separate environment-gated publication job is the only job with `contents: write`; it downloads the reviewed workflow artifact and publishes those exact files. CI and release generation also regenerate the protocol schema and fail if the checked-in schema drifts from the TypeScript source.

CI and release builds install the pinned `cargo-audit 0.22.2` scanner and check
the current RustSec database. The repository audit policy is limited to the
supported x64 Windows product graph. Its two documented `quick-xml` exceptions
come exclusively from Slint's Linux-only `wayland-scanner` lockfile branch; the
Windows dependency tree contains no `quick-xml`. Remove those exceptions as
soon as upstream `wayland-scanner` accepts `quick-xml` 0.41 or later.

The workflow produces GitHub artifact attestations for the installer, OBS plugin, checksums, and mod bundle. Those attestations establish repository/workflow provenance but do not replace Windows Authenticode signing. Until a protected code-signing identity is configured, Windows may still show an unknown-publisher warning; verify the release checksum and GitHub attestation before running the installer.

Create and push a release tag only after the normal CI checks pass:

```powershell
git tag -a v0.3.0-beta.1 -m "Beatblock Online v0.3.0-beta.1"
git push origin v0.3.0-beta.1
```

The GitHub Actions run is the source of published binaries. Do not commit generated executables or DLLs back into the repository.
