# Beatblock Online installer/runtime capability trial

Generated: 2026-07-17T14:41:43.822Z

Automated gate: **PASS**

- PASS - Protocol v2 typecheck: 0.53 s
- PASS - Protocol v2 schema generation: 0.13 s
- PASS - Protocol v2 tests: 0.87 s
- PASS - Rust runtime, installer, Lua, and stress tests: 179.75 s
- PASS - Package both in-game adapters: 1.48 s
- PASS - In-game mod conformance: 0.07 s
- PASS - Hidden runtime lifecycle and resource gate: 7.88 s
- PASS - 16-player / 32-spectator direct-host simulation: 37.29 s
- PASS - Runtime I/O benchmark report: 0.05 s

Machine-readable metrics are in `full-capability-latest.json`, `host-room-simulation-latest.json`, `runtime-lifecycle-latest.json`, and `runtime-benchmark-latest.json`. Physical WAN, OBS, GPU, and clean-machine release gates use the manual trial sheets under `docs/trials`; simulations are not reported as physical results.
