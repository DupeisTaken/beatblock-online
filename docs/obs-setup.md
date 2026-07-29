# OBS, reconstructed streams, and text exports

The Online shell and hidden runtime own four stable slots: Stream A, B, C, and D. Select a Player in the Room inspector, open **Broadcast**, and assign that candidate to a slot. Reassigning a slot does not require changing the OBS scene.

The host's revisioned configuration is labeled **Host Plan**. A host-granted Commentator receives the same plan read-only and may explicitly enable **This PC** mirroring after the resource warning. A Commentator needs the locked chart locally (or must accept the host transfer) before local video can start.

## Install the native OBS source

Open the BBT installer, enable **Install OBS 32 source (restart OBS)**, and choose Install/Update. Close OBS before installation, then restart it. The installer discovers the standard `%ProgramFiles%\obs-studio` locations automatically. For a portable or custom installation, type its OBS root or use **Browse...**; the selected folder must contain `bin\64bit\obs64.exe`. Selections of the root, `bin`, `bin\64bit`, or the executable itself are normalized to the OBS root, and the explicit path is preserved when Windows requests administrator access. After a successful installation, that custom location is rediscovered from the verified install record.

In OBS, press **+** in Sources and add **Beatblock Online Player Stream** for
each Stream A-D video feed. Then add **Beatblock Online Audio** for each
renderer audio feed you want in the mixer and choose Stream A-D or Autoplay.
Audio fine sync is independently adjustable from 0-2000 ms and defaults to
0 ms. The installed DLL and locale live under
`%ProgramData%\obs-studio\plugins\beatblock-online-obs`; the installer records
and verifies their hashes. The reviewed alpha artifact has been physically
loaded by OBS Studio 32.0.4 x64.

> **One-time upgrade:** existing Player Stream scene items become video-only.
> Their obsolete `capture_audio` setting is ignored. Add separate Beatblock
> Online Audio sources to the scene and route those sources in the OBS mixer.

The player source keeps a read-only mapping of the corresponding triple-buffered
RGBA frame ring. Header version 3 declares the payload as RGBA8 display/sRGB
bytes; readers reject an unknown encoding instead of displaying incorrect
colors. OBS uploads that single texture through its supported sRGB source draw
path, preserving the display-encoded bytes emitted by LÖVE. A versioned aligned
sequence commits each completed frame, and OBS verifies that sequence after
copying so a concurrent publish cannot produce a torn frame. The consumer must
use aligned read-only snapshots for this check: Windows interlocked
compare/exchange operations are writes and will crash against a `FILE_MAP_READ`
view. Assigning a participant to that stable slot happens inside Beatblock under
**Broadcast**. A source can therefore appear in OBS while showing no frame if
Online is closed, the slot is unassigned, the local chart is unavailable, or
its renderer has not published a frame. Check **Host Plan** and **This PC**
status before repairing the plugin.

After 1.5 seconds without a committed frame, the source releases its retained
CPU buffer and GPU texture. It also closes and retries the file mapping at a
bounded cadence, allowing a stopped or atomically replaced renderer ring to
recover without recreating the OBS source.

Each independent audio source owns a private OBS Application Audio Capture
child and reroutes that child's samples into its own mixer channel. Stream A-D
uses the exact stable renderer title, and Autoplay uses
`Beatblock Online Autoplay`; none can drift to the host game or a sibling.
Renderer audio already follows the delayed renderer clock, so fine sync should
normally remain 0 ms. On an OBS build without Application Audio Capture, only
the affected audio source shows a warning; video continues independently.

Renderer windows remain minimized rather than fully hidden. OBS Application
Audio Capture includes minimized windows in process discovery but rejects
windows whose Windows visibility flag is cleared. OBS reconnects to the stable
title after the runtime relaunches a renderer.

