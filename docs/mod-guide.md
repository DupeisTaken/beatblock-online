# Beatblock mod installation and player flow

Beatblock Together is controlled from inside Beatblock. The companion webpage is only used once for account setup and later for OBS or caster links.

## Install

The complete paths, folder trees, automatic installer, developer override, verification, and recovery steps are in [Injecting Beatblock Together into Beatblock](injection.md).

The short version is:

1. Find the folder containing `Beatblock.exe` through Steam's **Manage > Browse local files**.
2. Install Lovely's `version.dll` beside `Beatblock.exe`.
3. Choose exactly one ZIP from `mod/releases`:
   - `beatblock-together-standalone-0.1.0-alpha.1.zip` for Lovely without BeatblockPlus.
   - `beatblock-together-beatblock-plus-0.1.0-alpha.1.zip` when using BeatblockPlus 2.x.
4. Extract the selected ZIP into `%APPDATA%\Beatblock\Mods`, producing `%APPDATA%\Beatblock\Mods\BeatblockTogether`. BeatblockPlus users can instead drag the unopened BeatblockPlus ZIP onto its in-game **Mods** screen.
5. Install and start the Windows companion.
6. Open the companion tray console, enter the self-hosted instance URL, invite code, and display name.
7. Start Beatblock and select **Online** from the main menu.

The alpha rejects unknown Beatblock builds. The pinned executable SHA-256 is recorded in `mod/fixtures/patch-signatures.json`.

## Online screen controls

The Online screen is a mouse-, keyboard-, and controller-ready race-control dashboard:

- Hover and click any action, or navigate with the arrow keys/D-pad and select with Enter, Space, Z, or controller A.
- Disabled actions remain selectable and explain exactly what prerequisite is missing.
- Lobby codes accept direct keyboard entry, Backspace, and Enter, as well as the controller-friendly character grid.
- Player and spectator joins are separate actions.
- **Practice + Telemetry** opens Custom Levels even without a remote instance, keeping local companion and OBS output useful while offline.
- The lobby view shows chart verification, lifecycle, synchronization delay, spectators, and a two-column 16-player readiness roster.

The interface uses a high-contrast ink/cyan/mint scheme. Mint indicates verified or ready state, amber indicates an unmet prerequisite, and coral indicates a connection failure or destructive action.

## Play a race

### Organizer

1. Select **Create private lobby** in Beatblock.
2. Share the displayed six-character lobby code.
3. Select **Select custom chart**. Beatblock opens its normal Custom Levels song wheel.
4. Select the exact variant to race. The mod calculates Beatblock's expected maximum hit count, and the companion hashes the complete package before locking it.
5. Locate the chart locally if prompted, then ready up.
6. When every competitor is ready, select **Start synchronized race**.

### Player

1. Select **Join with code** and enter the code using the controller-friendly on-screen keyboard.
2. Select **Locate matching chart** and choose the announced chart and variant through Beatblock's song wheel.
3. A hash mismatch leaves readiness disabled and displays a specific error.
4. Select **Ready for race**.

The mod loads the verified chart several seconds before the scheduled start. Beatblock remains in its native `startPending` state until the synchronized timestamp, displays the countdown, and then releases gameplay. During play, the upper-right race HUD shows live server rank, accuracy, and combo. Normal Beatblock results return to the online lobby; retry and pause invalidate competitive runs.

## What remains in the webpage

- Invite redemption and instance configuration.
- Companion connection diagnostics.
- OBS player-card, leaderboard, versus, and caster URLs.
- Authenticated spectator handoff.

Creating, joining, selecting a chart, readying, and starting are ordinary in-game operations and do not require an open browser.

## Troubleshooting

- **Companion offline:** start the tray companion before Beatblock and confirm port 8975 is not occupied.
- **Instance offline:** use the tray console to reconnect the invited account. Production instances require HTTPS/WSS.
- **Chart notes were not preloaded:** remain on the song in Song Select until its preview finishes loading, then select it again.
- **Chart mismatch:** both the package bytes and selected variant must match. The mod does not automatically download charts.
- **Both distributions installed:** remove one package and restart Beatblock.
- **Unknown build:** do not bypass the check for a competitive race; update the patch fixture and validate every hook first.

Run `pnpm test:mod` and `powershell -NoProfile -File scripts/test-install-mod.ps1` before distributing either ZIP. They regenerate both packages, verify every Lovely signature against the supplied Beatblock source archive, check the in-game command contract, inspect both release archives, and exercise both installer paths.
