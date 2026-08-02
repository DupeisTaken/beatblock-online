# OBS 32.1.2 trial

**Status: blocked.** The integrated issue #28 branch loads and registers both
source types in OBS Studio 32.1.2 x64, installs the plugin into a selected portable
OBS copy, and renders all four stable player windows. The 2026-08-03 hardware run
still found two release blockers: an isolated Beatblock Online Audio source was
digital silence in the OBS recording, and the second chart never launched the
local game before the 30-second grace period expired. Keep the pull request draft
until both failures are fixed and the incomplete quantitative rows below pass.

## 2026-08-01 implementation rehearsal (partial)

The issue #28 worktree was exercised through the installer and an exact portable
OBS Studio 32.1.2 x64 build on Windows. OBS used a disposable 1280x720, 60 fps,
NV12, Rec. 709, Limited profile. The host selected the official **What's a
Keygen?** chart; Damoclism was not used. Stream A was assigned, unassigned, and
assigned again through Beatblock's Broadcast screen, and the OBS Player A and
Audio A sources recovered without restarting the room or OBS.

Computer-controlled visual checks confirmed that Stream A rendered the authored
background, palette, effects, blocks, paddle, tap judgements, and HUD while the
Audio A meter remained active. A 60-second passive rehearsal measured 59.98 fps
with no reported drops, sequence resets, or stale interval. A subsequent complete
chart run reached Beatblock's numeric Results screen in OBS instead of holding on
the terminal splash. Its final frame ring was header v4, 1280x720 RGBA, sequence
17,948 with 16 cumulative producer drops (0.09%). The renderer, runtime, portable
OBS, temporary profile, and test installation were then removed or stopped.

This is implementation evidence, not completion of issue #28. Exact sampled-pixel
deltas, all A-D and Autoplay routing/restoration cases, 250/500/1500 ms A/V
measurements, hitsound counting, and the ten-minute four-renderer soak still need
operator evidence. Until those rows are completed, any pull request should remain
draft and reference rather than close issue #28.

## 2026-08-03 integrated OBS 32.1.2 hardware validation (blocked)

The test used commit `7620e5aeb1204cc4213a8e81822cb2946900c942` after
merging current `origin/main`. The release installer targeted only the isolated
game copy at `E:\beatblock-online\.test\Beatblock` and the portable OBS copy at
`E:\beatblock-online-issue-28\.trial\obs-32.1.2`. Its SHA-256 was
`fb96d9717fc0bb2ad8d797a7478c7d9dc9c308f0804a0a626a892d5c57e14c91`.
The built and installed OBS plugin hashes both matched
`415963a85277c40819af9b7c3b0e737c7b9c3e266d7dc7a0e1b9de86f4925ee3`.

OBS loaded the module, registered Player Stream and Beatblock Online Audio, and
found exact stable renderer titles A-D plus Autoplay. A-D and Autoplay appeared as
separate active mixer sources. The official **What's a Keygen?** Easy chart ran to
numeric Results while Player Stream A rendered gameplay and Results in OBS. A-D
reported about 60 fps with zero initial drops; later B-D had 20-26 cumulative
producer drops across roughly 21,896 frames, remaining under one percent. Stream A
was reassigned from Full/500 ms to Clean/250 ms and restarted without changing
B-D. These observations validate discovery, source registration, the successful
single-chart video path, and the portable-plugin installation fix.

The mixer activity did not validate the recorded signal. After muting OBS Desktop
Audio, Mic/Aux, Audio B-D, and Autoplay so that only Audio A remained, repeated
FFmpeg volume detection windows returned `mean_volume: -91.0 dB` and
`max_volume: -91.0 dB`. The isolated Audio A interval from 00:44:00 through
00:44:20 in the recording is therefore digital silence even though the source was
active and the runtime reported the renderer session muted at the desktop. Earlier
non-silent recording windows included global or sibling audio and cannot count as
Audio A evidence. Until the native source produces non-silent recorded PCM while
desktop isolation remains enabled, mixer routing, desktop mute, Autoplay, and
hitsound acceptance remain blocked.