The runtime automatically mutes each hidden renderer's Windows audio sessions
across active render endpoints after launch while OBS captures its process
loopback. Matching is by the exact child PID only—never executable name—so the
host game and unrelated Beatblock processes are not candidates. The worker
keeps exact-PID discovery active at a bounded rate for the renderer's lifetime,
including when its audio session appears more than five seconds after launch.
It retains every session's original mute state and restores it before teardown
or after process exit. If Core Audio enumeration or muting fails, the runtime
reports a warning and leaves playback unchanged while discovery continues.

Some audio drivers expose process loopback after session volume instead. If the
OBS meter becomes silent when automatic isolation is enabled, open Online
**Settings** and disable **Renderer desktop mute**, then route the renderers to
a non-monitored or virtual endpoint manually.

## Optional Autoplay mix

The host can enable **Autoplay Mix** in Broadcast before countdown. It launches
one additional audio-only Beatblock process, follows the currently featured
renderer's delayed clock, and publishes no video frame ring. Beatblock's native
note logic is used to produce the chart song plus one normal perfect hitsound
for each positive scoring opportunity, including taps, holds, bounce, side, and
extra-tap paths. Mines and mine-holds are deliberately avoided; no miss or
barely sound is synthesized.

Autoplay requires an active featured renderer. Enablement and its featured clock
source are locked during countdown/gameplay. Host-granted Commentator mirrors
receive the optional plan field and may reproduce the source only after enabling
This PC mirroring.

Autoplay costs one additional full Beatblock audio/simulation process even
though it allocates no export canvases or video ring. Keep only the channels
needed by the production mix unmuted in OBS: each A-D source contains that
renderer's song, and Autoplay also contains the song. Unmuting more than one
combined channel duplicates the song and can produce comb filtering.

Default renderer settings are Full mode at 1280x720, 60 fps, with a 500 ms buffer. Delay is clamped to 250-1500 ms. Full mode publishes Beatblock's complete native composition: chart backgrounds and video, decorative entities, blocks, hit feedback, chart and Online HUDs, palette/accessibility conversion, and screen-space effects. Optional Clean mode uses the native base gameplay canvas without the final on-top composition, but still applies Beatblock's palette/accessibility shader; it no longer exposes red palette-index artwork or removes chart-authored scenery. Both modes preserve the source aspect ratio, and capture continues through Beatblock's native Results state. Disposable renderer profiles always select Beatblock's built-in default (`none`) Cranky costume instead of inheriting a costume from the host save.

The copy into the reusable OBS output canvas explicitly disables any stencil,
scissor, depth, or color-write mask left by the chart. Those masks have already
been applied to Beatblock's shaded source; applying them a second time would
leave pixels from older frames behind and create dithered VFX ghosts.

Renderer children finish Beatblock's threaded chart preload and then freeze the complete native simulation while they await source input. The Game state, global `flux` eases, and EntityManager share that pause boundary; hidden Player stencil canvases and procedural VFX therefore cannot age independently before playback. Cached delayed samples then advance the authored chart pre-roll while OBS output remains gated. This invisible warm-up lets Beatblock advance its own background events, eases, blocks, and effects instead of reconstructing them synthetically or collapsing them into the first visible frame. Once every assigned source reaches its first scoring note, the runtime releases the cohort on a shared clock, seeds each paddle once, and enables capture. Later frames install the delayed remote beat inside `GameManager` after its local audio-clock read but before native event processing. The remote mouse vector is rebuilt before `Player:update`; after that local input pass, the renderer restores the source Player's already snapped, offset, and native-capped angle before higher-layer notes perform collision. This prevents a repeated or skipped telemetry sample from applying the native angle cap twice, lagging the paddle, and leaving missed-note ghosts over authored VFX. Ordered raw tap edges carry the source player's already offset-adjusted judgement beat, with same-frame input flags as a bounded fallback.

