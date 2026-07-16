# Installation and injection

## Recommended path

Run `BeatblockTogetherInstaller.exe`. Its four Unity Mod Manager-style tabs are installation-only:

- **Install:** a selected-target card, Automatic/standalone/BeatblockPlus method, optional OBS source, real operation progress, Install/Update, Repair, Uninstall, Restore Game Files, and postflight Launch Beatblock verification.
- **Components:** a colored table for the game build, adapter, shared Lua payload, Lovely, runtime, renderer, optional OBS plugin, and firewall. Text labels accompany every color, and one **Repair Required Components** action fixes managed files.
- **Log:** bounded log with Copy and Save.
- **Settings:** update channel/check, backup folder, private/public firewall profile, data-retaining uninstall, and developer-only unknown-build override.

The path in the field is always the path being described and modified. Selection priority is the current field, then the managed manifest target, then Steam discovery. Any folder with `Beatblock.exe`, the required LÖVE/Lua libraries, and the expected `packed` archives is structurally valid—including repository/reference copies and paths containing spaces or Unicode. The installer validates the supported `Beatblock.exe` SHA-256 `c91d0853feb12aceb66a821eb5cdffb9c25acf69268bb2cf7451fa42f864de6b`; other fingerprints require **Developer: allow an uncertified Beatblock build** and remain blocked from competitive rooms.

Only one game folder is managed at a time. Selecting another valid folder changes the primary action to **Move Installation** and asks before restoring the former target. Mod files remain in the normal shared `%APPDATA%\Beatblock\Mods` location; the game-folder-specific change is the Lovely `version.dll` injector.

Installation starts with the player's normal Windows permissions. If a protected game, OBS, or firewall location returns an access-denied error, the same installer requests administrator approval through the native Windows UAC prompt and continues in a windowless elevated helper. The visible installer follows the helper through an atomic status file and waits for its process exit plus postflight verification. UAC cancellation and helper failures are explicit results. Build validation failures remain in the installer and do not trigger an unrelated UAC prompt.

Every mutating action reports monotonic phases and a percentage. Controls are locked once replacement begins. The complete Lua adapter is first written to a sibling staging directory, checked against the Lovely module declarations, then atomically swapped into place. The manifest is written only after required component hashes pass. Existing injector backups are never replaced during update or repair.

Installed layout:

```text
<Beatblock>\Beatblock.exe
<Beatblock>\version.dll
%APPDATA%\Beatblock\Mods\BeatblockTogether\
  runtime-path.txt
  installer-path.txt
  bbt\core.lua
  bbt\dashboard_model.lua
  bbt\ipc_thread.lua
  bbt\online_state.lua
  bbt\renderer.lua
  lovely\hooks.toml
  lovely\bootstrap.toml       (standalone only)
  mod.json + main.lua ...     (BeatblockPlus only)
%LOCALAPPDATA%\BeatblockTogether\...
  runtime\BeatblockTogetherRuntime.exe
  installer\BeatblockTogetherInstaller.exe
```

Lovely loads beside `Beatblock.exe` and applies signatures while LÖVE loads Lua chunks. BBT does not rewrite `Beatblock.exe` or packed archives. The standalone bootstrap uses Lovely's supported `{{lovely_hack:patch_dir}}` placeholder. Opening Online starts a literal-source LÖVE worker thread, passes the installed mod path through a channel, launches the GUI-subsystem runtime without a console, and connects to `\\.\pipe\beatblock-together-v2`. Bundled LuaSocket on `127.0.0.1:8975` remains the isolated fallback when LuaJIT FFI is unavailable. Normal menus never create this thread.

The bundled injector is built reproducibly from Lovely v0.9 with the default console disabled; `--enable-console` is retained for developer diagnosis. The maintained delta is recorded in `third-party/lovely-no-console.patch`. Existing Lovely installations are backed up and restored according to the installer transaction rather than overwritten blindly.

Repair restores only BBT-owned files. Uninstall restores the backed-up injector or removes a BBT-owned injector only when no other Lovely mod depends on it. Settings/history are preserved unless **Remove settings and match history** is checked.

## Launch verification

After Install/Update or Repair, **Launch Beatblock** starts the executable inside the selected folder with that folder as its working directory. For a non-Steam copy, the installer temporarily supplies Steam app ID `3045200` and restores the prior file afterward. Verification waits for the exact process to remain alive and for a new Lovely log under `%APPDATA%\Beatblock\Mods\lovely\log` to identify the selected game directory and report `Initialization complete`. A Lovely panic or early process exit is shown with the relevant log excerpt and repair/log actions.

## Repairing the missing-dashboard alpha

An earlier installer declared `bbt/dashboard_model.lua` in `lovely/hooks.toml` but omitted that source from both installed and renderer payloads. Lovely correctly aborted with `Module source "bbt/dashboard_model.lua" not found in preloaded sources`. The Components table now marks such installations **REPAIR REQUIRED**. Repair installs the missing file from the centralized payload inventory while preserving the existing Lovely backup; conformance tests prevent a declared Lovely module from being omitted again.

## Developer/test path

The two mutually exclusive development ZIPs are under `mod/releases`. Release users should use the one-EXE installer. For a disposable trial, clone the game folder, select that clone in the installer, and enable the unknown-build override only when its fingerprint is not certified. The selected-target card must name the clone before proceeding.

If Online reports damage, choose **Open Installer**, inspect the Components table, then use **Repair Required Components**. Steam Verify restores official files. Do not manually delete `version.dll` when another Lovely mod uses it.

The latest physical injected-game evidence is recorded in [`injected-installer-lifecycle-latest.md`](../reports/trial-runs/injected-installer-lifecycle-latest.md), including the hidden runtime, live telemetry ingestion, and explicit Online shutdown screenshots.

The current `.reference\Beatblock` installer and Lovely recovery evidence is recorded in [`installer-reliability-latest.md`](../reports/trial-runs/installer-reliability-latest.md). It distinguishes the verified payload/launch result from the administrator-approved firewall gate that remains pending after UAC cancellation.