The second official chart, **Rhythmic Shield** Easy, also failed. After chart
selection recovered through Refresh Diagnostics, the room entered countdown and
Playing, but the local game never transitioned to gameplay. The runtime recorded
the participant as DNF with `Game did not start within the 30-second launch grace
period`; all four streams ended at sequence zero and `actualFps: 0`. Because the
second chart had no valid frames, the ten-minute sampler was stopped rather than
using an invalid run as partial soak evidence.

The 48:20 OBS recording is retained locally as
`.trial/evidence/2026-08-03 01-22-17.mp4` (2,213,223,244 bytes, SHA-256
`670710139479c50c03f6e873f9db2ece1b6cd56ce5814829fd5b107ae4e178f7`).
The final OBS log is `.trial/evidence/obs-32.1.2-v3-full.log`; the final runtime
exports are under `.trial/evidence/v3-runtime-exports/`; and the failed second-run
frame snapshot is `.trial/evidence/snapshot-v3/obs-32.1.2-hardware-latest.json`.
All portable OBS, game, runtime, renderer, Autoplay, muxer, and analysis processes
started for the run were closed after evidence capture.

## Passive evidence sampler

Build the current checkout and run its automated gates before starting a physical
trial. Close OBS while installing the freshly built plugin, then reopen OBS
Studio 32.1.2 x64. Use a disposable OBS profile and scene collection configured
for SDR 1280x720 at 60 fps; an HDR or high-frame-rate production profile cannot
support the exact sRGB comparison below. Use **What's a Keygen?** for the trial,
because charts whose mechanics end a renderer early cannot produce a valid soak.

The sampler is deliberately passive: it never launches, stops, mutes, assigns,
or configures OBS, Beatblock, the runtime, or renderer processes. It requires the
release installer, release checksums, built OBS DLL, and installed OBS DLL to
exist, and fails before sampling unless OBS is exactly 32.1.2 and the installed
plugin SHA-256 matches the current build artifact. From the repository root run:

```powershell
pnpm test:obs-trial
pnpm trial:obs-32.1.2
```

The default run samples every five seconds for ten minutes. To make a short
operator rehearsal, or to select a portable OBS installation, invoke the script
directly. A shortened rehearsal is not ten-minute soak evidence:

```powershell
powershell -NoProfile -File scripts/run-obs-32.1.2-trial.ps1 `
  -ObsDirectory 'D:\OBS Studio' `
  -DurationSeconds 60 `
  -SampleIntervalSeconds 5
