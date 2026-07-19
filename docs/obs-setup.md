# OBS, reconstructed streams, and text exports

The Online shell and hidden runtime own four stable slots: Stream A, B, C, and D. Select a Player in the Room inspector, open **Broadcast**, and assign that candidate to a slot. Reassigning a slot does not require changing the OBS scene.

The host's revisioned configuration is labeled **Host Plan**. A host-granted Commentator receives the same plan read-only and may explicitly enable **This PC** mirroring after the resource warning. A Commentator needs the locked chart locally (or must accept the host transfer) before local video can start.

## Install the native OBS source

Open the BBT installer, enable **Install OBS 32 source (restart OBS)**, and choose Install/Update or Repair. Close OBS before installation, then restart it. In OBS, press **+** in Sources, add **Beatblock Online Player Stream**, and choose Stream A-D. The installed DLL and locale live under `%ProgramData%\obs-studio\plugins\beatblock-online-obs`; the installer records and verifies their hashes. The reviewed alpha artifact has been physically loaded by OBS Studio 32.0.4 x64.

The player source keeps a read-only mapping of the corresponding triple-buffered RGBA frame ring. A versioned aligned sequence commits each completed frame, and OBS verifies that sequence after copying so a concurrent publish cannot produce a torn frame. The consumer must use aligned read-only snapshots for this check: Windows interlocked compare/exchange operations are writes and will crash against a `FILE_MAP_READ` view. Assigning a participant to that stable slot happens inside Beatblock under **Broadcast**. A source can therefore appear in OBS while showing no frame if Online is closed, the slot is unassigned, the local chart is unavailable, or its renderer has not published a frame. Check **Host Plan** and **This PC** status before repairing the plugin.

After 1.5 seconds without a committed frame, the source releases its retained
CPU buffer and GPU texture. It also closes and retries the file mapping at a
bounded cadence, allowing a stopped or atomically replaced renderer ring to
recover without recreating the OBS source.

The plugin exposes video sources only. Use OBS Application Audio Capture for the featured renderer. Exactly one child is audible: feature switching stops the previous audio process before the new featured child is enabled, and audio follows the configured delayed clock.

Default renderer settings are Full mode at 1280x720, 60 fps, with a 500 ms buffer. Delay is clamped to 250-1500 ms. The OBS competition view removes chart scenery, background images/video, and background noise while retaining the player, notes, hit feedback, chart-controlled HUD, online race HUD, palette/accessibility conversion, and screen-space effects. Backdrop suppression is scoped to the isolated renderer child and restored after every Game draw, so the host player's window is unchanged. Capturing occurs after the remaining gamestate composition, preventing raw palette-index artwork from appearing as a color mask. Optional Clean mode captures the same foreground-only gameplay canvas before final shading. Both modes preserve the source aspect ratio. Renderer children preload the chart and remain held until a genuinely delayed `playing` sample arrives—an initial sample is never allowed to bypass the configured buffer. Each frame reapplies the selected player's delayed beat, paddle, taps, and any material music-clock correction after hidden-window input has run.

Renderer output canvases explicitly use a `1.0` DPI scale. OBS dimensions are
physical pixels; allowing Windows display scaling to inflate a nominal
1280×720 canvas would make readbacks fail their exact-size guard and leave OBS
showing a stale or blank frame.

Assign and configure every stream before starting the synchronized countdown. The runtime rejects renderer reconfiguration during countdown or gameplay because a new child would not have observed the chart's earlier timed VFX events and could not reconstruct the original composition exactly. An active stream can still be stopped during a race.

Renderer processes receive their stream configuration through environment
variables, not command-line flags, because Lovely parses the game's command line
before the mod starts. They use a separate APPDATA profile and set Lovely's
`LOVELY_MOD_DIR` explicitly to that profile's dedicated BBT renderer adapter.
The explicit override is required on Windows because Lovely resolves its default
directory through the roaming known-folder API rather than trusting the
`APPDATA` environment variable. This prevents renderer children from loading the
player's normal mod set or sharing Lovely logs/dumps with the host game. Because
LÖVE also resolves Beatblock's save directory through a Windows known folder,
the renderer adapter disables save writes before entering Game state so a hidden
renderer cannot overwrite the player's save.

Developers can validate the raw producer without opening OBS by pointing the
physical probe at an isolated Beatblock test build. Run it once with
`BBT_PROBE_MODE=clean` and once with `BBT_PROBE_MODE=full`; it rejects torn,
transparent, uniformly black, or spatially empty frames and writes a BMP for
visual review. The probe follows Tutorial's real pre-roll; set
`BBT_PROBE_CAPTURE_BEAT` to inspect a later composed frame without skipping the
intervening chart events:

```powershell
$env:BBT_PROBE_GAME = 'C:\path\to\Beatblock.exe'
$env:BBT_PROBE_MODE = 'full'
$env:BBT_PROBE_CAPTURE_BEAT = '8'
cargo run --manifest-path companion/Cargo.toml --example renderer_frame_probe
```

