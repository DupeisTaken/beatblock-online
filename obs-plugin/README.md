# Beatblock Online OBS plugin

This native OBS 32 plugin registers the stable video-only **Beatblock Online
Player Stream** source for runtime slots A-D and the independent
**Beatblock Online Audio** source for A-D or Autoplay. The reviewed installer
artifact is built and smoke-tested against OBS Studio 32.0.4 x64.

Configure a build against an OBS SDK that exports `OBS::libobs`:

```text
cmake -S obs-plugin -B obs-plugin/build -Dlibobs_DIR=<obs-sdk>/cmake
cmake --build obs-plugin/build --config Release
```

For the pinned Windows release artifact, run `pnpm build:obs`. The repository script verifies the official OBS 32.0.4 source and runtime checksums and links against the export table of that pinned `obs.dll`.

`scripts/build-windows.mjs` embeds the reviewed artifact from `artifacts/obs`. The plugin build records the current `plugin.c` and DLL SHA-256 digests beside the artifact; the installer build refuses to embed a missing, modified, or stale artifact. A development build can override the DLL with `BBT_OBS_PLUGIN_DLL` when its matching `.build.json` manifest is present. The installer rejects empty, non-PE, or incorrectly exported payloads before touching OBS and installs valid files under `%ProgramData%\obs-studio\plugins\beatblock-online-obs`.

Video frame ingestion reads the hidden runtime's versioned frame rings under
`%LOCALAPPDATA%\BeatblockOnline\BeatblockOnline\data\render-streams`. Frame
header v4 declares RGBA8 display/sRGB payloads and carries an aligned generation
for every modulo slot. The producer invalidates a slot before reuse and commits
its generation only after the pixels, while the read-only OBS consumer verifies
both the slot and global generations around its copy. Older or unknown frame
contracts are rejected instead of being interpreted. OBS then uses its supported
single-texture sRGB draw path.

Each audio source owns an OBS `wasapi_process_output_capture` child and uses the
supported `reroute_audio` procedure to feed its own mixer channel. It derives an
exact title-priority target such as
`Beatblock Online Renderer A:SDL_app:Beatblock.exe` or
`Beatblock Online Autoplay:SDL_app:Beatblock.exe`; the target cannot drift to
the host game or a sibling stream. Fine sync defaults to 0 ms. If OBS does not
register Application Audio Capture, that audio source reports a warning without
affecting video.

Player Stream items created by earlier versions become video-only after update;
add independent audio sources once. The runtime normally mutes hidden renderer
desktop sessions by exact PID and restores their original state at teardown.
Systems whose driver applies session volume before process loopback can disable
that setting and use a non-monitored/virtual endpoint instead.
