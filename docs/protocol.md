# Protocol v1

Every IPC, gateway, and local event uses this envelope:

```json
{
  "version": 1,
  "type": "run.score_delta",
  "sequence": 418,
  "timestampMs": 1784098142382,
  "payload": {}
}
```

Unknown versions, malformed required fields, and unknown remote message types receive explicit errors. Competitive score payloads include a separate zero-based `runSequence` used for journal reconciliation.

Remote messages are `client.hello`, `gateway.ready`, `gateway.disconnected`, `clock.ping`, `clock.pong`, `lobby.subscribe`, `lobby.snapshot`, `run.started`, `run.score_delta`, `run.invalid`, `run.finished`, and `gameplay.snapshot`. Local snapshots run at 20-30 Hz; remote rankings target 10 Hz.

The in-game mod sends local commands to the companion as ordinary envelopes: `lobby.create_request`, `lobby.join_request`, `lobby.chart_select_request`, `lobby.chart_verify_request`, `lobby.ready_request`, `lobby.start_request`, `lobby.leave_request`, and `lobby.close_request`. The companion performs credentialed HTTP and filesystem work off the gameplay thread, then returns `lobby.snapshot`, `lobby.context`, `chart.verification`, or `companion.error` through IPC.

`run.started` includes the selected package hash, variant name, and Beatblock-derived maximum hit count. A mismatch against the locked lobby chart invalidates the run before score ingestion.

Generate the executable JSON Schema bundle at `protocol/schemas/v1/protocol.json` with `pnpm generate:protocol`. Protocol major versions never silently downgrade.
