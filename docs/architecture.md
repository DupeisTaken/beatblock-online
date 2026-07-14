# Architecture

## Data path

```text
Beatblock scoring mutation
  -> shared Lua hook
  -> LÖVE thread channel
  -> named pipe (LuaSocket loopback fallback)
  -> Rust companion journal
  -> WSS gateway
  -> Fastify lobby/scoring service
  -> PostgreSQL + authenticated spectators

Rust companion
  -> loopback HTTP/WebSocket
  -> OBS browser sources / local applications
  -> atomic text and JSON exports
```

Beatblock is the player control plane. Its Online state owns lobby creation/joining, chart selection, local verification, readiness, countdown, race HUD, and results. The loopback webpage is limited to account setup, diagnostics, OBS configuration, and authenticated spectator handoff.

The gameplay thread only calculates small snapshots and pushes strings into a LÖVE channel. Pipe, socket, credential, HTTP, journal, and export work occurs outside that thread.

## Competitive lifecycle

`forming -> chart_locked -> ready -> countdown -> playing -> results -> closed`

Only organizers and operators create lobbies. A competitor can ready only when the complete canonical package hash and selected variant match and the client proof identifies the supported game build, an alpha-compatible client, and an allowed mod inventory. The mod calculates expected maximum hits using Beatblock's own `Event.hitCount` functions. The server verifies the hash, variant, and max hits again when `run.started` arrives. It schedules a start five seconds in the future; clients calibrate server epoch time against LÖVE's monotonic clock and hold Beatblock's actual `startPending` gate until the scheduled time.

## Scoring

The server validates monotonic cumulative totals and computes accuracy itself:

```text
floor((((currentMaxHits - misses - barelies / 4) / currentMaxHits) * 100) * 100) / 100
```

Run score events have a run-local sequence. Duplicate events are idempotent and gaps invalidate the alpha run. The companion appends competitive run messages to NDJSON before queuing remote transmission and resends journals after reconnect.

## Chart canonicalization

Directories and ZIPs are reduced to normalized `/` paths and raw bytes. Entries are sorted bytewise; archive timestamps and known OS junk are excluded. Each path length, path, content length, and content is fed into SHA-256 under a versioned domain prefix.

## Supported reference

- `Beatblock.exe`: `c91d0853feb12aceb66a821eb5cdffb9c25acf69268bb2cf7451fa42f864de6b`
- `packed/obj.zip`: `e2e05a97902b879f2fc83442c36eb0abfab6d84d2373a8a3906d176227b1725f`
- `packed/states.zip`: `28bec969ddcd2f0a41cc5f0cf29dccab63c997cbb3010b5249bc37bc9a32a94f`

Additional builds are supplied through `SUPPORTED_GAME_BUILDS` after their patches pass the fixture validation suite.
