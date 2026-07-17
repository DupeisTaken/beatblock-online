# Direct-host protocol v3

Reliable messages use:

```json
{
  "version": 3,
  "type": "room.ready_request",
  "sequence": 42,
  "runTimeUs": 82431000,
  "requestId": "1784220000-123456-7",
  "payload": { "ready": true, "requestId": "1784220000-123456-7" }
}
```

Every in-game control request receives `control.ack` or `control.error` with its request ID and has a bounded client-side response deadline. Runtime lifecycle events are `runtime.ready`, `runtime.disconnected`, and `runtime.error`. A disconnect rejects the active request before the worker begins its bounded-backoff relaunch loop. Protocol v2 rooms are rejected with an upgrade message; mixed-version rooms are unsupported. The named pipe is `\\.\pipe\beatblock-online-v3`.

The first IPC message is `client.hello` with a per-game `instanceId`. The runtime accepts reconnects from that instance and rejects other game processes, preventing control replies and room snapshots from crossing between simultaneous Beatblock copies. A two-second `client.ping` / `runtime.heartbeat` exchange drives the in-game liveness indicator.

Control groups cover room admission/roles/Commentator grants/kick, setlist editing, chart-transfer consent/cache, renderer A-D configuration, Commentator mirror enablement, history delete/prune, settings, diagnostics, token rotation, export/log opening, restart, and session shutdown. `runtime.snapshot` publishes complete sanitized room, Broadcast plan, machine-local renderer health, Commentator status, history, settings, and diagnostics state to Lua.

Host room snapshots and `room.start_scheduled` events carry `serverTimeMs`.
Participant runtimes convert `scheduledStartTimeMs`/`serverStartTimeMs` into
their local clock domain while preserving the host's remaining countdown.
`run.started` must match the verified chart's authoritative note count. A room
accepts one final result per participant per chart, independent of the
client-provided run ID.

High-rate 32-byte datagrams contain version, stable render-source ID/sequence, run timestamp, beat, processed paddle angle, held taps, and flags. Reliable score mutations remain authoritative. The host relays samples only for active plan assignments and only to authorized, enabled Commentators.

Custom packages use authenticated request/offer/decision control messages and one bounded QUIC unidirectional stream per peer. Archive SHA-256 and the extracted canonical chart hash are both required. The executable schema is [protocol.json](../protocol/schemas/v3/protocol.json); [v2](../protocol/schemas/v2/protocol.json) is archived only to identify incompatible clients.
