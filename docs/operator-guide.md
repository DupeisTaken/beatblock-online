# Advanced hosting and room operations

> Documentation: [Player Guide](player-guide.md) · [Technical Reference](technical-reference.md)

This guide covers event operation beyond the public walkthrough. The host
controls the room from Beatblock’s in-game Online dashboard; there is no
operator website or hosted control plane.

## Contents

- [Create a room from the in-game dashboard](#create-a-room-from-the-in-game-dashboard)
- [Admission, roles, and participant control](#admission-roles-and-participant-control)
- [Charts and setlists](#charts-and-setlists)
- [Starting, scoring, and results](#starting-scoring-and-results)
- [Direct UDP networking](#direct-udp-networking)
- [Host through an frp public endpoint](#host-through-an-frp-public-endpoint)
- [End the event](#end-the-event)
- [Runtime and API-only capabilities](#runtime-and-api-only-capabilities)

## Create a room from the in-game dashboard

Open **Online**, wait for `READY TO CONNECT`, and choose **Host a Room**. The
current form exposes:

| Setting                             | Player-facing behavior                                                                                                                                                                   |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Host Role: Play / Direct**        | Play assigns the host as a Player. Direct keeps host authority while excluding the host from readiness and scoring.                                                                      |
| **Run Checks: On / Off**            | On is competitive. Off is casual: retries are allowed and missing ordered score events may recover at the next cumulative update. Both modes still validate counters and DNF completion. |
| **Same Build: Require / Allow Any** | Require is the safe default. Allow Any is for casual compatibility testing and cannot be made strict again after the room is active; create a new room to restore strict matching.       |
| UDP port and password               | One UDP port carries room traffic. The password is required and case-sensitive.                                                                                                          |

The in-game form currently creates a host-approval room, enables custom-chart
transfer, and starts with transfer requests set to manual. After creation,
**Settings > Requests** switches between manual and automatic requests.
Automatic requests do not bypass the receiving Player’s consent.

Open **Settings > Modifiers** after creating the room to inspect or change the
complete chart policy. The host can set game speed (0.5x-5.0x), VFX
(Full/Decreased/None), Taps (Default/Lenient/Strict/Auto), Sides
(Default/Lenient/Auto), Barelies (Default/Lenient/Strict), and Restart On
(None/Miss/Barely). Changes lock when countdown begins. All Players, including
the host, run the authoritative choices during Game and Results; their saved
local preferences are restored when they return to Online. Non-hosts can open
the same panel read-only.

Modifier enforcement is a room-authentication capability even though the wire
envelopes remain protocol v3. A current host rejects an older v3 runtime that
would ignore the policy. If a participant sees an update requirement while
joining, update Beatblock Online on every machine before recreating the room.
With **Restart On: Miss/Barely**, competitive Run Checks still treat a restarted
attempt as invalid; turn Run Checks off only when intentional retries should
replace earlier attempts.

The host’s own roster inspector provides **Direct Next Race** or
**Play Next Race** before a later race. Changing participation clears stale
readiness and requires fresh verification where applicable.

## Admission, roles, and participant control

New connections appear under the **Pending** roster filter. Select a stable
participant row to approve or reject it. After admission, the inspector can:

- switch a Player to Spectator or a Spectator to Player;
- grant or revoke Commentator on a Spectator; or
- remove a participant.

Role changes and removal are disabled during countdown and gameplay. Rejection
remains available for a still-pending request.

The terms are distinct:

- **Host** owns room and Broadcast authority and may Play or Direct.
- **Player** competes and must verify and ready the locked chart.
- **Spectator** follows room state and results without competing.
- **Commentator** is an additional host-granted permission on a Spectator; it
  does not create a Player or change room capacity.

## Charts and setlists

### One chart

Use the highlighted **Select Chart** action and choose **Official Chart** or
**Custom Chart**. Official content is never redistributed. A custom chart is
locked by its package hash, variant, and note count.

If a setlist already exists, one-off selection asks before replacing the
ordered queue.

### Ordered set

Open **Setlist** and use **Add Official** or **Add Custom**. Six visible rows
show order, chart, variant, and **Now**, **Next**, **Queued**, or **Done**.
Select a future entry to move or remove it. The active and completed boundary
is protected, and all editing locks during countdown/gameplay.

At Results, **Next Chart** is the authoritative continuation control. It
advances the queue and opens the corresponding Beatblock selector so a playing
host can verify the newly active chart locally.

### Custom-chart fallback

Players search their own charts first. If the exact custom package is missing,
they may request the host copy. The offer names its size and whether script or
executable content exists. Normal packages can be accepted once or trusted for
that live room; script/executable content always requires separate consent and
cannot inherit room trust.

Accepted packages are isolated in Beatblock Online’s transfer cache rather
than added to the Player’s normal Custom Levels library. See
[chart matching and host fallback](mod-guide.md#chart-matching-and-host-fallback)
for validation and resource limits.

## Starting, scoring, and results

The in-game **Start Race** action becomes available only when every assigned
Player is admitted, verified, and ready. A directing host also needs at least
one assigned ready Player.

The current dashboard does not expose Force Start. It always sends a normal
start request. Resolve each readiness or chart mismatch before beginning; see
[player troubleshooting](player-guide.md#chart-readiness-and-result-problems).

Complete valid attempts rank normally. With **Run Checks: On**, retries,
integrity failures, and unrecoverable ordered-event gaps become **INVALID**.
Explicit quits, incomplete attempts, launch timeouts, and expired disconnects
become **DNF** in either mode. INVALID and DNF contribute `0.00` to the set
total, and the Player remains available for later charts. Select the
participant at Results and choose **Run Details** for the authoritative reason.

## Direct UDP networking

The installer adds a program-scoped UDP firewall rule for the runtime.
Private/domain profiles are the default; Public is opt-in. The runtime attempts
UPnP mapping while a room owns the port. If it fails, forward the configured
UDP port manually to the host PC.

Beatblock Online does not include a relay. A host behind CGNAT needs a VPN with
inbound UDP, a UDP proxy such as frp, or a different connection. LAN Players
can use the host’s private IPv4 address; internet Players use the public
IPv4/DNS endpoint and public UDP port.

For normal participants, plan for 5 Mbps download and 1 Mbps upload. Host
requirements scale with participant count; a small eight-participant event
should treat 10 Mbps upload as a minimum, while the maximum 48-participant room
has a conservative 250 Mbps recommendation. Streaming upload is additional.
See [measured and calculated budgets](benchmarking.md#recommended-system-and-network-specifications)
instead of duplicating the formulas here.

## Host through an frp public endpoint

Beatblock Online room traffic requires a UDP proxy. If `frpc` runs on the
Beatblock host PC, the relevant portion of `frpc.toml` is:

```toml
# Keep the existing frps connection settings.
serverAddr = "203.0.113.10"
serverPort = 7000

[[proxies]]
name = "beatblock-online"
type = "udp"
localIP = "127.0.0.1"
localPort = 32145
remotePort = 42145
```

The host creates the room on UDP `32145`. Participants enter:

```text
Host Address: 203.0.113.10
UDP Port:     42145
Password:     <the room password>
```

`serverPort = 7000` is the frp control connection and is not entered by
Players. If `frpc` runs elsewhere on the host LAN, set `localIP` to the
Beatblock host PC’s private IPv4 address and allow the room UDP port through
that PC’s Private firewall profile.

Before sharing the endpoint, verify:

- the active room port equals `localPort`;
- `frpc` reports the UDP proxy online;
- the frps operating-system firewall and cloud security group permit inbound
  UDP on `remotePort`;
- the server permits that remote port and supports UDP proxies; and
- only the room UDP port is exposed. The local API and game IPC are not public
  room services.

All host room traffic passes through the proxy. Choose a nearby server with
enough bandwidth and test from another network. Consult the
[official frp TCP/UDP proxy guide](https://gofrp.org/en/docs/features/tcp-udp/)
for current frp syntax.

## End the event

At Results, use **Next Chart** to continue the ordered set. To end:

1. Review Results and History.
2. Select the host’s own roster row and choose **Close Room**.
3. From the Room workspace, press Back and confirm **Exit Online**.

Closing the room disconnects participants. Exiting Online flushes storage and
stops local exports, API, renderers, and the hidden runtime. Match summaries
remain until deleted. Raw journals retain 30 days by default; runtime logs and
chart-hash cache have their own bounded retention.

## Runtime and API-only capabilities

The following are implemented below the in-game dashboard but are not current
player controls:

- **Password-only admission:** supported by the runtime and bearer-protected
  local API; the in-game host form always requests host approval.
- **Force Start:** supported by protocol/runtime/API; the in-game dashboard
  sends `force=false` and has no override control.
- **`bbt://` join URI:** generated by the runtime/API without the password; the
  in-game Connect form still requires separate address and UDP port fields.

Treat these as integration surfaces, not as released dashboard features. The
local API is documented under [Third-party local API](obs-setup.md#third-party-local-api),
and wire behavior is documented in [protocol v3](protocol.md).
