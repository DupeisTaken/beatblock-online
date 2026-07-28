# v0.4.0-alpha.1 ship-readiness review

Reviewed: 2026-07-28

Outcome: **shippable as an alpha prerelease**. No open P0 or P1 issue remains
from the code review and automated validation described below.

## Release-blocking findings fixed

- Strict-room reconnects now retain the participant's normalized Beatblock
  build identity. A real QUIC regression test reconnects to a same-build host.
- Chart-transfer eligibility is rechecked after waiting for the archive worker
  and after packaging, preventing a queued request from surviving a role,
  admission, verification, lock, or transfer-policy change.
- Fallback game-content identity uses a 64 KiB streaming buffer on a blocking
  worker instead of reading complete game archives on the asynchronous runtime.
- SDL3's boolean minimize result is handled as a Lua boolean, allowing the
  existing LÖVE and Windows fallbacks to run after a failed SDL minimize.
- Fixture-backed tests accept explicit isolated game/UI paths, so Git worktrees
  do not require proprietary archives to be copied into each checkout.
- The lifecycle fixture sends the displayed Beatblock build token used by the
  real Lua client and can reuse the runtime built by the aggregate trial.

## Automated evidence

| Gate                 | Result                                                                                                    |
| -------------------- | --------------------------------------------------------------------------------------------------------- |
| Protocol v3          | 13 tests passed; schema regenerated without drift                                                         |
| Rust release matrix  | 148 library, 2 installer, 2 runtime, 1 dashboard, 18 command, 1 Lua syntax, and 6 stress tests passed     |
| Strict static checks | Rustfmt, Prettier, default all-target Clippy, and installer-feature Clippy passed with warnings denied    |
| Lovely/mod packaging | 19 live Lovely signatures, 3 GameManager hooks, 29 in-game commands, and both `0.4.0-alpha.1` ZIPs passed |
| Deterministic UI     | 31 screenshots matched the versioned baselines at 0 differing pixels                                      |
| Script installer     | Standalone and BeatblockPlus install/uninstall plus duplicate rejection passed                            |
| Runtime lifecycle    | Ready on protocol 3; 12.5 MiB idle working set; mutex, explicit shutdown, and parent-exit cleanup passed  |
| Maximum room         | 16 players and 32 spectators passed with authoritative rankings and about 310k score events/s             |
| Release contracts    | 18 workflow/version/provenance tests and 3 patch-validator contract tests passed                          |

The fresh runtime benchmark passed every threshold:

- chart cache hit: 19.67 ms versus 149.95 ms cold, a 7.62× speedup;
- completed export publication p95: 2.07 ms;
- buffered journal throughput: about 989k events/s;
- SQLite journal throughput: about 347k events/s;
- 5,000 events recovered from each journal path.

Local Windows security scanning intermittently locked newly linked Rust
executables during the aggregate runner. The identical release target matrix
passed with one build job, and the remaining gates were then executed directly
against those artifacts. This was an artifact-finalization issue, not a failed
product assertion. The tag-triggered GitHub Actions run remains the independent
clean-runner build and publication authority.

## Alpha limitation

The OBS selector and private Application Audio Capture child are covered by
source/target tests and the native plugin build gate. Physical OBS mixer
routing cannot be proven by a headless automated run. Before an event, verify
all A-D renderer sessions on the production Windows/OBS 32 machine and mute or
route their desktop sessions as described in the
[OBS setup guide](../docs/obs-setup.md). Process-loopback capture copies audio;
it does not silence desktop playback.
