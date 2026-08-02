# Beatblock Online Technical Reference

This is the technical documentation hub for maintainers, event operators,
integrators, and readers who want implementation and validation detail. For
task-focused player instructions, use the [Player Guide](player-guide.md).

## Contents

- [Feature status and boundaries](#feature-status-and-boundaries)
- [Operation and compatibility](#operation-and-compatibility)
- [OBS and integrations](#obs-and-integrations)
- [Runtime and protocol](#runtime-and-protocol)
- [Contributor and release documentation](#contributor-and-release-documentation)
- [Repository components](#repository-components)
- [Common developer commands](#common-developer-commands)

## Feature status and boundaries

The status below distinguishes released player controls from backend
capabilities and future product expectations.

| Area                               | Current status                                                                                                                                                                                                                                                           |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Direct-IP room flow                | **Player-facing in beta.** The in-game UI hosts and joins password-protected rooms, admits Players/Spectators through host approval, verifies charts, readies Players, starts synchronized races, and displays results/history.                                          |
| Room size and roles                | **Player-facing in beta.** Up to 16 Players and 32 Spectators are modeled and stress-tested. A host may Play or Direct. Commentator is a host-granted permission on an admitted Spectator.                                                                               |
| Charts and sets                    | **Player-facing in beta.** Official/custom selection, exact matching, ordered setlists, and consent-based custom transfer are implemented. Official content is local-only.                                                                                               |
| Broadcast and OBS                  | **Advanced beta.** Four renderer slots, Commentator plan mirroring, Player Stream video, A–D audio, Autoplay audio, and text exports are implemented. Physical OBS 32.1.2 color/audio routing, A/V alignment, and long-run drift remain event-machine validation gates.  |
| Password-only admission            | **Backend/API only.** The runtime and local API support it, but the current in-game host form always creates a host-approval room.                                                                                                                                       |
| Force Start                        | **Backend/API only.** Protocol, runtime, and local API support a forced start; the current in-game dashboard always sends a normal start and exposes no Force Start control.                                                                                             |
| `bbt://` join URI                  | **Generated metadata only.** The runtime/API can return a password-free join URI, but the current in-game Connect form does not parse it; users enter address and UDP port separately.                                                                                   |
| Public discovery and relay         | **Not implemented.** There are no accounts, matchmaking, public room browser, hosted relay, operator website, or cloud deployment.                                                                                                                                       |
| Newer Beatblock builds             | **Accepted only when structurally valid; unverified until tested.** Exact room matching remains the normal default.                                                                                                                                                      |
| Installer trust and field coverage | **Beta limitation.** Release checksums and GitHub attestations provide integrity/provenance, but the installer is not Authenticode-signed. Disposable-machine updater, antivirus/locked-file, WAN, and supported-host physical trials remain environment-specific gates. |

Backend-only entries are implemented and tested surfaces, not finished player
features. Do not document them as available in the in-game workflow until the
dashboard exposes and validates them.

## Operation and compatibility

- [Advanced hosting and room operations](operator-guide.md): room policies,
  roles, setlists, results, direct UDP, frp, and orderly shutdown.
- [Beatblock compatibility](compatibility.md): tested baseline, future-build
  policy, exact room matching, and compatibility reports.
- [Online shell and room roles](mod-guide.md): dashboard state model, workspaces,
  chart transfer consent, and Commentator behavior.
- [Installation and injection](injection.md): installer transactions, payload
  layout, Lovely integration, repair, and developer/test paths.
- [Creating issues](issues.md): reporting and triage contract.

## OBS and integrations

- [OBS, reconstructed streams, and text exports](obs-setup.md): native source
  install, renderer audio, Autoplay, capture troubleshooting, export files, and
  local API.
- [Recommended resource budgets](benchmarking.md#recommended-system-and-network-specifications):
  code-derived and measured CPU, memory, disk, pixel-copy, and network bounds.

## Runtime and protocol

- [Installer/runtime architecture](architecture.md): process ownership,
  startup/shutdown, and Windows IPC.
- [Direct-host protocol v3](protocol.md): message contracts, QUIC room traffic,
  local control acknowledgements, chart transfer, and compatibility behavior.
- [Generated protocol v3 schema](../protocol/schemas/v3/protocol.json)
- [Latest complete acceptance report](../reports/trial-runs/full-capability-latest.md)

## Contributor and release documentation

- [Benchmarks, stress gates, and trials](benchmarking.md)
- [Reproducible release workflow](releasing.md)
- [Release history](releases/index.md), pairing concise public notes with
  technical changelogs
- [Beta.5 ship-readiness review](../reports/ship-readiness-review-v0.3.0-beta.5.md)
- [Security, correctness, resource, and UX audit](../reports/ship-readiness-audit-2026-07-20.md)
- [Installer reliability evidence](../reports/trial-runs/installer-reliability-latest.md)
- [Injected lifecycle evidence](../reports/trial-runs/injected-installer-lifecycle-latest.md)

## Repository components

| Path                 | Responsibility                                                                                                        |
| -------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `mod`                | Shared Lua telemetry/gameplay core and mutually exclusive standalone Lovely and BeatblockPlus adapters.               |
| `companion`          | Rust installer and hidden runtime, including networking, SQLite, renderer, local API, and exports.                    |
| `obs-plugin`         | Native Stream A–D video plus independent A–D and Autoplay audio sources.                                              |
| `protocol`           | Protocol v3 JSON Schema and TypeScript conformance implementation; archived v2 remains for compatibility diagnostics. |
| `scripts`            | Reproducible build, validation, packaging, trial, and UI automation.                                                  |
| `reports/trial-runs` | Machine-readable and Markdown acceptance evidence.                                                                    |

## Common developer commands

Install workspace dependencies with `pnpm install`. Rust must be available on
`PATH`.

```text
pnpm generate:protocol  Regenerate protocol v3 schemas
pnpm test:ui           Render and compare all 600x360 UI scenarios
pnpm test:mod          Package and validate both Lua adapters
pnpm test              Run protocol, workflow, and Rust tests
pnpm test:stress       Run direct-room and export stress tests
pnpm trial             Run the complete acceptance and benchmark suite
pnpm typecheck         Build protocol types and check the Rust workspace
pnpm build             Build the reproducible Windows release
```

`pnpm build` writes ignored generated outputs under `artifacts/`, `release/`,
and `mod/releases/`. Read [Reproducible release workflow](releasing.md) before
publishing or handling generated binaries.

[Return to the documentation home](README.md) or [project README](../README.md).
