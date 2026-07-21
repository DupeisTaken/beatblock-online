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

Room authentication uses mutual SPAKE proofs over the full client/server exchange. Both proofs include the protocol version, nonce, and the TLS certificate fingerprint observed by the client. The password therefore authenticates the self-signed QUIC channel without transmitting or storing the password as a reusable network credential.

The first IPC message is `client.hello` with a per-game `instanceId`. The runtime accepts reconnects from that instance and rejects other game processes, preventing control replies and room snapshots from crossing between simultaneous Beatblock copies. A two-second `client.ping` / `runtime.heartbeat` exchange drives the in-game liveness indicator.

Control groups cover room admission/roles/Commentator grants/kick, setlist editing, chart-transfer consent/cache, renderer A-D configuration, Commentator mirror enablement, history delete/prune, settings, diagnostics, token rotation, export/log opening, restart, and session shutdown. `runtime.snapshot` publishes complete sanitized room, Broadcast plan, machine-local renderer health, Commentator status, history, settings, and diagnostics state to Lua.

Room snapshots optionally carry `validityChecksEnabled`; omission preserves the
strict `true` default for older protocol-v3 peers. The host may change it only
before countdown. Strict rooms invalidate sequence gaps and client-reported
integrity failures. Casual rooms recover from a gap using cumulative score
totals and allow a new native run to replace an unfinished attempt. Structural
counter bounds, chart verification, completion, launch-timeout, and disconnect
DNF rules remain mandatory in both modes.

Host room snapshots and `room.start_scheduled` events carry `serverTimeMs`.
Participant runtimes convert `scheduledStartTimeMs`/`serverStartTimeMs` into
their local clock domain while preserving the host's remaining countdown.
Every attempt carries a non-empty `runId` of at most 128 characters on
`run.started`, score, invalidation, and finish events. `run.started` must match
the verified chart's authoritative note count. Duplicate starts for the active
ID are idempotent; a new ID resets cumulative counters for the new attempt,
retires the previous ID, and applies the room's competitive/casual retry policy.
Late events from retired attempts are ignored. A room accepts one final result
per participant per chart, independent of the client-provided run ID.

Structurally invalid score messages are recorded as a participant INVALID in a
competitive room but do not tear down local IPC. Cumulative score events only
advance their run sequence after entering the bounded ordered queue, preventing
local backpressure from manufacturing a sequence gap. Reconnect restores the
same authenticated participant and preserves existing INVALID/DNF verdicts.

High-rate 32-byte datagrams contain version, stable render-source ID/sequence, run timestamp, beat, processed paddle angle, held taps, and flags. Reliable source-authored renderer keyframes carry accuracy, score totals, average offset, and the final Results marker; each OBS child aligns them with the same delayed datagram timeline through a sequence-committed score sidecar. Authoritative room scoring still comes from ordered score mutations. The host relays renderer data only for active plan assignments and only to authorized, enabled Commentators.

Custom packages use authenticated request/offer/decision control messages and one bounded QUIC unidirectional stream per peer. A decision grants an exact, one-use receive authorization tied to the peer and complete offered metadata before disk allocation; executable or script content always requires separate confirmation and is never covered by room trust. Archive SHA-256 and the extracted canonical chart hash are both required. The executable schema is [protocol.json](../protocol/schemas/v3/protocol.json); [v2](../protocol/schemas/v2/protocol.json) is archived only to identify incompatible clients.
