# Injecting Beatblock Together into Beatblock

Beatblock Together supports two mutually exclusive Windows injection paths. Use **standalone Lovely** when BeatblockPlus is not installed. Use the **BeatblockPlus 2.x package** when it is.

Both paths use [Lovely Injector](https://github.com/ethangreen-dev/lovely-injector), which loads `version.dll` beside `Beatblock.exe` and patches Lua chunks as the game loads them. It does not rewrite `Beatblock.exe` or the packed game archives. Lovely discovers each mod in its own folder under `%APPDATA%\Beatblock\Mods`; its default folder is derived from the `Beatblock.exe` filename. BeatblockPlus then discovers `mod.json` files in those same per-mod folders.

## Supported baseline

- Windows x64.
- The Beatblock build recorded in `mod/fixtures/patch-signatures.json`.
- Lovely 0.9.0 or a later compatible release with LÖVE 12 support.
- BeatblockPlus 2.x for the BeatblockPlus distribution.

The pinned `Beatblock.exe` SHA-256 is:

```text
c91d0853feb12aceb66a821eb5cdffb9c25acf69268bb2cf7451fa42f864de6b
```

An unknown build may be used for non-competitive hook development, but the alpha intentionally blocks it from competitive races.

## Find the game folder

In Steam, open **Library > Beatblock > Manage > Browse local files**. The folder you need contains all three of these files:

```text
Beatblock.exe
love.dll
lua51.dll
```

The normal mod-data folder is separate:

```text
%APPDATA%\Beatblock\Mods
```

In PowerShell, `$env:APPDATA` expands `%APPDATA%` to the current Windows account's roaming application-data folder.

## Pathway 1: guarded repository installer

This is the least error-prone path from a source checkout. The installer verifies the game hash, locates or installs Lovely, enforces the correct distribution for the detected loader, and refuses to overwrite an existing installation unless `-Force` is explicit.

First build and validate both packages:

```powershell
pnpm test:mod
powershell -NoProfile -File scripts/test-install-mod.ps1
```

For standalone Lovely, download `lovely-x86_64-pc-windows-msvc.zip` from the [official Lovely release page](https://github.com/ethangreen-dev/lovely-injector/releases/latest), then run:

```powershell
.\scripts\install-mod.ps1 `
  -GameDir "C:\Program Files (x86)\Steam\steamapps\common\Beatblock" `
  -Distribution standalone `
  -LovelyArchive "$HOME\Downloads\lovely-x86_64-pc-windows-msvc.zip"
```

If Lovely and BeatblockPlus 2.x are already installed:

```powershell
.\scripts\install-mod.ps1 `
  -GameDir "C:\Program Files (x86)\Steam\steamapps\common\Beatblock" `
  -Distribution beatblock-plus
```

Use `-WhatIf` to preview changes. `-Force` moves the prior installation to `%APPDATA%\Beatblock\BeatblockTogether-backups` before installing. `-AllowUnknownBuild` is only for local hook development and does not make that build competition-compatible.

To uninstall only Beatblock Together:

```powershell
.\scripts\install-mod.ps1 `
  -GameDir "C:\Program Files (x86)\Steam\steamapps\common\Beatblock" `
  -Uninstall
```

The uninstaller deliberately keeps `version.dll`, because Lovely or other mods may still depend on it.

## Pathway 2: standalone Lovely, manual

1. Close Beatblock.
2. Download the Windows x64 Lovely ZIP from the [official releases](https://github.com/ethangreen-dev/lovely-injector/releases/latest).
3. Copy `version.dll` from that ZIP into the game folder, directly beside `Beatblock.exe`.
4. Create `%APPDATA%\Beatblock\Mods` if it does not exist.
5. Extract `mod/releases/beatblock-together-standalone-0.1.0-alpha.1.zip` directly into that `Mods` folder.
6. Start the companion, then launch Beatblock through Steam.

The final paths must be exactly:

```text
<Beatblock game folder>\
├── Beatblock.exe
├── love.dll
├── lua51.dll
└── version.dll                    <- Lovely's Windows proxy

%APPDATA%\Beatblock\Mods\
└── BeatblockTogether\
    ├── README.txt
    ├── bbt\
    │   ├── core.lua
    │   ├── ipc_thread.lua
    │   └── online_state.lua
    └── lovely\
        ├── bootstrap.toml
        └── hooks.toml
```

Do not leave an extra directory layer such as `Mods\beatblock-together-standalone-0.1.0-alpha.1\BeatblockTogether`. Lovely requires the folder containing `lovely\*.toml` to be the direct child mod folder.

Do not use the standalone package if BeatblockPlus is present. BeatblockPlus applies its own bootstrap before loading Beatblock Together's `mod.json` package.

## Pathway 3: BeatblockPlus 2.x

Install Lovely and [BeatblockPlus](https://github.com/BeatblockTools/BeatblockPlus) first. Confirm Beatblock's main menu contains **Mods**.

The preferred BeatblockPlus path uses its in-game ZIP installer:

1. Leave `beatblock-together-beatblock-plus-0.1.0-alpha.1.zip` unopened.
2. Launch Beatblock and open **Mods**.
3. Drag the ZIP file onto the Mods screen.
4. Accept the detected `Beatblock Together` mod and restart Beatblock when prompted.

The release ZIP intentionally contains `BeatblockTogether/mod.json`; this is the layout BeatblockPlus's drag-and-drop installer requires.

For a manual install, close the game and extract the ZIP directly into `%APPDATA%\Beatblock\Mods`. Verify:

```text
%APPDATA%\Beatblock\Mods\
├── BeatblockPlus\                 <- folder name may vary
│   ├── mod.json                   <- id is beatblock-plus
│   └── lovely\...
└── BeatblockTogether\
    ├── mod.json                   <- id is beatblock-together
    ├── main.lua
    ├── config.lua
    ├── bbt\...
    ├── lovely\hooks.toml
    └── states\Online.lua
```

Do not also extract the standalone release. The two packages share a core but use different bootstraps.

## Developer injection without touching normal mods

Lovely supports an explicit `LOVELY_MOD_DIR`. Use it to isolate a development copy from `%APPDATA%\Beatblock\Mods`:

```powershell
$game = "C:\Program Files (x86)\Steam\steamapps\common\Beatblock"
$devMods = Join-Path $PWD ".dev-mods\standalone"

pnpm test:mod
.\scripts\install-mod.ps1 `
  -GameDir $game `
  -ModsDir $devMods `
  -Distribution standalone

$env:LOVELY_MOD_DIR = $devMods
& (Join-Path $game "Beatblock.exe")
Remove-Item Env:LOVELY_MOD_DIR
```

Only the selected development directory is scanned during that launch. For an isolated BeatblockPlus run, copy the installed BeatblockPlus folder into `$devMods` first, then install with `-Distribution beatblock-plus`.

Regenerate the copied Lua core after changes with `pnpm package:mods`, or rerun the installer with `-Force`. The game must be restarted for Lovely patch changes; Lua patches are applied while chunks load.

## Confirm injection succeeded

1. A Lovely console opens with the game unless Lovely was launched with `--disable-console`.
2. Lovely logs appear under `%APPDATA%\Beatblock\Mods\lovely\log`, or under `<LOVELY_MOD_DIR>\lovely\log` for an isolated run.
3. Beatblock's main menu contains **Online**.
4. The Online screen reports the companion connection and offers create/join lobby actions.
5. A practice run updates `http://127.0.0.1:8974/v1/state` when opened with the local token generated by the companion.

For a pre-release check, inspect Lovely's patched sources under `Mods\lovely\dump` and confirm the BBT hook payloads appear in `states/Game.lua`, `states/SongSelect.lua`, `states/Results.lua`, and the main-menu state.

## Recovery and removal

- Launch once with `--disable-mods` if a patch prevents the menu from loading.
- Remove `%APPDATA%\Beatblock\Mods\BeatblockTogether` to uninstall BBT manually.
- Keep `version.dll` when any Lovely mod remains. Delete only that Lovely-provided `version.dll` to disable Lovely completely.
- If both BBT variants were mixed, remove the entire `BeatblockTogether` folder and reinstall exactly one release.
- If Lovely reports a missing patch signature, stop using the build competitively and run `pnpm validate:patches` against the updated game reference before changing the fixture.
- Steam's **Verify integrity of game files** restores official game files, but BBT normally never modifies them.

The installer never copies remote credentials or invite codes into the mod folder. Those remain in the companion's credential store.
