# Hosting from Beatblock

There is no operator website, account system, cloud deployment, or container. The host controls the room from the adaptive in-game dashboard.

## Host a room

1. Open **Online** and choose **Host a Room**.
2. Enter a room name, UDP port, password, display name, and admission policy.
3. Share the `IP:port` or password-free `bbt://...?...v=2` link separately from the password.
4. Select pending roster rows to approve/reject requests. Select admitted rows to change roles or remove a participant.
5. Follow **Select Chart**, build an optional setlist, and wait for every assigned player's exact verification.
6. When the dashboard reports all assigned players ready, choose **Start Race**.

The persistent header shows room name, runtime link, and lifecycle. The chart strip shows the locked chart and local verification. Above the eight-row roster, player, ready, and spectator totals stay visible even in a 16-player room.

## Charts and setlists

Open **Setlist** from the bottom utility bar.

- **Select Freeplay Chart** uses Beatblock's Freeplay list for official charts and never redistributes official content.
- **Select Custom** locks a custom chart for local hash, variant, and note-count verification. Host file transfer is intentionally unavailable in this release; every competitor installs the chart locally.
- **Add Official/Custom** creates an ordered set. Move or remove the selected entry before it is played.
- After Results, **Advance Setlist** locks the next entry and returns everyone to verification.

Readiness requires the supported game and mod versions, selected variant, expected maximum hits, gameplay settings, allowed-mod inventory, and exact package hash. A player with a mismatch sees **Locate Matching Chart** rather than Ready.

## Admission, roles, and Force Start

Password-only rooms admit valid clients immediately. Host-approval rooms place them in the roster as **Pending** until accepted. Pending requests do not count toward player/spectator/ready totals.

Role changes and removals are disabled during countdown/gameplay. **Room Options > Force Start** deliberately bypasses readiness and asks for confirmation. Complete valid journals rank normally; missing, mismatched, paused, retried, incomplete, or disconnected runs become visible DNF and add `0.00` to the set total. Players stay in the room for later charts.

## Networking

The installer adds a program-scoped UDP firewall rule for the runtime. Private/domain profiles are the default; public-profile access is opt-in. If automatic port mapping fails, forward the configured UDP port manually. CGNAT requires a VPN or a different host because BBT has no relay service.

An automatically-created UPnP mapping is renewed only while its owning room is
active and is removed on a normal leave, close, or runtime shutdown. A crashed
runtime still relies on the router's finite two-hour lease.

For a normal player or spectator, budget 5 Mbps download and 1 Mbps upload. A full 16-player/32-spectator host should have at least 250 Mbps stable upload, low packet loss, and preferably wired Ethernet. The large host figure comes from sending each peer a complete room snapshot at up to 20 Hz: the current maximum-room schema models at about 160.6 Mbps of payload before transport overhead. An eight-participant room models near 5.8 Mbps of host payload, so 10 Mbps upload is a minimum for that size and 20 Mbps or more provides safer headroom. Streaming to Twitch, YouTube, or another service is additional.

Use **Settings** for link, peer, renderer-budget, and local API diagnostics. **Restart Runtime** is a one-retry recovery action and invalidates an active competitive run. **Help** provides state-specific guidance plus log/installer shortcuts.

## End the event

At Results, review the rankings or advance the set. Match summaries remain in History until deletion; raw SQLite and NDJSON journals are automatically retained for 30 days by default. Runtime logs are capped at 14 days/64 MiB and chart-hash cache entries at 30 days/128 MiB. Choose **Room Options > Exit Online** after the event to close the room, flush storage, stop renderers/API/exports, and terminate the hidden runtime.
