# Installer-only architecture

```text
BeatblockOnlineInstaller.exe (maintenance only, exits)
    -> Lovely + one adapter + hidden runtime + optional OBS plugin

Steam -> Beatblock -> Online workspace shell -> LÖVE channel -> named pipe v3
                                      -> BeatblockOnlineRuntime.exe
                                         -> direct QUIC room
                                         -> SQLite/history
                                         -> API + atomic exports
                                         -> renderer A-D supervision
```

The installer never hosts, joins, renders, exports, or stays in the tray. The runtime has no Slint/tray dependency, console, or visible window. It is launched lazily with Beatblock's parent PID, is single-instance per user, and exits with **Exit Online** or the parent process.

Keeping QUIC, SQLite, hashing, renderer supervision, and OBS transport out of Beatblock prevents that work from blocking its frame loop and isolates native failures. A participant connection retries with a bounded 30-second grace; a runtime restart returns to Offline instead of presenting a serialized room without live ownership or network credentials.

Reconnect attempts use one owned task and one absolute deadline, and reconnect
reservations are expired by the runtime maintenance loop rather than detached
sleep tasks. Successful UPnP mappings are released on an orderly room exit; the
finite router lease remains the crash fallback.

The room lifecycle is `forming -> chart_locked -> ready -> countdown -> playing -> results -> set_complete -> closed`. The host derives accuracy from ordered score mutations and stores a two-minute diagnostic snapshot, but only a live authenticated network session can own or resume a room.

`run.started` is validated against the locked chart's note count and records one
started participant per scheduled chart. Completion is likewise
participant-scoped rather than trusting client-provided run IDs. Thirty seconds
after the scheduled start, assigned players that never entered Game are
finalized as DNF so Force Start and failed client loads cannot strand the room.

Scheduled timestamps are localized when they cross from the host runtime to a
participant runtime. The host sends its send time with the authoritative target;
the participant preserves the remaining countdown in its own clock domain
before forwarding the snapshot to Lua. System-clock skew therefore cannot
desynchronize launch.

The runtime/API never publishes passwords, API tokens, absolute chart paths, or unrelated process data to room snapshots. Non-v3 clients receive an explicit compatibility failure.

## Supported-build IPC details

The gameplay hooks only enqueue compact Lua tables. A dedicated LÖVE thread owns all IPC and therefore keeps pipe, network, export, hashing, and storage work off the gameplay thread. On Windows it uses LuaJIT FFI with `CreateFileW`/`ReadFile`/`WriteFile` against the owner-only, remote-client-rejecting `\\.\pipe\beatblock-online-v3`; the runtime does not expose the unauthenticated TCP fallback in production Windows builds. Writes are completed in a loop, outbound work is capped at 16 messages per pass, and carriage returns/newlines introduced by Beatblock's JSON encoder are removed before newline framing.

The Online shell derives explicit room, Results, Setlist, Broadcast, History, Settings, Help, Form, and Confirm view state from protocol-v3 snapshots. `dashboard_model.lua` owns phase selection, stable-ID filtering, score presentation, role gates, and next-action precedence. CI executes this pure module with Beatblock's bundled Lua 5.1 runtime and compares the real native UI against deterministic screenshots.

The runtime returns only runtime-owned snapshots, acknowledgements, rankings, compatibility errors, and diagnostics. Client telemetry is never echoed back to the same game. Explicit session shutdown discards unsent live snapshots and makes `runtime.session_end` the final queued control message.

High-rate score mutations are validated and journaled immediately, while full
room snapshots, recovery state, OBS room exports, and peer fan-out coalesce onto
a 20 Hz publication clock. SQLite journal rows commit in 25 ms transactions and
local NDJSON journals keep one buffered writer per run with a 50 ms flush bound.
The durable retry backlog is capped at both 32,768 events and 16 MiB, the
non-blocking NDJSON queue at 8,192 messages, and only 32 journal files remain
open at once. Raw SQLite and NDJSON telemetry is pruned to the documented
30-day retention period at startup. Local IPC rejects frames larger than 1 MiB;
QUIC control frames use a 64 KiB ceiling plus smaller per-message-class limits.
Application event channels hold at most 2,048 messages and each peer control
queue holds 512. Failed-password tracking is bounded by both address count and
time. Local IPC serves at most 16 simultaneous clients; the host serves at most
64 pending QUIC authentications and closes any handshake that does not finish
within ten seconds. Client and server SPAKE proofs cover the complete exchange
and the TLS certificate fingerprint observed by the client, so the room
password authenticates the otherwise self-signed QUIC channel. These limits
prevent abandoned or hostile connections from retaining an unbounded number
of tasks and socket buffers.
OBS text exports collapse into 100 ms batches, skip unchanged fields, and use
atomic replacement without forcing ephemeral overlay state through the physical
disk cache. These clocks keep durable ordering separate from presentation work.

Renderer input alignment parks when no stream is active and otherwise follows
the fastest configured 30/60 Hz stream. Game clients send 60 Hz render input
during gameplay and five Hz while held on an Online screen. Renderer processes
use a fixed three-frame memory-mapped ring, two asynchronous GPU readback
canvases, and sequence-last publication so OBS never consumes a partial frame.
Cached samples warm each isolated native game behind a closed capture gate.
First-scoring-note anchors translate participant-local monotonic clocks into one
cohort presentation epoch; the release sample seeds the paddle and enables OBS,
then delayed beats drive Beatblock's own event and results lifecycle. Reliable
tap edges retain the originating player's judgement beat and input offset.
An async readback slot that receives no driver callback is reclaimed after one
second with ticket validation for late callbacks. OBS releases stale CPU/GPU
buffers and periodically reopens an idle mapping so atomic renderer-file
replacement cannot leave a source attached to an abandoned file object.

Chart hashing and imported ZIP extraction stream file contents and reject
packages above 1 GiB, 20,000 entries, or the path-length safety ceiling. An
incoming transfer needs an exact, one-use authorization matching its peer,
request ID, hashes, size, name, and executable-content decision before a
UUID-named temporary file is created. Failed, replayed, cancelled, and
disconnected transfers remove their temporary archives. Packaging and
installation use serialized blocking workers so large archives cannot stall
the network event loop. These limits bound memory, file handles, and disk
allocation before imported content is activated.

Runtime launch happens from the worker through `CreateProcessA` with an explicitly
defined Win32 startup structure and `CREATE_NO_WINDOW`. Physical `.test`
injection showed that `WinExec` can wait indefinitely for GUI input-idle from
the intentionally windowless runtime, leaving the host form without a reply.
The worker closes its process/thread handles immediately and continues into the
named-pipe handshake. LuaSocket remains a loopback development fallback on
non-Windows environments; the supported Windows build uses only the
access-controlled named pipe.
