# Beatblock Together OBS plugin

This native OBS 32 plugin registers the stable **Beatblock Together Player Stream** source for runtime slots A-D. The reviewed installer artifact is built and smoke-tested against OBS Studio 32.0.4 x64.

Configure a build against an OBS SDK that exports `OBS::libobs`:

```text
cmake -S obs-plugin -B obs-plugin/build -Dlibobs_DIR=<obs-sdk>/cmake
cmake --build obs-plugin/build --config Release
```

For the pinned Windows release artifact, run `pnpm build:obs`. The repository script verifies the official OBS 32.0.4 source checksum and links against the export table of the locally installed OBS runtime.

`scripts/build-windows.mjs` embeds the reviewed artifact from `artifacts/obs-32.0.4`. A development build can override it with `BBT_OBS_PLUGIN_DLL`. The installer rejects empty, non-PE, or incorrectly exported payloads before touching OBS and installs valid files under `%ProgramData%\obs-studio\plugins\beatblock-together-obs`.

Video frame ingestion reads the hidden runtime's versioned frame rings under `%LOCALAPPDATA%\BeatblockTogether\BeatblockTogether\data\render-streams`. Audio is deliberately left to OBS Application Audio Capture until process-specific capture and a song-only fallback can be certified.
