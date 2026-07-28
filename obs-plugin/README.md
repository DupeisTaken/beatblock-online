# Beatblock Online OBS plugin

This native OBS 32 plugin registers the stable **Beatblock Online Player Stream** source for runtime slots A-D. The reviewed installer artifact is built and smoke-tested against OBS Studio 32.0.4 x64.

Configure a build against an OBS SDK that exports `OBS::libobs`:

```text
cmake -S obs-plugin -B obs-plugin/build -Dlibobs_DIR=<obs-sdk>/cmake
cmake --build obs-plugin/build --config Release
```

For the pinned Windows release artifact, run `pnpm build:obs`. The repository script verifies the official OBS 32.0.4 source and runtime checksums and links against the export table of that pinned `obs.dll`.

`scripts/build-windows.mjs` embeds the reviewed artifact from `artifacts/obs`. The plugin build records the current `plugin.c` and DLL SHA-256 digests beside the artifact; the installer build refuses to embed a missing, modified, or stale artifact. A development build can override the DLL with `BBT_OBS_PLUGIN_DLL` when its matching `.build.json` manifest is present. The installer rejects empty, non-PE, or incorrectly exported payloads before touching OBS and installs valid files under `%ProgramData%\obs-studio\plugins\beatblock-online-obs`.

Video frame ingestion reads the hidden runtime's versioned frame rings under `%LOCALAPPDATA%\BeatblockOnline\BeatblockOnline\data\render-streams`. Audio uses OBS's own private `wasapi_process_output_capture` source and its supported `reroute_audio` procedure to feed the Player Stream mixer channel. Each source derives a title-priority target such as `Beatblock Online Renderer A:SDL_app:Beatblock.exe` from its selected slot; the target cannot drift to the host game or a sibling stream. The renderer is minimized, not fully hidden, because OBS includes minimized windows but rejects windows without the Windows visibility flag. Audio and video share the delayed renderer clock, so fine sync defaults to 0 ms and applies only to the private audio child. If OBS does not register Application Audio Capture, the plugin logs a warning and continues video-only. Windows process loopback does not suppress the renderer's desktop playback; OBS captures before the Windows session volume, so mute the renderer session in Volume Mixer or route it to a non-monitored/virtual endpoint when local playback is unwanted.