Renderers normalize chart-directory paths, resolve named variants to their
manifest objects, and satisfy Beatblock's threaded audio-preload gate with an
empty preload table before entering Game state. The renderer child's numeric
audio settings are zeroed before Beatblock loads that chart's song. They read delayed input from
`stream-X.bbtstate`, draw in a hidden and muted Beatblock process, pace capture
at the configured FPS, and publish `stream-X.bbtframe`. LÖVE 12 asynchronous
texture reads return `GraphicsReadback` requests; the adapter polls two
independent requests and commits only completed image data. Per-slot tickets
discard superseded results. A capture exception is written to
`stream-X.bbterror` and appears as the slot's dashboard error instead of
crashing the child process.

The host's normal Beatblock process remains available for play. Changing charts
relaunches every assigned slot against the new local chart path; stopping,
kicking, disconnecting, or converting a target to spectator clears its slots
and delayed telemetry buffers and causes OBS to clear stale video.

## Broadcast-host performance budget

For four default 1280x720 60 fps renderer slots, use a modern 8-core/16-thread CPU, 32 GiB system memory, a dedicated GPU with 8 GiB VRAM, and an SSD with at least 2 GiB free. This is a recommended host configuration, not a claim that BBT alone consumes all of those resources: every slot is a separate Beatblock process, while OBS, the encoder, the host game, chart assets, other sources, and the OS share the same machine.

The code-visible minimums explain the headroom:

- Four fixed triple-buffer frame mappings reserve 94.92 MiB of system memory and backing-file space.
- Four 720p60 RGBA streams move at least 843.75 MiB/s through renderer readback before OBS performs its own copy, texture upload, composition, and encoding.
- The visible renderer/OBS pixel buffers account for at least 42.19 MiB of VRAM at 720p, but duplicated game assets, source canvases, driver staging, and encoder surfaces are outside that number.
- Four 1080p60 streams raise the raw pixel-copy floor to 1,898.44 MiB/s. This configuration is experimental and should be reduced to 30 fps, Clean mode, or fewer slots when drops appear.

Before assigning slots, keep the host game and OBS below 80% CPU and GPU utilization so renderer bursts have headroom. During the [four-renderer trial](trials/four-renderers.md), pass only if frame-ring drops stay below 1%, the host game has no material frame-time regression, and memory does not trend upward. Renderer mappings are file-backed, so Windows may report disk activity even though they are used as shared memory; measure physical writeback on the target system rather than treating raw RGBA copy throughput as a disk requirement.

## Diagnose a missing source

1. Open the installer Components tab. **OBS plugin — Installed** must say **Installed and hash verified**.
2. Restart OBS completely after installation.
3. Open OBS **Help → Log Files → View Current Log** and search for `[Beatblock Online] OBS player stream source registered` and `beatblock-online-obs.dll`.
4. If the source is present but remains black while the runtime reports published frames, close OBS and use **Repair Required Components**. This replaces an older plugin DLL before OBS can load it again.
5. If the row is Missing or Broken, close OBS and use **Repair Required Components**. The installer preserves unrelated OBS plugins and scenes.

## Text capture

The runtime atomically updates these video-aligned files under `%LOCALAPPDATA%\BeatblockOnline\BeatblockOnline\exports` while Online is active:

```text
featured_accuracy.txt
featured_combo.txt
featured_misses.txt
featured_name.txt
featured_rank.txt
song_name.txt
room_name.txt
state.json
gameplay.json
streams\A-D\...
```

Add a normal OBS Text source and enable **Read from file**. Featured files use the same delayed state selected by the featured renderer.

`state.json` contains the room and stream-slot snapshot. `gameplay.json` contains the host game's local gameplay snapshot. Featured files have one delayed-state writer and are cleared when no participant is featured.

## Rebuild the reviewed plugin artifact

Run `pnpm build:obs` on Windows with Visual Studio C++ Build Tools. The script downloads the official OBS 32.0.4 source and portable x64 archives, verifies both published SHA-256 checksums, generates an import library from the pinned `obs.dll`, and writes the generated plugin plus its source/artifact digest manifest under `artifacts/obs`. Pass `-ObsDirectory` directly to `scripts/build-obs-plugin.ps1` only when intentionally testing against a locally installed OBS build.

`pnpm build` also rebuilds the pinned Lovely injector, embeds both native dependencies into the installer, validates the generated files, and writes SHA-256 checksums. GitHub Actions uses the same command for manual artifacts and tagged releases; see [the release workflow](releasing.md).

## Third-party local API

Read-only loopback routes are `GET /v1/state`, `/v1/room`, `/v1/players`, `/v1/run`, `/v1/streams`, and `WS /v1/events`. Supply the per-install token as `?token=...`; it is generated by the runtime and should not be logged or shared. The service exists only after entering Online, binds only to 127.0.0.1, and rejects missing tokens and disallowed origins. Passwords, absolute chart paths, and credentials never appear.
