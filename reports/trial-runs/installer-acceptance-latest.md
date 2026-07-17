# Installer acceptance matrix

**Date:** 2026-07-16
**Branch:** `codex/installer`
**Installer:** `release/BeatblockTogetherInstaller.exe`
**SHA-256:** `7213112e60f9f171c7d986c1c3e808096feac31b398d2448413ea30014c32903`
**Result:** Automated and non-elevated physical gates pass. Current-build elevated success, protected-folder repair, and destructive physical uninstall remain blocked on an unanswered Windows UAC prompt.

## Bugs found by the expanded trial

1. Moving BBT to another game folder left the BBT-owned `version.dll` in the old folder. The active `.BeatblockTogether.rollback-*` transaction directory was mistaken for another Lovely mod. Transaction directories are now excluded from external-mod ownership checks, with a full move regression test.
2. A firewall/elevation failure rolled back the mod, runtime, and injector but left the maintenance installer copy behind. The maintenance executable now uses the same rollback guard as the other managed files.
3. A failed atomic replacement could strand a UUID-suffixed temporary file when a directory occupied a managed file path. Failed writes now remove their temporary payload.
4. Manually selecting Standalone while BeatblockPlus is installed could load both BBT adapters. The installer now rejects that combination, while Automatic and Repair re-detect BeatblockPlus and select exactly one adapter.

## Automated transaction coverage

The Rust suite now executes the actual staging, hashing, atomic directory swap, injector backup, maintenance payload, renderer profile, manifest, move, repair, restore, OBS payload, and uninstall paths inside isolated directories. Only firewall and uninstall-registry mutations are disabled in these isolated tests; production callers always enable them.

| Scenario                                  | Result | Evidence                                                         |
| ----------------------------------------- | ------ | ---------------------------------------------------------------- |
| Supported reference shape and fingerprint | Pass   | Pinned `Beatblock.exe` SHA-256 matches                           |
| Arbitrary Unicode/spaced folder           | Pass   | Valid structure, uncertified policy applied                      |
| Unknown build without override            | Pass   | Rejected before mutation                                         |
| Standalone fresh install                  | Pass   | Every payload and manifest hash verified                         |
| Existing Lovely install                   | Pass   | Original bytes backed up once and preserved through repair       |
| Corrupt Lua/runtime/injector repair       | Pass   | All three restored; original Lovely backup unchanged             |
| Restore game files                        | Pass   | Mod removed and original injector restored                       |
| Uninstall, preserve user data             | Pass   | Runtime/mod/OBS removed; history retained                        |
| Uninstall, remove user data               | Pass   | Retention files removed                                          |
| Move between game folders                 | Pass   | Old BBT-owned injector removed, new target activated             |
| BeatblockPlus automatic adapter           | Pass   | Plus payload installed without standalone bootstrap              |
| BeatblockPlus missing                     | Pass   | Explicit Plus selection rejected                                 |
| Standalone/BeatblockPlus conflict         | Pass   | Explicit Standalone selection rejected                           |
| Mid-transaction Lovely failure            | Pass   | Previous mod/runtime restored; no manifest or temp payload left  |
| OBS payload and marker                    | Pass   | DLL exports, layout, file hashes, locale, and uninstall verified |
| Progress contract                         | Pass   | Monotonic progress and exactly one terminal event                |

Automated totals:

- 34 Rust unit/transaction tests
- 1 adaptive dashboard test
- 5 protocol-v2/game-command tests
- 1 distributed Lua compilation test
- 4 release stress tests
- 4 TypeScript scoring/protocol tests
- 13 Lovely signature fixtures, 3 GameManager hooks, 18 in-game commands, and both mod ZIPs

## Physical release-EXE failure matrix

The self-contained release helper was run against disposable targets under `.test/installer-acceptance`. Every failure returned exit code 1, wrote one terminal error record, and left no active mod, injector, runtime, maintenance copy, or manifest.

| Scenario                                 | Exact terminal diagnosis                | Rollback |
| ---------------------------------------- | --------------------------------------- | -------- |
| Missing folder                           | `does not contain Beatblock.exe`        | Clean    |
| Uncertified executable                   | `this Beatblock build is not certified` | Clean    |
| BeatblockPlus selected but absent        | `BeatblockPlus 2.x was not detected`    | Clean    |
| Standalone selected with BeatblockPlus   | `avoid loading both BBT adapters`       | Clean    |
| Unknown-build override without elevation | Windows Firewall requires administrator | Clean    |
| Supported build without elevation        | Windows Firewall requires administrator | Clean    |

The complete six-case matrix was rerun against the current hash after moving the workspace to `E:\beatblock-online`; every case returned its expected terminal diagnostic and left no managed files or transaction directories behind.

The visible installer was also inspected on the managed `.reference\Beatblock` target. The selected target, supported build, standalone method, Repair Required state, colored Components table, verified OBS row, and Repair action were visible and readable at the current Windows scale.

## Physical gates requiring user interaction

The current build reached its native UAC repair handoff, but the secure-desktop prompt was not approved. The test process was terminated afterward and the existing `.reference\Beatblock` manifest, injector, and mod were confirmed present. No installer, helper, game, runtime, Cargo, or OBS process was left running.

These gates must not be reported as current-build passes until one UAC prompt is accepted:

- elevated Repair completion and progress handoff;
- protected-folder install/update;
- physical Move Installation followed by restoration to `.reference\Beatblock`;
- physical Restore and Uninstall round trips;
- post-repair Launch Beatblock verification;
- 125% and 150% installer screenshot review.

The current OBS module (`f8ed58d62e5888234cf9877a0d62f9f8a9fc592d8ccb165a30237251ff1bd063`) passed a direct Windows `LoadLibrary` smoke test with `obs_module_load`, `obs_module_ver`, and `obs_module_set_pointer` resolved. The preceding physical OBS 32.0.4 Add Source trial used the earlier `dd5052508268fef635a45ee66ef6326c3b5feb29c98aca32421e6c74d9bdbbcf` artifact, so a new physical OBS UI verification remains required for the current DLL.