Source-authored score keyframes travel on the reliable renderer stream and are selected against the same delayed motion timestamp as each video frame. A sequence-committed `stream-X.bbtscore` sidecar supplies the player's exact accuracy and totals after the hidden simulation update, so a local replay mismatch cannot change the OBS HUD. The hidden child is not allowed to enter its locally derived Results state; it waits for the player's final Results keyframe, installs the transmitted totals and average offset, and then renders Beatblock's native Results composition from that data.

Beatblock's native render stack is preserved rather than reimplemented: entities first draw by numeric layer into the base gameplay canvas or a chart-defined custom canvas; effect canvases collect recolor, displacement, and halftone masks; the on-top shader composites those masks plus waves, glitch, fisheye, and pixelation; HUD and on-top decorations follow; finally `shuv.finish()` applies palette/accessibility conversion and optional chromatic aberration. Full export copies that final shaded canvas. Keygen's white `noisetexture.png` is intentional shader carrier geometry for `Plasma`, not a missing-resource placeholder; the chart keeps Plasma and Spiral hidden until their authored reveal events. The isolated renderer releases the initial menu's retained entities and eases before Game initialization, matching SongSelect's native ownership handoff and preventing `MenuBackground` geometry from being composited over the chart as a false mask.

Choose **Advanced Export** in Broadcast to edit each Stream A-D independently. The complete visible controls are Full/Clean mode, 1280×720/1920×1080 output, 30/60 fps, and 250/500/1000/1500 ms delay. **Apply to Stream** stores those values even while the slot is unassigned; a later **Assign** uses that slot's saved configuration instead of resetting it to defaults. A 1080p60 selection is marked **High GPU Load**. Apply is disabled during countdown/gameplay.

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
intervening chart events. Set `BBT_PROBE_CHART`, `BBT_PROBE_VARIANT`, and
`BBT_PROBE_FIRST_NOTE_BEAT` to inspect another installed chart. The optional
`BBT_PROBE_TIMEOUT_SECS` bounds longer captures. `BBT_PROBE_BEATS_PER_SECOND`
may accelerate a state-handoff smoke test, but an accelerated run is not
evidence of visual timing fidelity. Set `BBT_PROBE_PRESTART_HOLD_SECS` to
reproduce a renderer parked after preload and confirm that a delayed assignment
does not age hidden chart entities before playback:

```powershell
$env:BBT_PROBE_GAME = 'C:\path\to\Beatblock.exe'
$env:BBT_PROBE_MODE = 'full'
$env:BBT_PROBE_CHART = 'levels/Finished levels/whatsakeygen/'
$env:BBT_PROBE_VARIANT = 'Easy'
$env:BBT_PROBE_FIRST_NOTE_BEAT = '0'
$env:BBT_PROBE_CAPTURE_BEAT = '100.5'
cargo run --manifest-path companion/Cargo.toml --example renderer_frame_probe
```

Renderers normalize chart-directory paths, resolve named variants to their
manifest objects, and satisfy Beatblock's threaded audio-preload gate with an
empty preload table before entering Game state. Each renderer disables
Beatblock's focus-loss mute, restores its master/song volume, and adopts the
stable title `Beatblock Online Renderer X` before it is minimized. It reads
delayed input from `stream-X.bbtstate`, draws and plays audio in that isolated
Beatblock process, paces capture at the configured FPS, and publishes
`stream-X.bbtframe`. LÖVE 12 asynchronous
texture reads return `GraphicsReadback` requests; the adapter polls two
independent requests and commits only completed image data. Per-slot tickets
discard superseded results. A capture exception is written to
`stream-X.bbterror` and appears as the slot's dashboard error instead of
crashing the child process.

For resource diagnosis, inspect the renderer-profile Lovely log under the local
runtime data directory. A mounted chart archive, loaded `initObject` class, and
absence of playback/resource errors rule out an asset-loading failure. If the
authored procedural background is visible but old note shapes accumulate over
it, diagnose paddle/collision replay before copying or replacing chart assets.

