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

The room lifecycle is `forming -> chart_locked -> ready -> countdown -> playing -> results -> set_complete -> closed`. The host derives accuracy from ordered score mutations and stores a two-minute diagnostic snapshot, but only a live authenticated network session can own or resume a room.

The runtime/API never publishes passwords, API tokens, absolute chart paths, or unrelated process data to room snapshots. Protocol-v1 clients receive an explicit compatibility failure.

## Supported-build IPC details

The gameplay hooks only enqueue compact Lua tables. A dedicated LÖVE thread owns all IPC and therefore keeps pipe, network, export, hashing, and storage work off the gameplay thread. On the supported reference build it uses LuaJIT FFI with `CreateFileW`/`ReadFile`/`WriteFile` against `\\.\pipe\beatblock-together-v2`. Writes are completed in a loop, outbound work is capped at 16 messages per pass, and carriage returns/newlines introduced by Beatblock's JSON encoder are removed before newline framing.

The dashboard derives a UI-only normalized view from protocol-v2 room, participant, renderer, history, and diagnostics snapshots. `dashboard_model.lua` owns phase selection, roster totals and scrolling, focus transitions, and next-action precedence without changing the wire protocol. CI executes this pure module with Beatblock's bundled Lua 5.1 runtime.

The runtime returns only runtime-owned snapshots, acknowledgements, rankings, compatibility errors, and diagnostics. Client telemetry is never echoed back to the same game. Explicit session shutdown discards unsent live snapshots and makes `runtime.session_end` the final queued control message.

Runtime launch happens from the worker through `WinExec`, rather than a LuaJIT-owned `CreateProcessW` structure, because the supplied LÖVE 12 build crashed or stalled in those worker-thread variants during physical injection testing. Both installed binaries use the Windows GUI subsystem, so this compatibility choice still creates no terminal or visible runtime window. LuaSocket remains packaged as the isolated loopback fallback for environments where FFI cannot load; the supported build uses the named pipe.
