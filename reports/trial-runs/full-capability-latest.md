# Beatblock Online installer/runtime capability trial

Generated: 2026-07-29T12:23:21.277Z

Automated gate: **PASS**

- PASS - Protocol v3 typecheck: 2.98 s
- PASS - Protocol v3 schema generation: 1.64 s
- PASS - Protocol v3 tests: 22.34 s
- PASS - Build lean runtime payload: 112.48 s
- PASS - Rust runtime, installer, Lua, and stress tests: 715.32 s
- PASS - Package both in-game adapters: 16.74 s
- PASS - In-game mod conformance: 0.77 s
- PASS - Deterministic 600x360 screenshot gate: 87.75 s
- PASS - Hidden runtime lifecycle and resource gate: 11.32 s
- PASS - 16-player / 32-spectator direct-host simulation: 72.48 s
- PASS - Runtime I/O benchmark report: 0.16 s

Machine-readable metrics are in `full-capability-latest.json`, `host-room-simulation-latest.json`, `runtime-lifecycle-latest.json`, and `runtime-benchmark-latest.json`. Physical WAN, OBS, GPU, and clean-machine release gates use the manual trial sheets under `docs/trials`; simulations are not reported as physical results.
