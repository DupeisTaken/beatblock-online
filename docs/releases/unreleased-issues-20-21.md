# Unreleased broadcast fixes for issues #20 and #21

These changes are implemented for the release after `v0.3.0-beta.4`. They do
not change the current package version or publish a release.

## Correct OBS colors and independent renderer audio — #20

### What went wrong

The Player Stream used a custom texture/effect render path while LÖVE supplied
display-encoded RGBA bytes. OBS could decode those bytes as linear input without
a matching framebuffer conversion, shifting colors. Audio was also owned as a
private child of the video source. That coupled mixer lifetime and routing to a
scene item's video lifetime and left hidden renderer audio audible on the
desktop.

### Resolution

- Player Stream is video-only and declares an sRGB single-texture source.
- Frame-ring header v3 explicitly identifies RGBA8 display/sRGB bytes; unknown
  pixel contracts are rejected.
- Beatblock Online Audio is a separate A-D/Autoplay OBS source with independent
  0-2000 ms fine sync and status.
- Hidden renderer sessions are muted by exact child PID. Their original mute
  state is restored before teardown and retried after abnormal exit. Exact-PID
  discovery continues at a bounded rate for late-created sessions until the
  renderer exits. The host game and unrelated processes are never selected by
  executable name.
- Automatic isolation is enabled by default and has a Settings fallback for
  drivers whose process loopback follows session volume.
- Existing Player Stream scene items become video-only; broadcasters add the
  desired Audio sources once after upgrading.

## Optional song-plus-all-hits mix — #21

### What went wrong

Per-player reconstructed streams contain only the hits produced by that player.
There was no canonical feed containing the chart song plus every positive
scoring hitsound, and combining player channels duplicates the song.

### Resolution

- Broadcast can launch one optional audio-only Autoplay renderer before a race.
- It follows the featured renderer's delayed clock and uses Beatblock's native
  note behavior for perfect positive Block, Hold, Bounce, Side, tap, and
  ExtraTap decisions.
- Mines and mine-holds are avoided, and no custom miss, barely, or duplicate
  hitsound path is introduced.
- Autoplay has a dedicated APPDATA/Lovely profile, disables Beatblock save
  paths before changing options, and enables hitsounds plus nonzero music/SFX
  volume only in its disposable in-memory settings. Ordinary renderer profiles
  are unchanged.
- Autoplay allocates no video canvases or frame ring, but it is still one
  additional Beatblock simulation/audio process.
- Host and Commentator plans carry backward-compatible optional protocol-v3
  fields. Older peers can omit them.
- Enablement and clock-source changes are locked during countdown/gameplay and
  require an active featured renderer.

See [OBS setup and migration](../obs-setup.md) and the
[OBS 32.x physical trial](../trials/obs-32.1.2.md).

The OBS 32.1.2 hardware trial is still pending. In particular, compilation
cannot prove that a target system captures process loopback after
`ISimpleAudioVolume::SetMute(TRUE)`; the release must not claim that physical
compatibility gate until the trial records retained OBS audio, inaudible
renderer desktop output, color accuracy, and ten-minute drift/drop results.