The host's normal Beatblock process remains available for play, or the host can
choose **Direct Next Race** from its participant inspector. Changing charts
relaunches every assigned slot against the new local chart path; stopping,
kicking, disconnecting, or converting a target to spectator clears its slots
and delayed telemetry buffers, republishes the Commentator plan, and causes OBS
to clear stale video.

## Broadcast-host performance budget

For four default 1280x720 60 fps renderer slots, use a modern 8-core/16-thread CPU, 32 GiB system memory, a dedicated GPU with 8 GiB VRAM, and an SSD with at least 2 GiB free. This is a recommended host configuration, not a claim that BBT alone consumes all of those resources: every slot is a separate Beatblock process, optional Autoplay adds a fifth audio/simulation process, while OBS, the encoder, the host game, chart assets, other sources, and the OS share the same machine.

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
6. For portable/custom OBS, verify the editable OBS field resolves to a folder containing `bin\64bit\obs64.exe`. An invalid explicit choice is reported and never falls back silently to another OBS copy.

## Diagnose missing, duplicated, or audible renderer audio

1. Confirm the scene has a separate **Beatblock Online Audio** source; an
   upgraded Player Stream is intentionally video-only.
2. Open the audio source properties and verify the selected A-D/Autoplay target
   and read-only connection status. Restart the renderer if it has not launched.
3. If the source warns that Application Audio Capture is unavailable, update to
   the supported OBS 32.x build. This does not affect Player Stream video.
4. If the desktop renderer is audible, check Online Settings for
   **Renderer desktop mute** and inspect the per-renderer isolation status. A
   warning means BBT left audio unchanged rather than risking the host process.
5. If the OBS meter is silent only while desktop mute is on, disable the setting
   and use a non-monitored/virtual endpoint; the audio driver is probably
   applying session volume before loopback.
6. If the song sounds doubled or phased, mute every combined A-D/Autoplay
   channel except the one intended for the program mix.

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

`state.json` contains the room and stream-slot snapshot. `gameplay.json` contains the host game's local gameplay snapshot. Featured files have one delayed-state writer and are cleared when no participant is featured. Each `streams\A-D` directory also has stable `player_name.txt`, `accuracy.txt`, `combo.txt`, `misses.txt`, and `rank.txt` paths. Those five files are cleared when the slot is unassigned or its participant disappears, so an OBS Text source cannot retain the previous player's values indefinitely.

## Rebuild the reviewed plugin artifact

Run `pnpm build:obs` on Windows with Visual Studio C++ Build Tools. The script downloads the official OBS 32.0.4 source and portable x64 archives, verifies both published SHA-256 checksums, generates an import library from the pinned `obs.dll`, and writes the generated plugin plus its source/artifact digest manifest under `artifacts/obs`. Pass `-ObsDirectory` directly to `scripts/build-obs-plugin.ps1` only when intentionally testing against a locally installed OBS build.

`pnpm build` also rebuilds the pinned Lovely injector, embeds both native dependencies into the installer, validates the generated files, and writes SHA-256 checksums. GitHub Actions uses the same command for manual artifacts and tagged releases; see [the release workflow](releasing.md).

## Third-party local API

Read-only loopback routes are `GET /v1/state`, `/v1/room`, `/v1/players`, `/v1/run`, `/v1/streams`, and `WS /v1/events`. REST clients send the per-install token in `Authorization: Bearer <token>`. Native WebSocket clients may use the same header; browser WebSocket clients request the `bbt-token.<token>` subprotocol. Tokens are never accepted in query strings, where browser history and HTTP tracing could expose them.

The runtime generates a 256-bit per-install token, replaces an empty or malformed token file on startup, and rotates it from the in-game command. Treat it as a credential and do not log or share it. The service exists only after entering Online, binds only to 127.0.0.1, and rejects missing credentials and browser origins other than exact loopback HTTP origins. Passwords, absolute chart paths, and credentials never appear in API responses.
