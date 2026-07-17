# Beatblock Online installer/runtime capability trial

Generated: 2026-07-17T19:19:34.934Z

Automated gate: **PASS**

- PASS - Protocol v3 typecheck: 0.50 s
- PASS - Protocol v3 schema generation: 0.14 s
- PASS - Protocol v3 tests: 0.82 s
- PASS - Build lean runtime payload: 43.18 s
- PASS - Rust runtime, installer, Lua, and stress tests: 73.26 s
- PASS - Package both in-game adapters: 1.25 s
- PASS - In-game mod conformance: 0.06 s
- PASS - Deterministic 600x360 screenshot gate: 16.84 s
- PASS - Hidden runtime lifecycle and resource gate: 8.00 s
- PASS - 16-player / 32-spectator direct-host simulation: 14.35 s
- PASS - Runtime I/O benchmark report: 0.05 s

Machine-readable metrics are in `full-capability-latest.json`, `host-room-simulation-latest.json`, `runtime-lifecycle-latest.json`, and `runtime-benchmark-latest.json`. Physical WAN, OBS, GPU, and clean-machine release gates use the manual trial sheets under `docs/trials`; simulations are not reported as physical results.
