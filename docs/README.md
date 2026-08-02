# Beatblock Online documentation

Choose the path that matches what you are trying to do. Player instructions
avoid implementation details; technical pages explain the design, limits, and
validation behind those instructions.

## Contents

- [For players, spectators, and hosts](#for-players-spectators-and-hosts)
- [For technical readers and contributors](#for-technical-readers-and-contributors)
- [Find an answer quickly](#find-an-answer-quickly)
- [Release information](#release-information)

## For players, spectators, and hosts

Start with the **[Player Guide](player-guide.md)**. It covers:

- installing, updating, repairing, and uninstalling;
- joining as a Player or Spectator;
- chart matching, readiness, races, and results;
- hosting a room and preparing an internet connection;
- common installer, connection, chart, result, and OBS problems.

The guide tells you what to do in the released interface. Features that exist
only in a developer-facing backend are not presented as player controls.

## For technical readers and contributors

Start with the **[Technical Reference](technical-reference.md)**. It separates
current player-facing behavior, backend-only capabilities, known omissions,
and pending physical beta validation before indexing the specialist pages.

## Find an answer quickly

| Question                                     | Destination                                                                                           |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| How do I install?                            | [Install or update](player-guide.md#install-or-update)                                                |
| How do I join?                               | [Join as a Player](player-guide.md#join-as-a-player)                                                  |
| How do I host?                               | [Host a room](player-guide.md#host-a-room)                                                            |
| Why can’t I connect or ready up?             | [Troubleshooting](player-guide.md#troubleshooting)                                                    |
| How do I configure an event or UDP proxy?    | [Advanced hosting and room operations](operator-guide.md)                                             |
| Which Beatblock versions are supported?      | [Beatblock compatibility](compatibility.md)                                                           |
| How do I set up OBS?                         | [OBS setup](obs-setup.md)                                                                             |
| How is the mod installed?                    | [Installation and injection](injection.md)                                                            |
| How does the runtime or protocol work?       | [Architecture](architecture.md) and [protocol v3](protocol.md)                                        |
| How do I build, test, benchmark, or release? | [Contributor and release documentation](technical-reference.md#contributor-and-release-documentation) |
| How do I report a problem?                   | [Creating issues](issues.md)                                                                          |

## Release information

- [Current public release note](releases/v0.3.0.md)
- [Current technical changelog](changelogs/v0.3.0.md)
- [Complete release history](releases/index.md)
- [Beta.5 ship-readiness review](../reports/ship-readiness-review-v0.3.0-beta.5.md)

[Return to the project README](../README.md).
