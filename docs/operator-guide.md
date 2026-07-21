# Playing and hosting from Beatblock

There is no operator website, account system, cloud deployment, or container. The host controls the room from the adaptive in-game dashboard.

## Join a room as a Player

Ask the host for these three connection values:

- **Host Address**: the public IPv4 address or DNS name that receives room traffic.
- **UDP Port**: the public UDP port that receives room traffic. The default direct-host port is `32145`, but always use the value supplied by the host.
- **Password**: the room password. Receive it separately from the public address when possible.

When the host uses frp or another reverse proxy, **Host Address** is the public
address of the proxy server and **UDP Port** is the proxy's public
`remotePort`. Do not use the host PC's private address, the frp control
`serverPort`, or the proxy's `localPort` unless the host explicitly made the
public and local ports identical.

1. Install the same Beatblock Online release as the host, then launch Beatblock.
2. Open **Online** and wait for the header to show **ONLINE / PROTOCOL V3** and the session panel to show **READY TO CONNECT**.
3. Choose **Join as Player**.
4. Enter a display name. In **Host Address**, enter only the public IPv4 address or DNS name, such as `203.0.113.10` or `play.example.net`; do not include `http://`, `https://`, or a port.
5. Enter the public **UDP Port** and room **Password**, then choose **Join**.
6. In a host-approval room, **Waiting for Approval** is expected until the host accepts the request. In a password-only room, admission is immediate.
7. Follow the highlighted session action. When a chart is locked, select or accept the exact matching chart, choose **Ready**, and wait for the host to start the race.
8. Use **Room Options > Exit Online** when finished.

Choosing **Join as Spectator** follows the same connection procedure but does
not put the participant in the racing or readiness roster.

### Join through an frp public endpoint

Beatblock Online carries room traffic over QUIC on one UDP port. The frp proxy
must therefore use `type = "udp"`; a TCP, HTTP, or HTTPS proxy will not work.
For example, if `frpc` runs on the same PC as the Beatblock host:

```toml
# Relevant excerpt from frpc.toml. Keep your existing frps connection settings.
serverAddr = "203.0.113.10"
serverPort = 7000

[[proxies]]
name = "beatblock-online"
type = "udp"
localIP = "127.0.0.1"
localPort = 32145
remotePort = 42145
```

In this example, the host creates the room on UDP port `32145`, while Players
join with:

```text
Host Address: 203.0.113.10
UDP Port:     42145
Password:     <the room password>
```

`serverPort = 7000` is used by `frpc` to reach `frps`; it is not the port that
Players enter. If `frpc` runs on a different machine on the host's LAN, set
`localIP` to the Beatblock host PC's private IPv4 address instead of
`127.0.0.1`, and allow the configured room UDP port through the host PC's
private-profile firewall.

Before sharing the endpoint, the host should verify all of the following:

- The room is active and its configured UDP port equals the frp `localPort`.
- `frpc` reports that the UDP proxy is online.
- The `frps` host's operating-system firewall and cloud security group allow inbound UDP on `remotePort`.
- The frp server permits that remote port and the provider supports UDP proxies.
- Only the Beatblock room UDP port is exposed. The local runtime API and IPC endpoints are not public room services.

