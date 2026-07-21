# Online shell and room roles

Opening **Online** lazily starts the hidden runtime. The interface always uses Beatblock's `600×360` logical canvas and shows one workspace at a time. Only forms, destructive confirmations, transfer consent, and full error details use a modal.

The header identifies the runtime and protocol. Beneath it, the pinned session strip shows the current room/chart, verification context, and exactly one next action: host, select chart, locate chart, ready, start, wait, next chart, select next chart, or view results. Button labels use measured font metrics and every interactive control is at least 22 logical pixels high. Visually disabled controls have no activation callback, so mouse, keyboard, and controller cannot bypass lifecycle locks.

Online resets Beatblock's fixed menu palette when it opens. Muted text uses the
palette's neutral-gray source index, and modals use an opaque black focus veil
instead of translucent colors; arbitrary RGB or alpha colors are invalid under
Beatblock's palette shader and can appear purple. Leaving Online restores the
native full-color menu shader before the destination state draws, including
transitions that reuse an already-loaded Menu state. Online also clears the
native radial-menu entities when it opens and exits, so a retained `Player`
instance cannot keep updating beside the next state's player. It reapplies
Beatblock's menu font on entry, draw, and exit so neither icon art nor stale
font metrics can leak across states.

Before joining a room, **Host a Room** appears only as the session strip's next
action. The Connect workspace separates **Player** and **Spectator** choices,
explains their scoring behavior, and keeps Exit Online visually secondary.

## Room workspace

The roster has **All**, **Players**, **Spectators**, and **Pending** filters. Selection is stored by `sessionId`, so sorting, reconnects, filter changes, and workspace changes cannot silently target a different person. The participant inspector remains visible beside the roster and contains role, connection, chart, run state, and only the actions permitted to the local user.

Before play the roster shows readiness/verification state and never fabricates a `100.00` score. During play and Current Results it shows separate **Rank** and **Accuracy** columns. Missing values render as `—`; invalid outcomes render as **DNF** or **INVALID**, with the runtime's reason available through **Run Details**. Current Results is a room phase, including cumulative set total where applicable. **History** is only the archive.

The terms are intentionally distinct:

- **Host** owns room and Broadcast authority and can either Play or Direct the next race.
- **Player** competes and must locally verify the locked chart.
- **Spectator** follows room state and rankings without competing.
- **Commentator** is a host-granted permission layered on an admitted Spectator. It does not change room capacity or make that participant a Player.

The host grants or revokes Commentator from the participant inspector. A role change, removal, revocation, room exit, or shutdown clears the grant's active subscriptions and stale local renderer state.

## Global workspaces

- **Setlist** selects official/custom charts, builds the ordered room set, and exposes Up/Down/Remove for the selected row. Results return the host here; advancing opens the correct selector for local verification.
- **Broadcast** owns the host's four Stream A–D assignments. The default view shows candidate, assignment, Feature, Stop, and local health; **Advanced Export** edits mode, resolution, FPS, and delay per stream and warns on 1080p60.
- **History** lists saved match summaries.
- **Settings** controls the gameplay HUD and the host's pre-race **Run Checks** policy, shows protocol/runtime details and the isolated transfer-cache size, and provides **Clear Cache**. Competitive checks remain the default; casual mode permits retries and ordered-event recovery while retaining structural score validation and DNF completion rules.
- **Help** explains roles, controls, chart transfer, and player troubleshooting without granting room controls. Runtime errors remain visible in the footer, while complete INVALID/DNF reasons live in **Run Details**.

Back closes one modal or returns one workspace. Keyboard, controller, and mouse update the same focus model. In host and join forms, text follows the active keyboard layout; Backspace or Delete removes the final complete character, including Unicode input.

Returning from chart selection or cancelling it keeps the Setlist workspace
open, so hosts can add several charts without reopening the tool each time.

## Chart matching and host fallback

For every locked chart, Online tries the current selection and managed hash/path indexes before asking the Player. Official charts are local-only. For a custom chart, **Select Local Chart** verifies the canonical package hash, variant, and note count. If the host enabled transfers, **Request Host Transfer** requests the original archive (or a bounded archive produced from the host's selected directory).

The offer shows size and whether script/executable content exists. Normal packages can be accepted once or with **Trust This Room**, which auto-accepts later ordinary packages only for that live room. Script/executable content always requires a separate confirmation and never inherits room trust.

Transfers use an authenticated QUIC file stream, one stream at a time per peer, with backpressure, a 30-second stall timeout, a 120-second send timeout, cancellation on disconnect, and a 1 GiB limit. The runtime validates archive SHA-256, traversal, links, entry count, expanded size, executable content, and the final canonical chart hash.

Accepted content is extracted by hash under BBT's read-only Online cache, not the user's Custom Levels library. The cache has a 2 GiB LRU budget and never evicts the active chart.

## Commentator Broadcast mirror

The host owns the authoritative revisioned **Host Plan**. A granted Commentator sees it read-only and must explicitly enable **This PC** after a performance warning. This may start up to four hidden Beatblock renderer children. If the chart is missing, assignments and text remain visible but video stays disabled until local matching or an accepted host transfer succeeds.

Only active assigned Player telemetry is relayed, and only to authorized Commentators that enabled mirroring. Stable protocol-v3 render-source IDs map remote samples into local Stream A–D frame rings. Featured text exports and video use the same delayed clock. Only the featured renderer receives audio; the previous featured child is stopped before the new one starts audibly.

Long renderer failures are bounded in the workspace. **Details** opens the full message without hiding slot controls. See [OBS setup](obs-setup.md) for source and application-audio configuration.

## Screenshot verification

`pnpm test:ui` stages the tracked harness in a `bbt-ui-*` directory under `%TEMP%`, uses the ignored LÖVE fixture named by `BBT_UI_FIXTURE`, and renders 31 states sequentially in one process. A successful run cleans its stage. A forced termination may leave the stage behind; follow the [temporary artifact hygiene](benchmarking.md#temporary-artifact-hygiene) procedure rather than deleting the persistent fixture. Captures come from the same deterministic `600×360` canvas; `1200×720` review copies and red diff images are emitted under ignored `reports/ui`.

If a review image is open and Windows locks it, set `BBT_UI_REPORTS` to a fresh
workspace-relative report directory; the baselines and comparison thresholds
remain unchanged. `BBT_UI_PYTHON` may point to a local Python runtime that
provides Pillow when the default `python` command does not.

The gate compares approved files under `tests/ui-baselines` at threshold `0.1` and fails above `0.05%` changed pixels. It also fails on out-of-canvas text and controls below 22 logical pixels. Baselines change only through `pnpm test:ui:update` followed by human review.
