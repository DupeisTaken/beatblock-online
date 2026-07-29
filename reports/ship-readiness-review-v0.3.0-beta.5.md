# v0.3.0-beta.5 ship-readiness review

Reviewed: 2026-07-29

Outcome: **shippable as a beta prerelease**. Code review and the complete
payload-backed acceptance suite found no remaining P0 or P1 defect in the
release scope.

## Release-blocking findings fixed

- A non-Windows OBS audio worker returned `()` because of a trailing semicolon;
  Linux compilation now returns the worker handle.
- Autoplay originally shared renderer profile state; it now uses a dedicated
  APPDATA/Lovely profile and disables Beatblock save paths before applying
  audio options.
- Installer promotion originally had unsafe recovery edges: temporary creation
  is exclusive, parent-process access failures fail closed, the previous
  managed installer remains until the new native window proves readiness, and
  elevated self-update is refused.
- ToolHelp process enumeration could report Beatblock/OBS absent after an
  incomplete snapshot; errors now disable mutations and propagate through
  backend transactions.
- Moving the active setlist row could leave an unplayed row behind the cursor
  and cause **NEXT CHART** to skip it. Only rows strictly after the active row
  can now reorder, with room and UI regressions.

## Automated evidence

| Gate                           | Result                                                                                                                                                 |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Protocol and release contracts | 13 protocol tests, 22 release/workflow tests, 3 patch-validator tests, six public-note/changelog pairs, and generated schema with no drift             |
| Full Rust release matrix       | 165 library, 3 installer, 2 runtime, 1 autoplay, 1 dashboard, 18 command, 1 Lua syntax, and 6 stress tests passed with real isolated fixtures/payloads |
| Static checks                  | Rustfmt, Prettier, default all-target Clippy, and installer-feature Clippy passed with warnings denied                                                 |
| Mod packaging                  | 19 Lovely signatures, 3 GameManager hooks, 29 commands, and both `0.3.0-beta.5` ZIPs passed                                                            |
| Deterministic UI               | 44 versioned 600×360 screenshots reproduced with zero differing pixels                                                                                 |
| Script installer               | Standalone and BeatblockPlus install/uninstall plus duplicate rejection passed                                                                         |
| Runtime lifecycle              | Ready on protocol 3; 26.1 MiB idle working set; mutex, explicit shutdown, and parent-exit cleanup passed                                               |
| Maximum room                   | 16 players and 32 spectators passed with authoritative rankings at about 131k score events/s                                                           |
| Pull requests/main             | Final OBS, dashboard, installer, and issue-form PR checks passed; combined dashboard-plus-installer main CI passed                                     |

The fresh companion benchmark passed every threshold:

- chart cache hit: 56.54 ms versus 399.41 ms cold, a 7.06× speedup;
- completed export publication p95: 9.42 ms, with enqueue p95 0.084 ms;
- buffered journal throughput: about 234k events/s;
- SQLite journal throughput: about 112k events/s;
- 5,000 events recovered from each journal path.

The complete `pnpm trial` report is
[`full-capability-latest.md`](trial-runs/full-capability-latest.md), with
machine-readable lifecycle, maximum-room, and benchmark results beside it.

## Publication gate

The beta tag must point to the release-preparation merge on `main`. The
tag-triggered GitHub Actions Release workflow must independently:

1. validate the exact tag/package version;
2. audit, test, format, lint, package, and build on `windows-2022`;
3. compile and run the produced installer's `--version` contract;
4. attest the installer, checksums, OBS DLL, and both mod ZIPs;
5. create a prerelease containing those exact assets.

The release is not considered published until that workflow succeeds and the
GitHub Release metadata, checksums, x64 installer, asset names, tag target, and
prerelease flag are verified.

## Beta limitations

Physical OBS 32.1.2 color accuracy, A-D/Autoplay routing, process-loopback
behavior after desktop mute, A/V alignment, and long-run drift remain in
[#28](https://github.com/DupeisTaken/beatblock-online/issues/28). Automated
coverage does not replace a disposable real-machine installer self-update,
antivirus/locked-destination trial, or live Beatblock/OBS close-and-refresh
trial on every supported host.

The release remains unsigned. SHA-256 checks and GitHub attestations establish
integrity and workflow provenance, but Windows may still show
**Unknown publisher**.