The FRP route replaces router UPnP/manual port forwarding for this connection
path, but all room traffic and host upload now pass through the FRP server.
Choose a nearby server with enough bandwidth and test from a device on a
different network before the event. See the
[official frp TCP/UDP proxy guide](https://gofrp.org/en/docs/features/tcp-udp/)
for current proxy configuration syntax.

### Player connection troubleshooting

| Symptom                                  | What to check                                                                                                                                                                                                                    |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Host address resolution fails            | Enter a valid public IPv4 address or DNS name in **Host Address**, without a URL scheme or port. Confirm the DNS name resolves on the Player's PC.                                                                               |
| Connection times out                     | Confirm the room is still active, the proxy type is UDP, the room port equals `localPort`, the Player entered `remotePort`, and inbound UDP on `remotePort` is allowed by both the FRP server firewall and cloud security group. |
| Password/authentication is rejected      | Re-enter the exact case-sensitive room password. Repeated failed attempts may require waiting before another attempt.                                                                                                            |
| **Waiting for Approval** remains visible | The network connection and password succeeded; the host still needs to accept the Player.                                                                                                                                        |
| Protocol is incompatible                 | Host and Player must install the same Beatblock Online release/protocol version.                                                                                                                                                 |
| **Locate Matching Chart** appears        | The room connection succeeded, but the local chart does not match. Select the exact official chart, locate the matching custom chart, or request the host transfer when offered.                                                 |

## Host a room

1. Open **Online** and choose **Host a Room**.
2. Enter a room name, UDP port, password, and display name. Choose **Play** to race or **Direct** to host without joining the race.
3. Share the `IP:port` or password-free `bbt://...?...v=3` link separately from the password.
4. Select pending roster rows to approve/reject requests. Select admitted rows to change roles or remove a participant.
5. The host's own roster row keeps **Direct Next Race** / **Play Next Race** available before later races, so the creation choice can be changed without rebuilding the room.
6. Follow **Select Chart**, build an optional setlist, and wait for every assigned player's exact verification.
7. When the dashboard reports all assigned players ready, choose **Start Race**. A directing host can start once at least one Player is assigned and ready.

The persistent header shows room name, runtime link, and lifecycle. The chart strip shows the locked chart and local verification. Above the eight-row roster, player, ready, and spectator totals stay visible even in a 16-player room.

## Charts and setlists

Open **Setlist** from the bottom utility bar.

- **Select Official** uses Beatblock's official selector and never redistributes official content.
- **Select Custom** locks a custom chart for local hash, variant, and note-count verification. When chart transfers are enabled, Players still search locally first and can then request the authenticated host fallback. Official charts remain local-only.
- **Add Official/Custom** creates an ordered set. Select a row, then use **Up**, **Down**, or **Remove** to edit its play order. After Results, the completed boundary is locked so an unplayed chart cannot be moved behind it and silently skipped.
- After Results, the host returns to Setlist. **Next Chart** / **Continue to Next Chart** locks the next entry and opens the matching official or custom selector so a playing host can verify it locally. If the set is complete, add another entry first.

Readiness requires the supported game and mod versions, selected variant, expected maximum hits, gameplay settings, allowed-mod inventory, and exact package hash. A player with a mismatch sees **Locate Matching Chart** rather than Ready.

## Admission, roles, and Force Start

Password-only rooms admit valid clients immediately. Host-approval rooms place them in the roster as **Pending** until accepted. Pending requests do not count toward player/spectator/ready totals.

Role changes and removals are disabled during countdown/gameplay. **Room Options > Force Start** deliberately bypasses readiness and asks for confirmation. Escape and controller pause inputs are ignored during online gameplay, while offline practice retains the native pause menu. Complete valid journals rank normally; missing, mismatched, paused, retried, incomplete, or disconnected runs become visible DNF and add `0.00` to the set total. Players stay in the room for later charts.

Host participation is also locked during countdown/gameplay. Directing keeps room and Broadcast authority but excludes the host from readiness, scoring, and renderer assignment; returning to Play clears stale readiness and requires fresh chart verification.

## Networking

The installer adds a program-scoped UDP firewall rule for the runtime. Private/domain profiles are the default; public-profile access is opt-in. If automatic port mapping fails, forward the configured UDP port manually. CGNAT requires a VPN or a different host because BBT has no relay service.

An automatically-created UPnP mapping is renewed only while its owning room is
active and is removed on a normal leave, close, or runtime shutdown. A crashed
runtime still relies on the router's finite two-hour lease.

For a normal player or spectator, budget 5 Mbps download and 1 Mbps upload. A full 16-player/32-spectator host should have at least 250 Mbps stable upload, low packet loss, and preferably wired Ethernet. The large host figure comes from sending each peer a complete room snapshot at up to 20 Hz: the current maximum-room schema models at about 160.6 Mbps of payload before transport overhead. An eight-participant room models near 5.8 Mbps of host payload, so 10 Mbps upload is a minimum for that size and 20 Mbps or more provides safer headroom. Streaming to Twitch, YouTube, or another service is additional.

Use **Settings** for link, peer, renderer-budget, and local API diagnostics. **Restart Runtime** is a one-retry recovery action and invalidates an active competitive run. **Help** provides state-specific guidance plus log/installer shortcuts.

## End the event

At Results, review the rankings, then use **Next Chart** to continue or **Select Next Chart** to extend/reorder a completed set. Match summaries remain in History until deletion; raw SQLite and NDJSON journals are automatically retained for 30 days by default. Runtime logs are capped at 14 days/64 MiB and chart-hash cache entries at 30 days/128 MiB. Choose **Room Options > Exit Online** after the event to close the room, flush storage, stop renderers/API/exports, and terminate the hidden runtime.
