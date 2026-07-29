# Beatblock Online

Beatblock Online is a Windows direct-IP competition mod for Beatblock. Host a
password-protected room, invite Players and Spectators, run single charts or an
ordered set, and review synchronized results without leaving Beatblock.

> **Beta prerelease:** the current release is
> [`v0.3.0-beta.5`](docs/releases/v0.3.0-beta.5.md), tested with Beatblock
> `1.7.1a (Early Access)[d40b7083]`. Newer Beatblock builds may install, but
> remain unverified until tested. The installer is currently unsigned, so
> Windows may show **Unknown publisher**.

## Start here

| I want to…                                                                        | Read…                                                  |
| --------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Install, join, play, spectate, host, or fix a common problem                      | **[Player Guide](docs/player-guide.md)**               |
| Understand compatibility, networking, protocol, installation, OBS, or development | **[Technical Reference](docs/technical-reference.md)** |
| Browse everything by topic                                                        | [Documentation home](docs/README.md)                   |

The former README quickstart now lives in the Player Guide, where each task has
its own steps and nearby troubleshooting.

## What the current beta includes

- Direct-IP rooms for up to 16 Players and 32 Spectators.
- Password authentication and host approval from the in-game host flow.
- Playing or directing hosts, exact chart checks, competitive or casual run
  checks, ordered setlists, synchronized starts, rankings, and match history.
- Optional custom-chart transfer, four host-controlled Broadcast slots,
  Commentator access, OBS video/audio sources, and text exports.

Beatblock Online does **not** provide accounts, matchmaking, a public room
browser, or a hosted relay. Internet hosts need a reachable UDP port; players
behind CGNAT may still join, but a host behind CGNAT needs a VPN, UDP proxy, or
different host connection. See [feature status and boundaries](docs/technical-reference.md#feature-status-and-boundaries)
for controls that are implemented only in the runtime/API and for remaining
physical beta validation.

![Concentrated host dashboard](tests/ui-baselines/host-lobby.png)

## Download

Download `BeatblockOnlineInstaller.exe` from the
[GitHub Releases page](https://github.com/DupeisTaken/beatblock-online/releases).
Most people do not need the separate mod ZIPs or OBS DLL. Follow the
[installation walkthrough](docs/player-guide.md#install-or-update) before
starting Beatblock.

## Get help or report a problem

Start with [Troubleshooting](docs/player-guide.md#troubleshooting). If the
problem remains, read [Creating issues](docs/issues.md) and choose the matching
[guided GitHub issue form](https://github.com/DupeisTaken/beatblock-online/issues/new/choose).
Remove passwords, public addresses, usernames, tokens, and personal filesystem
paths from logs and screenshots before posting them.

Developer commands, component descriptions, benchmarks, release procedures,
and validation reports are indexed in the
[Technical Reference](docs/technical-reference.md).
