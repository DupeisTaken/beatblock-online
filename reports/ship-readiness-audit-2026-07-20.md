# Beatblock Online ship-readiness audit

Date: 2026-07-20

Audited version: `0.3.0-alpha.3`, protocol v3

Scope: Rust companion and installer, Lua/Lovely mod payloads, TypeScript protocol,
release automation, documentation, deterministic UI, and runtime resource use.

## Decision

The repository is ready to produce a release candidate. No known code-level
critical or high-severity findings remain open after this pass.

A public Windows release still requires the external acceptance gates in
[Remaining release gates](#remaining-release-gates). In particular, the
installer and runtime must be Authenticode-signed with the release certificate;
that cannot be completed from source code or a development workstation without
the release signing identity.

## Findings and fixes

### Security

| ID     | Severity | Finding                                                                                                                                                                                                | Resolution                                                                                                                                                                                                                                                                                                                                           |
| ------ | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| SEC-01 | Critical | An authenticated peer could send host-owned envelope kinds and reach shared-state handlers. Pending participants and spectators also had paths into gameplay mutation.                                 | Added a deny-by-default peer message classifier, per-kind payload ceilings, and admission/role checks before any room, score, renderer, or broadcast mutation. Host snapshots and unknown kinds are rejected.                                                                                                                                        |
| SEC-02 | Critical | The password exchange authenticated the password but did not prove the host's side of the exchange or bind the self-signed QUIC certificate to it. A malicious endpoint could impersonate a room host. | Added mutual SPAKE proof verification and bound both SPAKE messages plus the observed certificate digest into the authenticated transcript. Added password-attempt rate limits and bounded pending authentication tasks.                                                                                                                             |
| SEC-03 | Critical | Chart transfer acceptance was broader than one exact offer, making unsolicited or replayed archives possible. Executable chart content did not have a distinct confirmation boundary.                  | Authorization is now keyed to participant, request ID, hashes, size, name, and executable flag; it is one-shot and expires. Unsolicited, replayed, mismatched, and unconfirmed executable transfers are rejected and temporary files are removed.                                                                                                    |
| SEC-04 | High     | The localhost API allowed credential placement in URLs and had an overly broad browser trust boundary. Token corruption could also result in a weak or unbounded credential read.                      | Replaced query tokens with a 256-bit random bearer token, or a WebSocket subprotocol token. Tokens are validated, regenerated after corruption, and durably replaced. CORS/WebSocket origins are restricted to exact loopback HTTP origins, comparisons are constant-time, request bodies are limited to 64 KiB, and tracing excludes query strings. |
| SEC-05 | High     | Windows exposed a second unauthenticated loopback TCP IPC transport in addition to the intended game IPC path. Any local process could impersonate the mod.                                            | Windows now exposes only an owner-only named pipe with a protected DACL and remote clients disabled. The TCP development transport is compiled only on non-Windows. A same-user named-pipe integration test covers the production path.                                                                                                              |
| SEC-06 | High     | Installer manifests, OBS markers, and elevated-operation status paths could be forged to reference arbitrary filesystem locations. Cleanup and restore actions trusted those paths.                    | All persisted paths are revalidated against installer-owned roots, expected game shapes, allowlisted OBS locations, UUID status filenames, and non-symlink files before mutation. Manifest and marker reads are size-bounded. Forged escape and oversized-file regression tests were added.                                                          |
| SEC-07 | High     | Release jobs did not strictly bind a tag to package metadata or constrain final ZIP contents, and mutable action tags widened the workflow supply-chain boundary.                                      | Release tags must exactly equal the package version. Final ZIPs reject missing and unexpected entries. Third-party actions are pinned to commit SHAs, build and publication permissions are separated, schemas are regenerated as a drift gate, and release artifacts receive attestations.                                                          |
| SEC-08 | Medium   | Network, IPC, API, storage, and local JSON inputs had several unbounded or mismatched size assumptions.                                                                                                | Added 64 KiB network frames, per-message payload limits, 1 MiB IPC frames, a 1 GiB chart-transfer ceiling matching the schema, bounded config/manifest/token reads, bounded API bodies, and byte-counted storage backlogs.                                                                                                                           |

### Correctness and reliability

| ID     | Severity | Finding                                                                                                                                                                                                  | Resolution                                                                                                                                                                                                                          |
| ------ | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| COR-01 | High     | Score mutations could be accepted from the wrong role or lifecycle state, and rankings diverged between Rust and TypeScript for ties and Unicode names.                                                  | Score updates now require an admitted active player, validate sequence/counter monotonicity, preserve invalid/DNF outcomes, and use the same accuracy/progress/combo/Unicode-scalar ordering in both implementations.               |
| COR-02 | High     | IPC assumed complete reads/writes and could lose or corrupt frames under partial I/O. Some control events were not forwarded back to the game.                                                           | Implemented buffered partial-frame parsing/writing, frame ceilings, bounded queues, and an explicit event allowlist. Transfer, broadcast, progress, acknowledgement, and error events are covered by tests.                         |
| COR-03 | High     | The release lifecycle test launched a stale ignored release executable, so a green result did not prove the current source. Aggregate lifecycle status could also ignore an individual failed invariant. | The test now performs a locked current-source release build, uses the production Windows named pipe, and computes pass/fail from every invariant. Failure permutations are unit-tested.                                             |
| COR-04 | Medium   | Renderer readback could publish an incomplete frame or wait indefinitely, and disabled renderer controls could still be activated.                                                                       | Added committed-frame verification and timeout handling, transactional renderer reconfiguration, exact delayed-sample alignment, disabled-action enforcement, and a final shaded-frame visual probe.                                |
| COR-05 | Medium   | Protocol transfer limits had drifted between the generated schema and Rust validation. The schema generator was not formatter-stable, making the CI drift gate non-deterministic.                        | Rust validation now matches protocol v3 exactly. Generation formats with the repository's Prettier configuration and produces a stable SHA-256 across repeated runs.                                                                |
| COR-06 | Medium   | Configuration updates truncated `config.json` in place, while startup read config and install manifests without ceilings. A crash or corrupt file could erase settings or force a large allocation.      | Settings are validated, serialized to a unique same-directory temporary file, flushed, and atomically replaced. Startup reads are capped at 64 KiB for config and 1 MiB for the install manifest; invalid config falls back safely. |
| COR-07 | Medium   | Lua editing and confirmation strings could split UTF-8 characters, timestamp domains were mixed, and host chart changes could verify against the previous lock.                                          | Added character-safe truncation/deletion, monotonic runtime timestamps with host countdown localization, and post-selection chart-lock verification. Regression coverage includes multibyte names and chart replacement.            |
| COR-08 | Medium   | Installer operation state could overlap, progress reporting could publish inconsistent terminal results, and rollback boundaries were incomplete.                                                        | Operations are mutually exclusive, status files are confined to managed UUID paths, progress is monotonic with one terminal result, and Lovely/OBS/core changes participate in tested rollback transactions.                        |

### Resource allocation

| ID     | Severity    | Finding                                                                                                                   | Resolution                                                                                                                                                                             |
| ------ | ----------- | ------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RES-01 | High        | Slow peers, journal storage, event subscribers, and renderer/control paths could accumulate unbounded work.               | Added bounded peer queues (512), app event/network channels (2,048), storage events (32,768 and 16 MiB), authentication/transfer semaphores, timeouts, and coalesced room publication. |
| RES-02 | High        | Archive build/install and SQLite flush work could run on async networking workers, delaying gameplay and control traffic. | Moved blocking transfer and storage work to bounded blocking tasks, serialized archive builds, and retained control-stream progress under transfer load.                               |
| RES-03 | Medium      | Default nonblocking tracing reserved 128,000 log lines, and runtime-owned logs/cache had no total retention ceiling.      | Reduced the lossy log buffer to 4,096 lines and added age/size pruning: 64 MiB for logs and 128 MiB for chart cache. User imports and match history are excluded from pruning.         |
| RES-04 | Medium      | Renderer polling and export loops used active frequencies even when no renderer/featured slot was active.                 | Added activity-aware 33/100/250/500 ms cadence selection and skipped missed interval ticks rather than accumulating work.                                                              |
| RES-05 | Operational | Abandoned Rust incremental `*-working` directories consumed about 2.33 GB after interrupted/locked builds.                | Removed only verified abandoned working directories after confirming no Cargo or Rust compiler process was active. Normal reusable build caches were retained.                         |

### Design language and UX

| ID    | Severity | Finding                                                                                                                                            | Resolution                                                                                                                                                                |
| ----- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| UX-01 | Medium   | The native Online UI rendered at different physical sizes depending on host DPI, making spacing, typography, and screenshots inconsistent.         | Fixed the native canvas contract at 600×360 with deterministic DPI settings and regenerated all 26 canonical baselines. The harness now fails on any physical-size drift. |
| UX-02 | Medium   | Controls styled as disabled could still respond to keyboard/mouse activation, and disabled commentator/renderer states were visually inconsistent. | Unified disabled-state behavior and palette treatment across forms, room actions, commentator access, renderer controls, and confirmations.                               |
| UX-03 | Low      | A primary chart action clipped at the supported native width; some documentation linked obsolete trial captures.                                   | Shortened the action to “FIND MATCHING CHART,” verified long-error/Unicode states, and updated documentation to canonical current baselines.                              |

## Verification evidence

The final source state passed:

- `pnpm test`: 10 protocol tests, 17 release/lifecycle policy tests, 104
  companion library tests, 2 runtime binary tests, and 21 additional Rust
  integration tests.
- `cargo clippy --locked --all-targets -- -D warnings` and the
  `installer-ui` binary Clippy gate.
- `pnpm typecheck`, `pnpm format:check`, and `git diff --check`.
- `pnpm test:mod`: 17 Lovely signatures, 3 GameManager hooks, 23 in-game
  commands, and both packaged mod ZIPs.
- `pnpm test:ui`: 26 deterministic 600×360 screenshots with zero changed
  pixels.
- `pnpm test:renderer-visual`: final shaded 320×180 OBS frame, 46 frames
  published and none dropped.
- `pnpm test:installer` plus the installer UI status-path unit test:
  standalone and BeatblockPlus install/uninstall, duplicate rejection,
  rollback, forged-path, and bounded-file cases.
- `pnpm test:runtime`: protocol v3 readiness, duplicate mutex rejection,
  explicit shutdown, parent-exit cleanup, no console request, and a
  12,980,224-byte idle working set against the 31,457,280-byte limit.
- `pnpm test:stress`: all six release-mode capacity/backpressure scenarios,
  including 16 players plus 32 spectators and four renderer streams.
- Repeated `pnpm generate:protocol`: stable schema SHA-256
  `27FC928C76F1EA2D4A6E4FD587A033FF9959AF3281EF7B50DBA4708FDDA01217`.
- `pnpm audit --prod`: no known production JavaScript vulnerabilities.
- `cargo-audit 0.22.2`: no applicable x64 Windows vulnerabilities. RustSec
  reported two high-severity `quick-xml 0.39.4` advisories in the Linux-only
  `wayland-scanner` lockfile branch. The supported Windows tree contains no
  `quick-xml`, so both are explicitly documented in
  `companion/.cargo/audit.toml` until upstream permits `quick-xml` 0.41 or
  later. Three unmaintained transitive packages from current Slint dependencies
  remain visible as warnings rather than being hidden.

## Remaining release gates

These are external/physical acceptance gates, not unresolved source defects:

1. Sign the installer and runtime with the production Authenticode certificate,
   verify signatures after packaging, and retain the artifact attestations.
2. Run the signed installer through a clean Windows VM including the UAC
   elevation, firewall rule, repair, move, and uninstall flows.
3. Complete a real Beatblock race over LAN and a separate WAN/NAT setup, with
   disconnect/reconnect and a chart transfer.
4. Load the packaged native source in the supported OBS version and complete a
   sustained four-source capture.

The release should not be labeled generally available until these gates are
recorded against the exact signed artifacts.
