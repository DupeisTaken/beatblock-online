# Installer-only architecture

```text
BeatblockTogetherInstaller.exe (maintenance only, exits)
    -> Lovely + one adapter + hidden runtime + optional OBS plugin

Steam -> Beatblock -> adaptive Online dashboard -> LÖVE channel -> named pipe v2
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

The runtime/API never publishes passwords, API tokens, absolute chart paths, or unrelated process data to room snapshots. Protocol-v1 clients receive an explicit compatibility failure.

## Supported-build IPC details

The gameplay hooks only enqueue compact Lua tables. A dedicated LÖVE thread owns all IPC and therefore keeps pipe, network, export, hashing, and storage work off the gameplay thread. On the supported reference build it uses LuaJIT FFI with `CreateFileW`/`ReadFile`/`WriteFile` against `\\.\pipe\beatblock-together-v2`. Writes are completed in a loop, outbound work is capped at 16 messages per pass, and carriage returns/newlines introduced by Beatblock's JSON encoder are removed before newline framing.

The dashboard derives a UI-only normalized view from protocol-v2 room, participant, renderer, history, and diagnostics snapshots. `dashboard_model.lua` owns phase selection, roster totals and scrolling, focus transitions, and next-action precedence without changing the wire protocol. CI executes this pure module with Beatblock's bundled Lua 5.1 runtime.

The runtime returns only runtime-owned snapshots, acknowledgements, rankings, compatibility errors, and diagnostics. Client telemetry is never echoed back to the same game. Explicit session shutdown discards unsent live snapshots and makes `runtime.session_end` the final queued control message.

High-rate score mutations are validated and journaled immediately, while full
room snapshots, recovery state, OBS room exports, and peer fan-out coalesce onto
a 20 Hz publication clock. SQLite journal rows commit in 25 ms transactions and
local NDJSON journals keep one buffered writer per run with a 50 ms flush bound.
The durable retry backlog is capped at 32,768 events, the non-blocking NDJSON
queue at 8,192 messages, and only 32 journal files remain open at once. Raw
SQLite and NDJSON telemetry is pruned to the documented 30-day retention period
at startup. Local IPC and QUIC control frames reject messages larger than 1 MiB,
and failed-password tracking is bounded by both address count and time. Local
IPC serves at most 16 simultaneous clients; the host serves at most 64 pending
QUIC authentications and closes any handshake that does not finish within ten
seconds. These limits prevent abandoned or hostile connections from retaining
an unbounded number of tasks and socket buffers.
OBS text exports collapse into 100 ms batches, skip unchanged fields, and use
atomic replacement without forcing ephemeral overlay state through the physical
disk cache. These clocks keep durable ordering separate from presentation work.

Renderer input alignment parks when no stream is active and otherwise follows
the fastest configured 30/60 Hz stream. Game clients send 60 Hz render input
during gameplay and five Hz while held on an Online screen. Renderer processes
use a fixed three-frame memory-mapped ring, two asynchronous GPU readback
canvases, and sequence-last publication so OBS never consumes a partial frame.
An async readback slot that receives no driver callback is reclaimed after one
second with ticket validation for late callbacks. OBS releases stale CPU/GPU
buffers and periodically reopens an idle mapping so atomic renderer-file
replacement cannot leave a source attached to an abandoned file object.

Chart hashing and imported ZIP extraction stream file contents and reject
packages above 1 GiB, 20,000 entries, or the path-length safety ceiling. These
limits bound memory, file handles, and disk allocation before imported content
is activated.

Runtime launch happens from the worker through `CreateProcessA` with an explicitly
defined Win32 startup structure and `CREATE_NO_WINDOW`. Physical `.test`
injection showed that `WinExec` can wait indefinitely for GUI input-idle from
the intentionally windowless runtime, leaving the host form without a reply.
The worker closes its process/thread handles immediately and continues into the
named-pipe handshake. LuaSocket remains the isolated loopback fallback for
environments where FFI cannot load; the supported build uses the named pipe.
