# Injected installer lifecycle trial

**Result:** PASS  
**Date:** 2026-07-15 (Windows x64)  
**Payload:** `release/BeatblockOnlineInstaller.exe` (`aa971c1dec7ecea3ec3ad65c62a0039ecf81630f751baa096436732c23a5efcb`)

The final packaged standalone-Lovely payload was installed into the disposable `.test/Beatblock` copy and launched as a normal game process. Lovely produced no console window. The hidden runtime was absent before Online, started after entering Online, exposed the token-protected local API, and received real Lua gameplay snapshots through the v2 named pipe.

The runtime working set was 18,329,600 bytes (17.48 MiB), below the 30 MiB idle target. The API reported `gameplay.updatedAtMs = 1784114026000`, which is direct evidence that telemetry reached the Rust runtime rather than merely proving that the API port opened.

Activating the Online hub's exit path returned Beatblock to its main menu. Beatblock remained alive and responsive; `BeatblockOnlineRuntime.exe` terminated and port 8974 stopped listening within the eight-second observation window.

The final installer-only header cleanup did not change the mod or runtime source. After the runtime rename, the self-contained installer was rebuilt with `BeatblockOnlineRuntime.exe`; transactional installer tests verify the new installed path, legacy-path cleanup, and rollback behavior. The installer was then opened one process at a time at Slint scale factors 1.0, 1.25, and 1.5 and closed after each capture. No installer process remained.

## Evidence

- Online hub with `RUNTIME READY`, `LOCAL API ACTIVE`, and runtime bytes received: [`injected-online-hub-ready.png`](injected-online-hub-ready.png)
- Main menu after Exit Online: [`injected-after-exit-online.png`](injected-after-exit-online.png)
- Installer at 100%, 125%, and 150%: [`installer-100-percent.png`](installer-100-percent.png), [`installer-125-percent.png`](installer-125-percent.png), [`installer-150-percent.png`](installer-150-percent.png)
- Machine-readable observations: [`injected-installer-lifecycle-latest.json`](injected-installer-lifecycle-latest.json)

This trial certifies the local installer payload, injected UI, lazy hidden-runtime lifecycle, IPC ingestion, local API, and explicit shutdown path. It does not certify the separately gated public-WAN, four-renderer endurance, native OBS 32.1.2 audio/video, NAT-PMP, resumable transfer, signed release, or clean Steam-distribution scenarios.
