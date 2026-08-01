# OBS 32.1.2 trial

**Status: pending.** Source registration and module loading have passed on OBS
Studio 32.0.4 x64. That compile/load result does not validate the OBS 32.1.2
color, mixer, process-loopback, desktop-mute, or long-run behavior below. Do not
mark the physical compatibility gate complete until this checklist is recorded
against OBS Studio 32.1.2 on supported Windows hardware.

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

| Gate                 | Required result                                                                                               | Status  | Evidence / measurements |
| -------------------- | ------------------------------------------------------------------------------------------------------------- | ------- | ----------------------- |
| Build identity       | Installer, built plugin, installed plugin, and checksums agree; report names the tested commit                | PENDING |                         |
| OBS registration     | OBS 32.1.2 x64 log loads the module and registers both BBT source types                                       | PENDING |                         |
| Color                | Raw renderer, game window, and OBS palette pixels differ by at most 1 per 8-bit channel                       | PENDING |                         |
| Mixer routing        | A-D and Autoplay are independent sources and cannot drift to host/sibling processes                           | PENDING |                         |
| Desktop mute         | Host remains audible; renderer capture remains audible in OBS; original states restore in every teardown case | PENDING |                         |
| Delay and A/V sync   | 250/500/1500 ms stay within one 60 fps frame plus measured network jitter                                     | PENDING |                         |
| Hitsounds            | One normal hit per positive opportunity; none for misses, barely, mines, or mine-holds                        | PENDING |                         |
| Ten-minute soak      | 59.94-60.06 fps, under 1% renderer drops, no stale frame over 1.5 s, drift, duplication, or leak              | PENDING |                         |
| Reassignment/results | Reassignment, featured switching, two-chart advance, Results, and cleared text exports stay aligned           | PENDING |                         |

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
