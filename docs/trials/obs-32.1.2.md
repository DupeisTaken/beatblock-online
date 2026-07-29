# OBS 32.1.2 trial

**Status: pending.** Source registration and module loading have passed on OBS
Studio 32.0.4 x64. That compile/load result does not validate the OBS 32.1.2
color, mixer, process-loopback, desktop-mute, or long-run behavior below. Do not
mark the physical compatibility gate complete until this checklist is recorded
against OBS Studio 32.1.2 on supported Windows hardware.

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
