# Beatblock Online installer/runtime capability trial

Generated: 2026-08-01T16:41:31.887Z

Automated gate: **PASS**

- PASS - Protocol v3 typecheck: 0.54 s
- PASS - Protocol v3 schema generation: 0.22 s
- PASS - Protocol v3 tests: 0.87 s
- PASS - Build lean runtime payload: 25.24 s
- PASS - Rust runtime, installer, Lua, and stress tests: 66.19 s
- PASS - Package both in-game adapters: 1.29 s
- PASS - In-game mod conformance: 0.07 s
- PASS - Deterministic 600x360 screenshot gate: 22.85 s
- PASS - Hidden runtime lifecycle and resource gate: 6.41 s
- PASS - 16-player / 32-spectator direct-host simulation: 15.82 s
- PASS - Runtime I/O benchmark report: 0.04 s

Machine-readable metrics are in `full-capability-latest.json`, `host-room-simulation-latest.json`, `runtime-lifecycle-latest.json`, and `runtime-benchmark-latest.json`. Physical WAN, OBS, GPU, and clean-machine release gates use the manual trial sheets under `docs/trials`; simulations are not reported as physical results.
