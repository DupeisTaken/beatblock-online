# Beatblock Online OBS plugin

This native OBS 32 plugin registers the stable **Beatblock Online Player Stream** source for runtime slots A-D. The reviewed installer artifact is built and smoke-tested against OBS Studio 32.0.4 x64.

Configure a build against an OBS SDK that exports `OBS::libobs`:

```text
cmake -S obs-plugin -B obs-plugin/build -Dlibobs_DIR=<obs-sdk>/cmake
cmake --build obs-plugin/build --config Release
```

For the pinned Windows release artifact, run `pnpm build:obs`. The repository script verifies the official OBS 32.0.4 source and runtime checksums and links against the export table of that pinned `obs.dll`.

`scripts/build-windows.mjs` embeds the reviewed artifact from `artifacts/obs`. The plugin build records the current `plugin.c` and DLL SHA-256 digests beside the artifact; the installer build refuses to embed a missing, modified, or stale artifact. A development build can override the DLL with `BBT_OBS_PLUGIN_DLL` when its matching `.build.json` manifest is present. The installer rejects empty, non-PE, or incorrectly exported payloads before touching OBS and installs valid files under `%ProgramData%\obs-studio\plugins\beatblock-online-obs`.

Video frame ingestion reads the hidden runtime's versioned frame rings under `%LOCALAPPDATA%\BeatblockOnline\BeatblockOnline\data\render-streams`. Audio is deliberately left to OBS Application Audio Capture until process-specific capture and a song-only fallback can be certified.
