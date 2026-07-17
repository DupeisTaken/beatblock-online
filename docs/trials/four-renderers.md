# Four-renderer performance trial

## Recommended test host

Use a modern 8-core/16-thread CPU, 32 GiB system RAM, a dedicated GPU with 8 GiB VRAM, and an SSD with 2 GiB free for the baseline four-slot 1280x720 60 fps trial. Four 1920x1080 60 fps slots are experimental; use a modern 12-core/24-thread CPU, 32 GiB or more RAM, and 12 GiB or more VRAM when testing that configuration.

The calculated copy floors are 843.75 MiB/s for four 720p60 RGBA streams and 1,898.44 MiB/s for four 1080p60 streams. Four fixed frame mappings reserve 94.92 MiB regardless of selected output resolution. These values exclude OBS copies, GPU texture upload, composition, encoding, renderer assets, and the host game.

## Record

Record:

- CPU model, physical/logical core count, package utilization, per-process utilization, and clock/thermal throttling;
- GPU model, driver, GPU engine utilization, dedicated/shared memory, encoder utilization, and temperature;
- installed and peak committed/working-set memory for the runtime, host game, every renderer, and OBS;
- disk model, free space, active time, read/write bytes per second, and any page-file pressure;
- network adapter, link type, interface send/receive rate, packet loss, room size, and simultaneous streaming bitrate;
- OS, Beatblock build, chart, BBT build, renderer mode/resolution/FPS, OBS build, encoder, and scene contents.

From the in-game roster and **Spectate + OBS** overlay, assign four different participants to Streams A-D. Run the host as a player while all slots render 1280x720 at 60 fps with a 500 ms buffer for ten minutes. Exercise blocks, taps, holds, mines, multi-paddle changes, Full and Clean mode, participant/broadcast appearance, featured switching, and stream reassignment. Confirm the dashboard keeps stable slot names and reports frame/drop status. Pass with under 1% renderer frame drops and no material host-player frame-time regression. Repeat mixed FPS and 1080p configurations and verify warnings plus graceful degradation.

Also pass only when the runtime stays below its 30 MiB idle working-set gate before renderer assignment, memory does not trend upward over the ten-minute run, disk activity returns to idle after shutdown, and all renderer/runtime child processes exit. Record CPU/GPU headroom rather than accepting a run pinned at 100%; the recommended pre-assignment ceiling is 80% so launch, chart, and encoder bursts remain absorbable. For WAN trials, compare measured host upload with the modeled 3.4 Mbps per remote peer at maximum snapshot size and note any transport overhead, loss, or retransmission.
