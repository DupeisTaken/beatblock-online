# Beatblock Together

Beatblock Together is an invite-only, self-hostable competition layer for Beatblock. The alpha synchronizes private races, verifies complete chart packages, recomputes live rankings on the server, and exposes telemetry to spectators, OBS, and other local applications.

This repository contains one shared Lua gameplay core with standalone Lovely and BeatblockPlus 2.x packages, a Windows Rust companion, a Fastify/PostgreSQL service, and a responsive React broadcast console.

## Quick start

Requirements for local development are Node.js 22+ and pnpm 10. Building the companion additionally requires the stable Rust toolchain and Windows SDK.

```powershell
pnpm install
pnpm build
pnpm test
pnpm package:mods
```

Install the standalone mod from a repository checkout with:

```powershell
.\scripts\install-mod.ps1 `
  -GameDir "C:\Program Files (x86)\Steam\steamapps\common\Beatblock" `
  -Distribution standalone `
  -LovelyArchive "$HOME\Downloads\lovely-x86_64-pc-windows-msvc.zip"
```

See [injection and installation paths](docs/injection.md) for the manual Lovely route, BeatblockPlus drag-and-drop route, isolated developer injection, exact folder trees, and recovery steps.

Run the maximum-capacity stress gate, performance benchmarks, or the complete demonstrator:

```powershell
pnpm test:stress
pnpm benchmark
pnpm trial
```

Run a memory-backed development instance and the web console:

```powershell
$env:ALLOW_INSECURE_HTTP='true'
pnpm dev
```

For self-hosting, copy `.env.example`, set unique secrets and a domain, then run:

```powershell
docker compose --env-file .env -f deploy/compose.yml up -d --build
docker compose --env-file .env -f deploy/compose.yml exec server node server/dist/cli.js invite-create --role organizer
```

## Packages

- `protocol` — protocol v1 schemas, scoring rules, and shared TypeScript contracts.
- `server` — HTTPS/WSS API, invites, sessions, lobbies, authoritative scoring, persistence, and `bbtctl`.
- `web` — spectator console plus four OBS layouts.
- `companion` — loopback API, IPC bridge, event journal, chart hashing, credential vault, tray, and atomic exports.
- `mod` — shared hooks and two mutually exclusive package bootstraps.
- `deploy` — PostgreSQL/Caddy Docker deployment.

See [injection and installation](docs/injection.md), [mod and player guide](docs/mod-guide.md), [architecture](docs/architecture.md), [operator guide](docs/operator-guide.md), [OBS setup](docs/obs-setup.md), [protocol](docs/protocol.md), and [benchmark/trial guide](docs/benchmarking.md).

## Alpha boundaries

Spectating is telemetry-based; gameplay video still comes from game capture. The alpha does not include public signup, matchmaking, chat, chart distribution, replay rendering, global ratings, or a claim of cheat-proof results.

Remote production instances require HTTPS. Invite and refresh credentials are hashed at rest, refresh tokens rotate, Windows credentials live in Credential Manager, browser handoffs expire after 60 seconds, and local APIs bind only to `127.0.0.1` with a per-install token.
