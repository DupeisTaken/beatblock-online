# Four-renderer performance trial

Record CPU, GPU, RAM, display driver, OS, Beatblock build, chart, BBT build, and OBS build.

From the in-game roster and **Spectate + OBS** overlay, assign four different participants to Streams A-D. Run the host as a player while all slots render 1280x720 at 60 fps with a 500 ms buffer for ten minutes. Exercise blocks, taps, holds, mines, multi-paddle changes, Full and Clean mode, participant/broadcast appearance, featured switching, and stream reassignment. Confirm the dashboard keeps stable slot names and reports frame/drop status. Pass with under 1% renderer frame drops and no material host-player frame-time regression. Repeat mixed FPS and 1080p configurations and verify warnings plus graceful degradation.
