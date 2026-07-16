# Direct-host protocol v2

Reliable messages use:

```json
{
  "version": 2,
  "type": "room.ready_request",
  "sequence": 42,
  "runTimeUs": 82431000,
  "requestId": "game-7",
  "payload": { "ready": true, "requestId": "game-7" }
}
```

Every in-game control request receives `control.ack` or `control.error` with its request ID. Runtime startup/error events are `runtime.ready` and `runtime.error`. Version 1 is rejected; the named pipe is `\\.\pipe\beatblock-together-v2`.

Control groups cover room admission/roles/kick, setlist editing, renderer A-D configuration, history delete/prune, settings, diagnostics, token rotation, export/log opening, restart, and session shutdown. `runtime.snapshot` publishes complete sanitized room, renderer, history, settings, and diagnostics state to Lua.

High-rate 32-byte datagrams contain version, session/sequence, run timestamp, beat, processed paddle angle, held taps, and flags. Reliable score mutations remain authoritative. The executable schema is [protocol.json](../protocol/schemas/v2/protocol.json).
