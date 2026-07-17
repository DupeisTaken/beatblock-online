# OBS 32.x trial

Install the native plugin and create Player Stream sources A-D. Add OBS Application Audio Capture for the featured renderer. Record the exact OBS build, OBS log, source-menu screenshot, and source property screenshots. Source registration and module loading have passed on OBS Studio 32.0.4 x64.

Configure every slot from the in-game **Broadcast** workspace. Verify host and Commentator video after runtime restart, slot reassignment without scene changes, featured switching, single-process audio, 250/500/1500 ms A/V delay, and `featured_accuracy.txt` alignment. Confirm that Open Exports reaches the same files used by OBS. Pass only when the displayed frame, audio, and exported text identify the same buffered participant moment. Record audio fallback and measured offset.

For every 60 fps slot, capture at least ten uninterrupted minutes and record the runtime frame sequence, reported renderer drops, OBS rendered frames, OBS missed frames, and measured input-to-video delay. Pass video only at 59.94-60.06 delivered fps, under 1% drops, no stale frame more than 1.5 seconds after stop/disconnect, and measured delay within one frame plus network jitter of the configured delay. Reassign each slot and advance a two-chart setlist during the run.
