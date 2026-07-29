# Beatblock Online Player Guide

This is the public guide for Players, Spectators, and first-time hosts. Follow
the labels shown inside the installer and Beatblock; no command line is needed.

> **Current beta:** Beatblock Online `v0.3.0-beta.5` is tested with Beatblock
> `1.7.1a (Early Access)[d40b7083]` on Windows 10 2004+ and Windows 11 x64.
> Newer Beatblock builds may install but remain unverified. See
> [Before you start](#before-you-start).

## Contents

- [Before you start](#before-you-start)
- [Install or update](#install-or-update)
- [Join as a Player](#join-as-a-player)
- [Play a race](#play-a-race)
- [Join as a Spectator](#join-as-a-spectator)
- [Host a room](#host-a-room)
- [Run more than one chart](#run-more-than-one-chart)
- [Finish or leave a session](#finish-or-leave-a-session)
- [Repair or uninstall](#repair-or-uninstall)
- [Troubleshooting](#troubleshooting)
- [Report a problem](#report-a-problem)

## Before you start

Everyone in a room should use:

- the same Beatblock Online release; and
- the same Beatblock build unless the host deliberately chooses
  **SAME BUILD: ALLOW ANY** for casual compatibility testing.

Ask the host for three separate values:

| Value            | Example                              | Important                                                                |
| ---------------- | ------------------------------------ | ------------------------------------------------------------------------ |
| **Host Address** | `203.0.113.10` or `play.example.net` | Do not add `http://`, `https://`, or the port.                           |
| **UDP Port**     | `32145`                              | Use the value supplied by the host, even if it differs from the default. |
| **Password**     | supplied privately                   | Passwords are case-sensitive.                                            |

Beatblock Online has no accounts, matchmaking, public room list, or hosted
relay. A host must make one UDP port reachable. Players and Spectators do not
normally need to change their router.

Because this is a beta prerelease:

- Windows may show **Unknown publisher** for the unsigned installer. Download
  only from the project’s [GitHub Releases page](https://github.com/DupeisTaken/beatblock-online/releases).
- Broadcast and OBS integration are available, but some physical OBS 32.1.2
  audio, color, and long-run checks remain open. Test the complete scene before
  a live event.
- A newer Beatblock version being accepted by the installer does not mean it
  has been verified. Check the [compatibility page](compatibility.md) if the
  displayed build differs from the tested version above.

## Install or update

1. Close Beatblock. Also close OBS if you intend to install or update the
   optional OBS source.
2. Open the [newest GitHub Release](https://github.com/DupeisTaken/beatblock-online/releases)
   and download `BeatblockOnlineInstaller.exe`.
3. Run the installer. Confirm the **Selected game** card names the Beatblock
   folder you actually use. A green **COMPATIBLE** badge means the folder has
   the required game files; it does not certify an untested future game build.
4. Leave **Automatic** selected unless you already know that you need the
   standalone or BeatblockPlus adapter.
5. Select the optional OBS component only if you use OBS and the detected OBS
   folder is correct. Choose whether the firewall rule should cover normal
   Private networks only or Public networks too.
6. Choose **Install / Update**. Follow the phase above the progress bar. If
   Windows opens an administrator prompt during a protected-file or firewall
   phase, approve that one prompt and return to the installer.
7. Wait for the verified success result, then choose **Launch Beatblock**.
8. In Beatblock, open **Online**. Wait until the top-right status shows
   `v0.3.0-beta.5 / READY` and the session panel says **READY TO CONNECT**.

When an installer update is offered, **Update Installer** updates the managed
maintenance copy of the installer. It does not update the Beatblock mod by
itself; after the new installer opens, use **Install / Update**.

## Join as a Player

1. Open **Online** and wait for **READY TO CONNECT**.
2. Choose **Join as Player**.
3. Enter a display name. It must differ from names already reserved in the
   room; capitalization alone does not make it unique.
4. Enter the **Host Address**, **UDP Port**, and case-sensitive **Password**
   supplied by the host, then choose **Join**.
5. **Waiting for Approval** means the connection and password succeeded. Wait
   for the host to select your roster entry and approve it.
6. Follow the highlighted action in the session strip. If a chart is already
   locked, Beatblock Online will ask you to find or accept the exact chart.

Do not paste an `IP:port` pair or `bbt://` link into **Host Address**. The
current in-game form accepts an address and UDP port in separate fields.

## Play a race

1. When the host locks a chart, select the matching official chart or the exact
   matching custom chart and variant.
2. If the host offers a custom chart, review the package before accepting it.
   Script or executable content always gets a separate warning. Official charts
   are never transferred.
3. When the session strip offers **Ready**, choose it and wait. If you change
   your mind before the race, select your own roster row and choose **Unready**.
4. The host can choose **Start Race** only after all assigned Players are
   verified and ready. Beatblock opens the locked chart after the countdown.
5. Finish the attempt and return to the Online room for **Current Results**.
   Select a participant and use **Run Details** when a result is **INVALID** or
   **DNF**.

In an online race, normal pause inputs are intentionally unavailable. A
competitive room can mark a retried or incomplete attempt invalid; a casual
room permits retries but still marks incomplete attempts and expired
disconnects as DNF.

## Join as a Spectator

Choose **Join as Spectator** and use the same address, port, password, and host
approval steps. Spectators follow room state and results but do not select,
ready, or play the chart.

A host may grant a Spectator **Commentator** access. That permission makes the
host’s Broadcast plan visible. Enabling **This PC** is optional and may start
up to four additional Beatblock renderer processes, so use it only on a
machine prepared for OBS reconstruction. See [OBS setup](obs-setup.md).

## Host a room

### Prepare the connection

For players on the internet, the chosen UDP port must reach the host PC.

1. Install Beatblock Online with the appropriate Windows Firewall profile.
2. Prefer wired Ethernet and choose one UDP port, normally `32145`.
3. Let the router’s automatic port mapping run. If players time out, manually
   forward the same UDP port to the host PC.
4. If the host connection uses CGNAT, ordinary port forwarding will not work.
   Use a VPN with inbound UDP, an advanced UDP proxy, or a different host
   connection. The project does not provide a relay.
5. Test from a different network before the event.

For a LAN-only room, Players can use the host PC’s private IPv4 address and do
not need router port forwarding. Advanced hosts using frp should follow the
[UDP proxy procedure](operator-guide.md#host-through-an-frp-public-endpoint).

### Create and operate the room

1. Open **Online**, wait for **READY TO CONNECT**, and choose **Host a Room**.
2. Enter your display name, room name, UDP port, and password.
3. Choose **Play** to compete or **Direct** to operate the room without racing.
4. Keep **Run Checks: On** for competitive scoring. Choose **Off** only for a
   casual room where retries are allowed.
5. Keep **Same Build: Require** for normal events. **Allow Any** is intended
   only for casual compatibility testing.
6. Choose **Create**. The current in-game flow always asks the host to approve
   each joining Player or Spectator.
7. Share the address, UDP port, and password. Send the password separately when
   practical.
8. Select each pending roster entry and choose **Approve** or **Reject**. After
   admission, the same inspector can change a Player to a Spectator, grant
   Commentator access to a Spectator, or remove someone.
9. Use **Select Chart**, choose **Official Chart** or **Custom Chart**, and
   select the chart in Beatblock.
10. Wait until every assigned Player is verified and ready, then choose
    **Start Race**.

There is no Force Start control in the current in-game dashboard. Resolve
readiness problems before starting. The runtime has a developer/API force
option, but it is not part of the supported player workflow.

## Run more than one chart

Open **Setlist** after creating the room:

1. Use **Add Official** or **Add Custom** to build the ordered queue.
2. Select a future row to **Move Up**, **Move Down**, or **Remove** it.
   Completed and current entries are protected.
3. Setlist editing is locked during countdown and gameplay.
4. At Results, use the highlighted **Next Chart** action. The host is returned
   to the correct Beatblock selector to verify the next entry locally.

The normal **Select Chart** action is for one chart. Choosing it while a setlist
exists asks before replacing the entire ordered set.

## Finish or leave a session

- To leave a room but keep Online open, select your own roster row and choose
  **Leave Room**.
- The host selects their own roster row and chooses **Close Room** to close the
  room for everyone.
- Press Back from the Room workspace, or choose **Exit Online** from the
  connection screen, to confirm shutdown and return to Beatblock’s main menu.

Exiting Online stops the local room services, exports, API, and renderer
processes. Match summaries remain in **History** unless removed later.

## Repair or uninstall

Open `BeatblockOnlineInstaller.exe` again:

- Use **Components** to find an item marked **REPAIR REQUIRED**, then choose
  **Repair Required Components**.
- Use **Repair** for a full verification and repair of managed files.
- Use **Uninstall** to remove Beatblock Online and restore installer-managed
  game files. Settings and match history are preserved by default; select
  **Also remove settings, API credentials, and match history** in the
  confirmation only if you want those removed too.

Do not manually delete `version.dll` when another Lovely mod may depend on it.
Let the installer restore the backup it recorded.

## Troubleshooting

### Installer or launch problems

| What you see                                                       | What to do                                                                                                                                                               |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| The selected game is not **COMPATIBLE**                            | Select the folder containing `Beatblock.exe`, not a shortcut or parent Steam folder. If Steam files are incomplete, verify Beatblock in Steam and refresh the installer. |
| Install/update controls are disabled                               | Close Beatblock. If the OBS option is selected, close OBS too. Choose **Refresh game & OBS status**.                                                                     |
| Progress appears paused during the firewall phase                  | Look for a Windows administrator prompt on the secure desktop, approve it once, and let the visible installer finish.                                                    |
| Windows shows **Unknown publisher**                                | This beta is unsigned. Confirm the file came from the project’s GitHub Release before continuing.                                                                        |
| Beatblock opens but **Online** is missing or reports damaged files | Open the installer, check **Components**, and run **Repair Required Components**.                                                                                        |
| Beatblock closes immediately and no Lovely log is created          | Repair once. If it repeats, follow the [Windows Code Integrity check](injection.md#developertest-path); security policy may be blocking `version.dll`.                   |
| **Update Installer** is unavailable while running as administrator | Close it and reopen the installer normally. Self-update intentionally does not run elevated.                                                                             |

### Connection problems

| What you see                                     | What to do                                                                                                                                                                     |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status stays **STARTING** or becomes **OFFLINE** | Close and reopen Online. If it repeats, open the installer and Repair.                                                                                                         |
| Host address resolution fails                    | Enter only a valid IPv4 address or DNS name. Put the UDP port in its own field.                                                                                                |
| Connection times out                             | Confirm the host’s room is still open and the port is correct. The host should recheck the Windows Firewall profile and UDP router forwarding, then test from another network. |
| The host uses frp or another proxy               | Enter the proxy’s public address and public UDP `remotePort`, not its control port or local port.                                                                              |
| Password is rejected                             | Re-enter the exact case-sensitive password. After repeated failures, wait briefly before retrying.                                                                             |
| **username taken**                               | Choose a genuinely different display name; letter casing alone does not count.                                                                                                 |
| **Waiting for Approval** does not change         | Ask the host to select your entry in the **Pending** roster filter and approve it.                                                                                             |
| Protocol is incompatible                         | Host and participant must install the same Beatblock Online release.                                                                                                           |
| Beatblock build mismatch                         | Install the same Beatblock build as the host. For casual testing only, the host can create a new room with **Same Build: Allow Any**.                                          |
| Online reports **RECONNECTING**                  | Keep Online open during the 30-second reconnect window. If it returns offline, join the room again.                                                                            |

### Chart, readiness, and result problems

| What you see                                         | What to do                                                                                                                                                     |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Find Matching Chart** or **Locate Matching Chart** | Select the exact official chart, or the exact custom package and variant chosen by the host.                                                                   |
| A custom chart still does not match                  | Use **Request Host Transfer** when offered, or obtain the exact package from the host. Review every transfer warning before accepting.                         |
| **Ready** never appears                              | Open your participant inspector. Confirm that the chart is verified and that you are a Player, not a Spectator.                                                |
| The host cannot choose **Start Race**                | Every assigned Player must verify the chart and choose Ready. The current dashboard has no Force Start control.                                                |
| Result is **INVALID**                                | Select the participant and open **Run Details**. In competitive rooms, retries, integrity failures, or missing ordered score events can invalidate an attempt. |
| Result is **DNF**                                    | Open **Run Details**. DNF means the attempt did not complete, timed out, quit, or outlived the reconnect period.                                               |
| **Online IPC is overloaded**                         | Do not continue as though scoring is current. Return to Online and reconnect before another competitive attempt.                                               |

### OBS problems

| What you see                         | What to do                                                                                                                  |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| Beatblock Online sources are missing | Close OBS, reopen the installer, enable the OBS component for the correct OBS folder, install/repair, then restart OBS.     |
| Video works but there is no audio    | Player Stream is video-only. Add the matching **Beatblock Online Audio A–D** source or the optional Autoplay audio source.  |
| Renderer audio is heard twice        | Keep one intended OBS audio route, check that the source is not duplicated, and review **Desktop Mute** in Online Settings. |

For complete source, color, audio, and performance checks, use the
[OBS troubleshooting guide](obs-setup.md#diagnose-a-missing-source).

## Report a problem

If the tables above do not solve the issue:

1. Update to the newest Beatblock Online release, or note why you cannot.
2. Reproduce one problem and record the exact Beatblock Online version,
   Beatblock version/build shown in the top-right corner, role, and visible
   error.
3. Use **Settings > Logs** when Online is running, or the installer’s **Log**
   tab. Include only a short relevant excerpt or screenshot.
4. Remove passwords, public addresses, usernames, Windows account names,
   filesystem paths, and tokens.
5. Follow [Creating issues](issues.md) and choose the matching
   [guided issue form](https://github.com/DupeisTaken/beatblock-online/issues/new/choose).

For implementation details, current validation boundaries, and all specialist
documents, continue to the [Technical Reference](technical-reference.md).