```

The script writes `reports/trial-runs/obs-32.1.2-hardware-latest.json` and a
human-readable `.md` companion. They record the exact Git commit, OBS version,
installer/plugin/checksum hashes, bounded process CPU and memory samples, and
the sequence/drop counters from the read-only A-D renderer frame headers. CPU is
normalized across the machine's logical processors. Frame rate and drop
summaries are observation-window estimates; a sequence reset is retained as a
reset rather than counted as an enormous frame delta.

The generated report always says **MANUAL REVIEW REQUIRED**. It cannot measure
OBS's rendered/missed frames, inspect mixer routing, compare pixels, hear audio,
or decide the restoration cases below. Attach sanitized OBS logs and screenshots
and complete this matrix before changing the trial status.

| Gate                 | Required result                                                                                               | Status  | Evidence / measurements                                                                                          |
| -------------------- | ------------------------------------------------------------------------------------------------------------- | ------- | ---------------------------------------------------------------------------------------------------------------- |
| Build identity       | Installer, built plugin, installed plugin, and checksums agree; report names the tested commit                | PASS    | Integrated commit and exact matching hashes recorded above.                                                      |
| OBS registration     | OBS 32.1.2 x64 log loads the module and registers both BBT source types                                       | PASS    | Exact portable OBS 32.1.2 log and visible source menu.                                                           |
| Color                | Raw renderer, game window, and OBS palette pixels differ by at most 1 per 8-bit channel                       | PENDING | Visual Full/Clean inspection passed; no exact pixel delta.                                                       |
| Mixer routing        | A-D and Autoplay are independent sources and cannot drift to host/sibling processes                           | FAIL    | Separate active sources appeared, but isolated Audio A recorded at -91 dB digital silence.                       |
| Desktop mute         | Host remains audible; renderer capture remains audible in OBS; original states restore in every teardown case | FAIL    | Renderer state reported muted, but required OBS capture was silent; full restoration matrix is incomplete.       |
| Delay and A/V sync   | 250/500/1500 ms stay within one 60 fps frame plus measured network jitter                                     | PENDING | 250/500 ms configuration applied; no frame-accurate measurement and no 1500 ms case.                             |
| Hitsounds            | One normal hit per positive opportunity; none for misses, barely, mines, or mine-holds                        | BLOCKED | Isolated recorded audio was silent, so counts and exclusions are not measurable.                                 |
| Ten-minute soak      | 59.94-60.06 fps, under 1% renderer drops, no stale frame over 1.5 s, drift, duplication, or leak              | FAIL    | Successful first chart was shorter than ten minutes; second run had sequence zero and was intentionally stopped. |
| Reassignment/results | Reassignment, featured switching, two-chart advance, Results, and cleared text exports stay aligned           | FAIL    | First Results/reassignment passed; second chart timed out before local gameplay and produced a DNF.              |

Record Windows build, CPU, GPU/driver, memory, OBS encoder and color settings,
Beatblock version/build, BBT commit, chart variant, room roles, slot settings,
and every fallback. Capture OBS **View > Stats** at the start and end of the soak
because its rendered and missed-frame counters are not present in the BBT frame
ring. Keep only one combined A-D/Autoplay song channel unmuted while judging
audio, then confirm all BBT runtime/renderer children have exited after the
operator ends the trial.

Install the native plugin and create video-only Player Stream sources A-D plus
independent Beatblock Online Audio sources A-D and Autoplay. Record the exact
OBS build, OBS log, source-menu screenshot, source property screenshots, and
mixer routing.

Configure every slot from the in-game **Broadcast** workspace. Compare a 1:1
OBS frame with the raw renderer probe and visible game window; representative
palette pixels must differ by no more than one 8-bit value per channel. Verify
host and Commentator video after runtime/OBS restart, slot reassignment without
scene changes, featured switching, 250/500/1500 ms A/V delay, source-player HUD
accuracy, source-player Results totals/offset, the default Cranky costume, and
`featured_accuracy.txt` alignment. Confirm A-D and Autoplay appear as separate
mixer sources and Open Exports reaches the same files used by OBS. Unassign each
slot and remove its participant; its five stable text files must become empty
instead of retaining the previous player. Pass only when the displayed frame,
audio, Results, and exported text identify the same buffered participant
moment.

With automatic renderer desktop mute enabled, confirm every renderer session is
inaudible at the desktop while OBS meters and recordings retain audio. Confirm
the host game remains audible and unchanged. Relaunch/reassign renderers, stop
them normally and abnormally, restart the runtime, and verify original session
mute states are restored. Repeat with the fallback switch disabled and document
driver behavior.

Enable Autoplay before countdown with a featured renderer. Confirm its song and
normal positive hitsounds align within one 60 fps frame of the rendered
judgement, mines and mine-holds remain silent, and no miss/barely/duplicate hit
is heard. Confirm enablement is rejected without a featured renderer and locked
during countdown/gameplay. Keep only one combined A-D/Autoplay channel unmuted
while judging quality so duplicate song playback cannot mask a defect.

For every 60 fps slot, capture at least ten uninterrupted minutes and record the
runtime frame sequence, reported renderer drops, OBS rendered frames, OBS missed
frames, audio drift/duplication, and measured input-to-video delay. Pass video
only at 59.94-60.06 delivered fps, under 1% drops, no stale frame more than
1.5 seconds after stop/disconnect, and measured delay within one frame plus
network jitter of the configured delay. Pass audio only with no drift or
duplicates during the ten-minute run. Reassign each slot and advance a
two-chart setlist during the run.
