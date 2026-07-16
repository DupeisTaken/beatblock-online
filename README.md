# Beatblock Together

Beatblock Together is a Windows direct-IP competition mod. One player hosts a password-protected room for up to 15 other players and 32 telemetry spectators. The host manages charts, readiness, starts, rankings, spectator streams, OBS exports, and match history from inside Beatblock.

## Player quick start

1. Download and run [BeatblockTogetherInstaller.exe](release/BeatblockTogetherInstaller.exe).
2. Confirm the **Selected game** card points to the folder you intend to modify. The Install page shows the adapter, OBS source, Windows Firewall profile, and uncertified-build choices together before anything changes. A green **SUPPORTED** badge identifies the certified build.
3. Choose **Install / Update** and follow the concrete phase shown above the progress bar. If progress pauses at the firewall phase, approve the native Windows administrator prompt on the secure desktop once. A result banner and one completion dialog report the final verified outcome; failures include the underlying Windows diagnostic.
4. Choose **Launch Beatblock** in the completion dialog to verify Lovely initialization, or close the installer and launch normally through Steam.
5. Open **Online**, then host a room or join with the host's `IP:port` and password.
6. Follow the dashboard's highlighted next action to select or verify the chart, ready players, and start the race. Select a roster row for participant details; the bottom utility bar opens Setlist, Spectate + OBS, History, and Settings without losing dashboard position.
7. Choose **Exit Online** when finished; the local runtime, API, exports, and renderers close with the Online session.

![Concentrated host dashboard](reports/trial-runs/dashboard-room-latest.png)

## Developer commands

Install Node workspace dependencies once with `pnpm install`. Rust must be available on `PATH`.

```text
pnpm generate:protocol  Regenerate protocol v2 schemas
pnpm test:mod           Package and validate both Lua adapters
pnpm test               Run protocol and Rust unit tests
pnpm test:stress        Run direct-room and export stress tests
pnpm trial              Run the complete acceptance and benchmark suite
pnpm build              Build the protocol and self-contained Windows installer
```

Development scripts operate on the repository and disposable `.test` copy. Players install and launch through the GUI and Steam.

## Detailed documentation

- [Installation, repair, adapters, and injection](docs/injection.md)
- [Adaptive Online dashboard and controls](docs/mod-guide.md)
- [Hosting, joining, setlists, and chart verification](docs/operator-guide.md)
- [OBS streams, local API, and text exports](docs/obs-setup.md)
- [Protocol v2](docs/protocol.md)
- [Installer/runtime architecture](docs/architecture.md)
- [Tests, benchmarks, and trial reports](docs/benchmarking.md)

The latest automated gate is [full-capability-latest.md](reports/trial-runs/full-capability-latest.md). The latest arbitrary-folder installer and Lovely recovery is [installer-reliability-latest.md](reports/trial-runs/installer-reliability-latest.md). UI captures are generated from the shipped Lua dashboard with Beatblock's reference fonts and 600x360 logical canvas.

## Components

- `mod`: shared Lua telemetry/gameplay core and mutually exclusive standalone Lovely and BeatblockPlus adapters.
- `companion`: feature-gated Rust installer and hidden runtime sharing installer, networking, SQLite, renderer, API, and export libraries.
- `obs-plugin`: native Stream A-D video source and shared-memory frame integration.
- `protocol`: protocol v2 JSON Schema and TypeScript conformance implementation.
- `reports/trial-runs`: machine-readable and Markdown acceptance evidence.

## Current alpha scope

Windows 10 2004+ and Windows 11 x64 are supported. A room supports 16 players, 32 telemetry spectators, four stable host renderer slots, password admission, optional host approval, synchronized starts, chart verification, authoritative event-derived rankings, SQLite history, and atomic OBS text exports.
