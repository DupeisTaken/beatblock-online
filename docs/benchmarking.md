# Benchmarks, stress tests, and capability trials

The verification suite covers both correctness under maximum alpha load and measured local performance. Run commands from the repository root on Windows with Node.js, pnpm, and stable Rust installed.

## Commands

```powershell
pnpm test:stress
pnpm benchmark
pnpm trial
```

- `test:stress` is the fast regression gate. It concurrently joins 16 players and 32 spectators, verifies readiness, ingests 1,920 authoritative score events, and runs the Rust queue, broadcast, chart-cache, and atomic-export stress tests.
- `benchmark` runs the maximum-capacity server scenario with 3,840 score events and the companion I/O workload with a 4 MiB chart, 250 OBS snapshots, and 5,000 journal events.
- `trial` is the automated release demonstration. It runs all TypeScript tests, production builds, in-game command and patch conformance, Rust stress tests, both benchmarks, and writes a single PASS/FAIL record.

Cargo is located automatically at `%USERPROFILE%\.cargo\bin\cargo.exe` when a newly installed Rust toolchain is not yet on the current shell's PATH.

## What a full trial demonstrates

The server trial proves invite redemption, rotating refresh credentials, one-time spectator browser handoff, 16-player and 32-spectator limits, concurrent joins, chart lock and compatibility readiness, a scheduled start, authoritative scoring/ranking, idempotent duplicate delivery, reconnect at capacity, and final results.

The companion trial proves canonical chart hashing and cache hits, cache invalidation after a package change, complete atomic OBS exports, local fan-out to 32 consumers, bounded remote-queue behavior, and recovery of every ordered event from the journal after simulated backpressure.

The mod conformance trial regenerates both distributions from the shared Lua core, checks every in-game command against a companion handler, validates Lovely patch signatures against the supplied Beatblock source archives, inspects both release ZIPs, and compiles every distributed Lua chunk with Beatblock's own `lua51.dll`. This is a deterministic integration gate; a clean-machine launch remains a separate manual release check because Lovely injection and graphics/input require the installed game process.

## Reports and thresholds

Generated reports are placed in `reports/trial-runs`:

- `full-capability-latest.md` — readable trial summary.
- `full-capability-latest.json` — complete trial manifest and environment.
- `server-stress-latest.json` — capacity, correctness, latency, throughput, event-loop, and memory measurements.
- `companion-benchmark-latest.json` — chart cache, atomic export, and journal throughput measurements.

The blocking alpha thresholds are:

| Measurement                                   |           Threshold |
| --------------------------------------------- | ------------------: |
| Authoritative event count                     |       exactly 3,840 |
| Server ingest p95 on loopback/in-memory store |        below 250 ms |
| Atomic export p95                             |        below 100 ms |
| Companion journal throughput                  |  above 100 events/s |
| Recovery, duplicate handling, capacity checks | exact, with no loss |

These are regression thresholds, not production capacity claims. PostgreSQL, TLS termination, and a real 100 ms network path must be load-tested on the intended host before an event. The acceptance target for an end-to-end deployed instance remains live-update p95 below 250 ms under 100 ms network latency.

## Adding a regression

Put deterministic server concurrency cases in `server/test/stress.test.ts` and companion durability cases in `companion/tests/stress.rs`. Keep benchmarks reproducible, report workload sizes with every metric, and fail with a nonzero exit code when an integrity invariant or published threshold is missed.
