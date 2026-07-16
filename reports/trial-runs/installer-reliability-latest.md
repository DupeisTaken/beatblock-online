# Installer reliability and Lovely recovery trial

**Result:** PASS for payload recovery and launch; Windows Firewall approval remains pending  
**Date:** 2026-07-16 (Windows x64)  
**Selected target:** `E:\beatblock-together\.reference\Beatblock`  
**Installer SHA-256:** `cd8902db6376c503a6e1d90c114f19f8fdf5740c4aee2b009927d27297b7c322`

## Observed workflow

1. The selected-target card described the `.reference\Beatblock` field rather than the separate Steam installation. It identified the certified fingerprint and standalone Lovely adapter.
2. The Components table identified the actual legacy failure: **Shared Lua payload — Broken**, while Lovely showed an explicit legacy-backup warning.
3. Repair displayed concrete monotonic phases and a determinate progress bar through the program-scoped firewall step. Cancelling the administrator handoff produced one red result banner and one failure dialog; the transaction guards restored the prior payload.
4. The reviewed recovery payload added `bbt/dashboard_model.lua` to the player and renderer inventories and synchronized the embedded runtime. The table then reported the shared payload and renderer as installed. The firewall rule remains **Missing** because the administrator request was cancelled; the next Install/Update or Repair will request approval again.
5. **Launch Beatblock** started `E:\beatblock-together\.reference\Beatblock\Beatblock.exe`. Lovely wrote a new log identifying that exact directory and reported `Initialization complete in 9ms`.
6. Beatblock remained alive, the Online entry opened, the hidden runtime reached **LINK READY**, and the local API/OBS export state reached **ACTIVE**. Exit Online terminated the runtime; Beatblock and the installer were then closed.
7. No matching Beatblock, `version.dll`, or Lovely error appeared in the Windows Application event log during the launch window.

## Crash recovery evidence

The prior crash was a Lovely panic caused by a declared source absent from the installed preloaded sources:

```text
Module source "bbt/dashboard_model.lua" not found in preloaded sources
```

The current payload inventory includes that module in both player and renderer profiles. Its SHA-256 is `77b58a7104423ccff6d636c9777cf2f9dc8193b74457621f7b1a7a6d8744175b`.

The new Lovely log contained:

```text
Lovely 0.9.0
Game directory is at "E:\\beatblock-together\\.reference\\Beatblock"
Initialization complete in 9ms
```

## Automated gates

- 21 Rust installer/runtime unit tests passed, including arbitrary Unicode targets, move-installation detection, missing-dashboard migration, UTF-8 BOM manifests, payload conformance, monotonic progress, and file/directory rollback.
- Four release stress tests passed for 16 players/32 spectators, chart hashing/cache invalidation, saturated journals, and 32 broadcast/export consumers.
- Four protocol-v2 score/conformance tests passed.
- Both mod distributions packaged; 13 Lovely signatures, three GameManager hooks, 18 in-game commands, and both ZIP payloads validated.
- No Beatblock, runtime, installer, Cargo, or Rust compiler test process remained after verification.

## Remaining physical gate

The UAC cancellation path is certified. A successful administrator-approved firewall creation was not completed in this run because the prompt was cancelled. This does not affect the repaired Lovely launch, local Online runtime, or joining another host, but Windows hosting may require approving the next installer repair or creating the inbound rule manually.
