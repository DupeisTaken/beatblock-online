# Installation and injection

## Recommended path

Run `BeatblockOnlineInstaller.exe`. Its four Unity Mod Manager-style tabs are installation-only:

- **Install:** a selected-target card, Automatic/standalone/BeatblockPlus method, optional OBS source, private/public firewall scope, explicit uncertified-build override, real operation progress, Install/Update, Repair, Uninstall, Restore Game Files, and postflight Launch Beatblock verification.
- **Components:** a colored table for the game build, adapter, shared Lua payload, Lovely, runtime, renderer, optional OBS plugin, and firewall. Text labels accompany every color, and one **Repair Required Components** action fixes managed files.
- **Log:** bounded log with Copy and Save.
- **Settings:** update channel/check and backup folder. Destructive data removal is shown in the Uninstall confirmation instead of being hidden in persistent settings.

The path in the field is always the path being described and modified. Selection priority is the current field, then the managed manifest target, then Steam discovery. Any folder with `Beatblock.exe`, the required LÖVE/Lua libraries, and the expected `packed` archives is structurally valid—including repository/reference copies and paths containing spaces or Unicode. The installer validates the supported `Beatblock.exe` SHA-256 `c91d0853feb12aceb66a821eb5cdffb9c25acf69268bb2cf7451fa42f864de6b`; other fingerprints require **Developer: allow an uncertified Beatblock build** and remain blocked from competitive rooms.

Only one game folder is managed at a time. Selecting another valid folder changes the primary action to **Move Installation** and asks before restoring the former target. Mod files remain in the normal shared `%APPDATA%\Beatblock\Mods` location; the game-folder-specific change is the Lovely `version.dll` injector.

Installation starts with the player's normal Windows permissions. If a protected game, OBS, or firewall operation returns access denied, requires elevation, or requests an administrator, the same installer opens the native Windows UAC prompt and continues in a windowless elevated helper. The visible installer follows the helper through an atomic status file and waits for its process exit plus postflight verification. It performs a final status read after process exit and retains failed status files for diagnosis, so UAC cancellation, invalid firewall arguments, and other privileged failures report the complete underlying error instead of only exit code 1. Build validation failures remain in the installer and do not trigger an unrelated UAC prompt.

The firewall rule is reconciled once per transaction. Its `program=` path is normalized to Windows backslashes, an absent previous rule is harmless, and the selected Private/Public profile is recorded for later Repair operations. If Repair stops at 76%, check for a UAC prompt on the secure desktop; accept it once and let the visible installer finish its postflight verification.

Every mutating action reports monotonic phases and a percentage. Controls are locked once replacement begins. The complete Lua adapter is first written to a sibling staging directory, checked against the Lovely module declarations and the required Online disconnect/timeout recovery contracts, then atomically swapped into place. The manifest is written only after required component hashes pass. Existing injector backups are never replaced during update or repair.

The elevated helper treats its manifest and status-file arguments as untrusted. Status updates are confined to UUID-named JSON files in the managed operations directory, and persisted Mods, backup, runtime, maintenance-installer, hash, and OBS paths must match installer-owned locations before any privileged copy or removal. A stale game folder is never used for game-file deletion unless it still has Beatblock's required executable/library shape.

Installed layout:

```text
<Beatblock>\Beatblock.exe
<Beatblock>\version.dll
<Beatblock>\steam_appid.txt        (isolated/non-Steam copies only)
%APPDATA%\Beatblock\Mods\BeatblockOnline\
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
%LOCALAPPDATA%\BeatblockOnline\...
  runtime\BeatblockOnlineRuntime.exe
  installer\BeatblockOnlineInstaller.exe
```

Lovely loads beside `Beatblock.exe` and applies signatures while LÖVE loads Lua chunks. BBT does not rewrite `Beatblock.exe` or packed archives. The standalone bootstrap uses Lovely's supported `{{lovely_hack:patch_dir}}` placeholder. Opening Online starts a literal-source LÖVE worker thread, passes the installed mod path through a channel, launches the GUI-subsystem runtime without a console, and connects to `\\.\pipe\beatblock-online-v3`. Bundled LuaSocket on `127.0.0.1:8975` remains the isolated fallback when LuaJIT FFI is unavailable. Normal menus never create this thread.

The worker's launch, error, and reconnect status envelopes use protocol v3,
matching the native runtime. Host and join forms validate their required text,
password, and UDP port fields before sending a control request; runtime
rejections remain visible even while the local runtime is connected.

