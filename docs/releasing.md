# Reproducible release workflow

> Documentation: [Player Guide](player-guide.md) · [Technical Reference](technical-reference.md)

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
- Pushing the exact `v<package.json version>` tag runs the same tests and build, uploads the workflow artifact, and creates a GitHub Release with the installer, OBS plugin, mod ZIPs, checksums, and the matching reviewed `docs/releases/v<tag>.md` public note. Every public note must have a matching `docs/changelogs/v<tag>.md` technical changelog and an entry in `docs/releases/index.md`; the release-documentation validator checks the pairing, required sections, concise-note size, and GitHub Release-safe links before publication. The display title is generated as `v<Online version> for Beatblock <tested version>+` from `package.json`; the Git tag itself stays semver-only for update tooling.
- Tags containing `-` (for example, `v0.4.0-beta.1`) publish as prereleases;
  stable tags such as `v0.3.0` publish as official releases.

Third-party actions are pinned to full commit SHAs. The build job has read-only repository access plus the OIDC permissions needed to produce GitHub artifact attestations. A separate environment-gated publication job is the only job with `contents: write`; it downloads the reviewed workflow artifact and publishes those exact files. CI and release generation also regenerate the protocol schema and fail if the checked-in schema drifts from the TypeScript source.

CI and release builds install the pinned `cargo-audit 0.22.2` scanner and check
the current RustSec database. The repository audit policy is limited to the
supported x64 Windows product graph. Its two documented `quick-xml` exceptions
come exclusively from Slint's Linux-only `wayland-scanner` lockfile branch; the
Windows dependency tree contains no `quick-xml`. Remove those exceptions as
soon as upstream `wayland-scanner` accepts `quick-xml` 0.41 or later.

The workflow produces GitHub artifact attestations for the installer, OBS plugin, checksums, and mod bundle. Those attestations establish repository/workflow provenance but do not replace Windows Authenticode signing. Until a protected code-signing identity is configured, Windows may still show an unknown-publisher warning; verify the release checksum and GitHub attestation before running the installer.

Before tagging, add the concise public note, matching technical changelog, and
both links in the release-history index. Public notes are used verbatim outside
the repository, so their Markdown links must be absolute HTTPS URLs or
same-document anchors. Run the documentation contract locally:

```powershell
pnpm validate:release-docs
```

Create and push a release tag only after the normal CI checks pass:

```powershell
git tag -a v0.3.0 -m "Beatblock Online v0.3.0"
git push origin v0.3.0
```

The GitHub Actions run is the source of published binaries. Do not commit generated executables or DLLs back into the repository.

## Extending the tested Beatblock baseline

The installer accepts newer structurally valid builds by default, but those
builds remain unverified until the compatibility suite passes. Do not add
executable hashes or publish an installer only to allow a routine Beatblock
update.

1. Preserve `.reference` and place the new upstream build in separate tested and
   untouched folders. Record the complete displayed version and bracketed build
   token.
2. Run installer, injection, official/custom chart, score, reconnect, transfer,
   renderer, and OBS validation against the disposable tested copy.
3. If the build passes, update `package.json` `beatblockCompatibility`, the
   matching Rust/Lua tested-baseline constants, and the compatibility table.
   Contract tests must reject drift between those three representations.
4. If the build fails, triage the dedicated **Installer incompatible with latest
   Beatblock release** issue and fix the adapter. Do not weaken same-build room
   matching to hide gameplay differences.
5. The next Online release title will automatically advertise the newer tested
   baseline. Previously published installers continue accepting later game
   layouts unless the upstream change caused a real incompatibility.
