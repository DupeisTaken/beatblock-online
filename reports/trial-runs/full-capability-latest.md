# Beatblock Together installer/runtime capability trial

Generated: 2026-07-15T16:09:53.194Z

Automated gate: **PASS**

- PASS - Protocol v2 typecheck: 0.57 s
- PASS - Protocol v2 schema generation: 0.15 s
- PASS - Protocol v2 tests: 0.95 s
- PASS - Rust runtime, installer, Lua, and stress tests: 65.80 s
- PASS - Package both in-game adapters: 2.96 s
- PASS - In-game mod conformance: 0.09 s
- PASS - Hidden runtime lifecycle and resource gate: 8.20 s
- PASS - 16-player / 32-spectator direct-host simulation: 13.20 s
- PASS - Runtime I/O benchmark report: 0.05 s

Machine-readable metrics are in `full-capability-latest.json`, `host-room-simulation-latest.json`, `runtime-lifecycle-latest.json`, and `runtime-benchmark-latest.json`. Physical WAN, OBS, GPU, and clean-machine release gates use the manual trial sheets under `docs/trials`; simulations are not reported as physical results.