Upgrades transactionally quarantine and remove the retired, installer-marked
`Mods\BeatblockTogether` package. Leaving both equal-priority Lovely
bootstraps installed can start the stale runtime from the former data
directory even when the current component checks are green. A same-named
folder without the legacy installer's exact marker and expected payload shape
is preserved.

The bundled injector is built from Lovely v0.9 with the default console disabled; `--enable-console` is retained for developer diagnosis. The maintained delta is recorded in `third-party/lovely-no-console.patch`. An existing compatible Lovely PE is backed up and preserved, including builds already approved by Windows Application Control. An absent, damaged, or previously installer-owned injector is installed or repaired from the bundled payload. All replacements remain reversible through the same installer transaction.

For a selected non-Steam game copy, the installer also owns direct-launch support: it writes `steam_appid.txt` with Beatblock app ID `3045200`. A pre-existing different file is backed up first. Move, Restore Game Files, and Uninstall restore that backup or remove the installer-owned marker. The installer never adds this override to a detected Steam library.

Installing the optional OBS source transactionally removes the two known files from the retired `beatblock-together-obs` module after the renamed source has passed hash verification. If the containing install fails, those legacy files are restored; after success OBS loads only the current `beatblock-online-obs` module.
OBS must be closed before this action because Windows locks loaded plugin DLLs even for an administrator. While `obs64.exe` is running, the GUI visibly disables and clears only the optional OBS checkbox so the core install can still complete; the execution path repeats the check to catch races without opening a UAC prompt that cannot resolve the lock.

Repair restores only BBT-owned files. Uninstall restores the backed-up injector or removes a BBT-owned injector only when no other Lovely mod depends on it. Settings/history are preserved unless **Remove settings and match history** is checked.

## Launch verification

After Install/Update or Repair, **Launch Beatblock** starts the executable inside the selected folder with that folder as its working directory. The installed app-id marker means the same non-Steam copy can subsequently be opened by double-clicking `Beatblock.exe`; PowerShell and `LOVELY_MOD_DIR` are not required. Verification waits for the exact process to remain alive and for a new Lovely log under `%APPDATA%\Beatblock\Mods\lovely\log` to identify the selected game directory and report `Initialization complete`. A Lovely panic or early process exit is shown with the relevant log excerpt and repair/log actions.

## Repairing the missing-dashboard alpha

An earlier installer declared `bbt/dashboard_model.lua` in `lovely/hooks.toml` but omitted that source from both installed and renderer payloads. Lovely correctly aborted with `Module source "bbt/dashboard_model.lua" not found in preloaded sources`. The Components table now marks such installations **REPAIR REQUIRED**. Repair installs the missing file from the centralized payload inventory while preserving the existing Lovely backup; conformance tests prevent a declared Lovely module from being omitted again.

## Developer/test path

The two mutually exclusive development ZIPs are under `mod/releases`. Release users should use the one-EXE installer. For a disposable trial, clone the game folder, select that clone in the installer, and enable the unknown-build override only when its fingerprint is not certified. The selected-target card must name the clone before proceeding.

If Online reports damage, choose **Open Installer**, inspect the Components table, then use **Repair Required Components**. Steam Verify restores official files. Do not manually delete `version.dll` when another Lovely mod uses it.

If Beatblock exits immediately without creating a Lovely log, inspect
`Microsoft-Windows-CodeIntegrity/Operational` for events 3033 or 3077 naming
the game folder's `version.dll`. Those events mean Windows rejected the
injector before Lovely could report an error; use a signed or previously
approved compatible Lovely build rather than repeatedly repairing the game.

The latest physical injected-game evidence is recorded in [`injected-installer-lifecycle-latest.md`](../reports/trial-runs/injected-installer-lifecycle-latest.md), including the hidden runtime, live telemetry ingestion, and explicit Online shutdown screenshots.

The current isolated `.test\Beatblock` installer, elevation diagnosis, normalized firewall command, helper-error trial, and Lovely recovery evidence are recorded in [`installer-reliability-latest.md`](../reports/trial-runs/installer-reliability-latest.md).

The expanded release-EXE failure matrix and full isolated Rust transaction round trips are recorded in [`installer-acceptance-latest.md`](../reports/trial-runs/installer-acceptance-latest.md). Its machine-readable companion is [`installer-acceptance-latest.json`](../reports/trial-runs/installer-acceptance-latest.json). The report distinguishes automated and non-elevated passes from physical gates that still require an accepted UAC prompt.
