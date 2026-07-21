use crate::model::{
    AdmissionMode, ChartLock, Participant, ParticipantRole, RoomLifecycle, RoomSnapshot,
    RunValidity, ScoreTotals, SetlistEntry, MAX_PLAYERS, MAX_SPECTATORS,
};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

#[derive(Debug)]
pub struct RoomEngine {
    pub snapshot: RoomSnapshot,
    finalized_runs: HashSet<String>,
    started_runs: HashSet<String>,
    active_run_ids: HashMap<String, String>,
    retired_run_ids: HashMap<String, VecDeque<String>>,
    disconnect_deadlines: HashMap<String, u64>,
}

impl RoomEngine {
    pub fn offline() -> Self {
        let now = unix_ms();
        Self {
            snapshot: RoomSnapshot {
                id: "offline".into(),
                name: "Offline practice".into(),
                host_session_id: "local".into(),
                lifecycle: RoomLifecycle::Forming,
                admission_mode: AdmissionMode::HostApproval,
                allow_chart_transfers: true,
                validity_checks_enabled: true,
                participants: Vec::new(),
                chart: None,
                setlist: Vec::new(),
                current_setlist_index: None,
                scheduled_start_time_ms: None,
                force_start: false,
                created_at_ms: now,
                updated_at_ms: now,
            },
            finalized_runs: HashSet::new(),
            started_runs: HashSet::new(),
            active_run_ids: HashMap::new(),
            retired_run_ids: HashMap::new(),
            disconnect_deadlines: HashMap::new(),
        }
    }

    pub fn host(name: String, host_name: String, admission_mode: AdmissionMode) -> Self {
        let mut room = Self::offline();
        let now = unix_ms();
        let host_id = Uuid::new_v4().to_string();
        room.snapshot = RoomSnapshot {
            id: Uuid::new_v4().to_string(),
            name,
            host_session_id: host_id.clone(),
            lifecycle: RoomLifecycle::Forming,
            admission_mode,
            allow_chart_transfers: true,
            validity_checks_enabled: true,
            participants: vec![Participant {
                session_id: host_id,
                display_name: host_name,
                role: ParticipantRole::Host,
                admitted: true,
                connected: true,
                ready: false,
                verified: false,
                progress: 0.0,
                accuracy: 100.0,
                rank: None,
                set_total: 0.0,
                totals: ScoreTotals::default(),
                validity: RunValidity::Pending,
                invalid_reason: None,
                last_sequence: None,
                commentator_access: false,
            }],
            chart: None,
            setlist: Vec::new(),
            current_setlist_index: None,
            scheduled_start_time_ms: None,
            force_start: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        room
    }

    pub fn request_join(
        &mut self,
        display_name: &str,
        requested: ParticipantRole,
    ) -> Result<String> {
        self.request_join_with_id(Uuid::new_v4().to_string(), display_name, requested)
    }

    pub fn request_join_with_id(
        &mut self,
        session_id: String,
        display_name: &str,
        requested: ParticipantRole,
    ) -> Result<String> {
        self.disconnect_deadlines.remove(&session_id);
        if self.snapshot.lifecycle == RoomLifecycle::Closed {
            bail!("room is closed");
        }
        if let Some(participant) = self
            .snapshot
            .participants
            .iter_mut()
            .find(|participant| participant.session_id == session_id)
        {
            if participant.connected {
                bail!("this room session is already connected");
            }
            // The network layer only reuses a session id after proving the
            // opaque resume token issued during the original PAKE exchange.
            participant.connected = true;
            participant.ready = false;
            self.touch();
            return Ok(session_id);
        }
        if matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("a race is in progress; retry joining after results");
        }
        let role = if requested == ParticipantRole::Host {
            ParticipantRole::Player
        } else {
            requested
        };
        let players = self
            .snapshot
            .participants
            .iter()
            .filter(|p| matches!(p.role, ParticipantRole::Player | ParticipantRole::Host))
            .count();
        let spectators = self
            .snapshot
            .participants
            .iter()
            .filter(|p| p.role == ParticipantRole::Spectator)
            .count();
        if role == ParticipantRole::Player && players >= MAX_PLAYERS {
            bail!("room has reached its 16 player limit");
        }
        if role == ParticipantRole::Spectator && spectators >= MAX_SPECTATORS {
            bail!("room has reached its 32 spectator limit");
        }

        let normalized = display_name.trim();
        if normalized.is_empty() || normalized.chars().count() > 48 {
            bail!("display name must contain 1-48 characters");
        }
        let display_name = self.unique_name(normalized);
        self.snapshot.participants.push(Participant {
            session_id: session_id.clone(),
            display_name,
            role,
            admitted: self.snapshot.admission_mode == AdmissionMode::PasswordOnly,
            connected: true,
            ready: false,
            verified: false,
            progress: 0.0,
            accuracy: 100.0,
            rank: None,
            set_total: 0.0,
            totals: ScoreTotals::default(),
            validity: RunValidity::Pending,
            invalid_reason: None,
            last_sequence: None,
            commentator_access: false,
        });
        self.touch();
        Ok(session_id)
    }

    pub fn admit(&mut self, session_id: &str, admit: bool, role: ParticipantRole) -> Result<()> {
        if admit
            && matches!(
                self.snapshot.lifecycle,
                RoomLifecycle::Countdown | RoomLifecycle::Playing
            )
        {
            bail!("participants cannot be admitted during an active race");
        }
        let role = self.normalized_role(role);
        if admit {
            self.require_role_capacity(session_id, role)?;
        }
        let participant = self.participant_mut(session_id)?;
        participant.admitted = admit;
        participant.connected = admit;
        participant.role = role;
        if !admit || role != ParticipantRole::Spectator {
            participant.commentator_access = false;
        }
        if !admit {
            participant.invalid_reason = Some("Join request rejected by host".into());
        }
        self.refresh_ready_lifecycle();
        self.touch();
        Ok(())
    }

    /// Host-side roster mutations used by the in-game Room page.
    pub fn set_role(&mut self, session_id: &str, role: ParticipantRole) -> Result<()> {
        if matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("participant roles are locked during an active race");
        }
        let role = self.normalized_role(role);
        self.require_role_capacity(session_id, role)?;
        let participant = self.participant_mut(session_id)?;
        participant.role = role;
        // A spectator has no chart obligation. Moving back into a racing role
        // deliberately clears verification so stale verification cannot ready
        // a participant for a different assignment.
        participant.ready = role == ParticipantRole::Spectator;
        participant.verified = role == ParticipantRole::Spectator;
        if role != ParticipantRole::Spectator {
            participant.commentator_access = false;
        }
        self.refresh_ready_lifecycle();
        self.touch();
        Ok(())
    }

    /// Toggles whether the room owner races or directs the room as a
    /// spectator. The dedicated mutation preserves the Host role when racing;
    /// the generic role setter intentionally normalizes Host to Player.
    pub fn set_host_participating(&mut self, participating: bool) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Forming | RoomLifecycle::ChartLocked | RoomLifecycle::Ready
        ) {
            bail!("host participation can only change before a race");
        }
        let host_session_id = self.snapshot.host_session_id.clone();
        let role = if participating {
            ParticipantRole::Host
        } else {
            ParticipantRole::Spectator
        };
        self.require_role_capacity(&host_session_id, role)?;
        let participant = self.participant_mut(&host_session_id)?;
        participant.role = role;
        participant.ready = !participating;
        participant.verified = !participating;
        participant.commentator_access = false;
        self.refresh_ready_lifecycle();
        self.touch();
        Ok(())
    }

    /// Changes the room's integrity policy only while pre-race state is still
    /// editable. Structural score bounds remain mandatory in both modes.
    pub fn set_validity_checks(&mut self, enabled: bool) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Forming | RoomLifecycle::ChartLocked | RoomLifecycle::Ready
        ) {
            bail!("run validity checks can only change before a race");
        }
        self.snapshot.validity_checks_enabled = enabled;
        self.touch();
        Ok(())
    }

    /// Commentator is a permission layered on the non-competing Spectator
    /// role. Keeping it out of ParticipantRole preserves player/spectator room
    /// capacity and makes revocation on a role change unambiguous.
    pub fn set_commentator_access(&mut self, session_id: &str, enabled: bool) -> Result<()> {
        if session_id == self.snapshot.host_session_id {
            bail!("the host already owns broadcast controls");
        }
        let participant = self.participant_mut(session_id)?;
        if !participant.admitted || participant.role != ParticipantRole::Spectator {
            bail!("commentator access can only be granted to an admitted spectator");
        }
        participant.commentator_access = enabled;
        self.touch();
        Ok(())
    }

    pub fn kick(&mut self, session_id: &str) -> Result<()> {
        if session_id == self.snapshot.host_session_id {
            bail!("the host cannot kick itself");
        }
        let participant = self
            .player(session_id)
            .ok_or_else(|| anyhow::anyhow!("participant was not found"))?;
        // Pending approval requests can still be rejected during a race. Once
        // admitted, a racer stays locked into the authoritative result roster.
        if participant.admitted
            && matches!(
                self.snapshot.lifecycle,
                RoomLifecycle::Countdown | RoomLifecycle::Playing
            )
        {
            bail!("active participants cannot be removed during a race");
        }
        let before = self.snapshot.participants.len();
        self.disconnect_deadlines.remove(session_id);
        self.active_run_ids.remove(session_id);
        self.retired_run_ids.remove(session_id);
        self.started_runs.remove(session_id);
        self.finalized_runs.remove(session_id);
        self.snapshot
            .participants
            .retain(|participant| participant.session_id != session_id);
        if before == self.snapshot.participants.len() {
            bail!("participant was not found");
        }
        self.refresh_ready_lifecycle();
        self.touch();
        Ok(())
    }

    pub fn remove_setlist(&mut self, index: usize) -> Result<()> {
        self.require_setlist_editable()?;
        if index >= self.snapshot.setlist.len() {
            bail!("setlist index is out of range");
        }
        if self.snapshot.current_setlist_index == Some(index)
            && matches!(
                self.snapshot.lifecycle,
                RoomLifecycle::Results | RoomLifecycle::SetComplete
            )
        {
            bail!("the completed active chart cannot be removed before choosing what comes next");
        }
        let active_id = self
            .snapshot
            .current_setlist_index
            .and_then(|active| self.snapshot.setlist.get(active))
            .map(|entry| entry.id.clone());
        let removed_active = self.snapshot.current_setlist_index == Some(index);
        self.snapshot.setlist.remove(index);
        if self.snapshot.setlist.is_empty() {
            self.snapshot.current_setlist_index = None;
            self.snapshot.chart = None;
            self.snapshot.lifecycle = RoomLifecycle::Forming;
        } else if removed_active {
            let replacement = index.min(self.snapshot.setlist.len() - 1);
            self.snapshot.current_setlist_index = Some(replacement);
            self.snapshot.chart = Some(self.snapshot.setlist[replacement].chart.clone());
            self.reset_for_locked_chart();
        } else {
            self.snapshot.current_setlist_index = active_id.and_then(|id| {
                self.snapshot
                    .setlist
                    .iter()
                    .position(|entry| entry.id == id)
            });
        }
        self.touch();
        Ok(())
    }

    pub fn move_setlist(&mut self, from: usize, to: usize) -> Result<()> {
        self.require_setlist_editable()?;
        if from >= self.snapshot.setlist.len() || to >= self.snapshot.setlist.len() {
            bail!("setlist index is out of range");
        }
        if matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Results | RoomLifecycle::SetComplete
        ) && self
            .snapshot
            .current_setlist_index
            .is_some_and(|active| from <= active || to <= active)
        {
            bail!("completed setlist entries cannot be reordered");
        }
        let active_id = self
            .snapshot
            .current_setlist_index
            .and_then(|active| self.snapshot.setlist.get(active))
            .map(|entry| entry.id.clone());
        let entry = self.snapshot.setlist.remove(from);
        self.snapshot.setlist.insert(to, entry);
        self.snapshot.current_setlist_index = active_id.and_then(|id| {
            self.snapshot
                .setlist
                .iter()
                .position(|entry| entry.id == id)
        });
        self.touch();
        Ok(())
    }

    pub fn advance_setlist(&mut self) -> Result<()> {
        if self.snapshot.lifecycle != RoomLifecycle::Results {
            bail!("the setlist can advance only after chart results");
        }
        let index = self
            .snapshot
            .current_setlist_index
            .context("setlist is not active")?;
        if let Some(entry) = self.snapshot.setlist.get_mut(index) {
            entry.completed = true;
        }
        let next = index + 1;
        if next >= self.snapshot.setlist.len() {
            bail!("the setlist has no remaining chart");
        }
        self.snapshot.current_setlist_index = Some(next);
        self.snapshot.chart = Some(self.snapshot.setlist[next].chart.clone());
        self.reset_for_locked_chart();
        self.touch();
        Ok(())
    }

    pub fn disconnect(&mut self, session_id: &str) {
        self.disconnect_at(session_id, unix_ms());
    }

    fn disconnect_at(&mut self, session_id: &str, now_ms: u64) {
        if let Ok(participant) = self.participant_mut(session_id) {
            participant.connected = false;
            participant.ready = false;
            self.disconnect_deadlines
                .insert(session_id.to_owned(), now_ms.saturating_add(30_000));
            self.refresh_ready_lifecycle();
            self.touch();
        }
    }

    pub fn expire_disconnect(&mut self, session_id: &str) -> bool {
        self.disconnect_deadlines.remove(session_id);
        let playing = matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        );
        let expired_run = if let Ok(participant) = self.participant_mut(session_id) {
            if playing && is_competitor(participant) && !participant.connected {
                if !matches!(
                    participant.validity,
                    RunValidity::Invalid | RunValidity::Dnf
                ) {
                    participant.validity = RunValidity::Dnf;
                    participant.accuracy = 0.0;
                    participant.invalid_reason =
                        Some("Disconnected for more than 30 seconds".into());
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        if expired_run {
            self.finalized_runs.insert(session_id.to_owned());
            self.rank();
            self.try_finish_chart();
            self.touch();
            return true;
        }

        // A pre-game reconnect reservation lasts for 30 seconds. Once that
        // grace period expires, remove the stale roster entry so abandoned
        // admission requests cannot consume room capacity indefinitely.
        let pregame = matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Forming | RoomLifecycle::ChartLocked | RoomLifecycle::Ready
        );
        let removable = pregame
            && self
                .player(session_id)
                .is_some_and(|participant| !participant.connected);
        if removable {
            self.snapshot
                .participants
                .retain(|participant| participant.session_id != session_id);
            self.active_run_ids.remove(session_id);
            self.retired_run_ids.remove(session_id);
            self.started_runs.remove(session_id);
            self.finalized_runs.remove(session_id);
            self.refresh_ready_lifecycle();
            self.touch();
            return true;
        }
        false
    }

    /// Expires only disconnects whose current grace deadline has elapsed. A
    /// reconnect followed by another drop replaces the old deadline, so stale
    /// watchdogs cannot shorten the participant's new 30-second grace period.
    pub fn expire_due_disconnects(&mut self, now_ms: u64) -> Vec<String> {
        let due = self
            .disconnect_deadlines
            .iter()
            .filter(|(_, deadline)| **deadline <= now_ms)
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>();
        due.into_iter()
            .filter(|session_id| self.expire_disconnect(session_id))
            .collect()
    }

    pub fn lock_chart(&mut self, chart: ChartLock, append_to_setlist: bool) -> Result<()> {
        if matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("cannot change charts during countdown or play");
        }
        if chart.hash.len() != 64 || !chart.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("chart hash must be a SHA-256 hex digest");
        }
        if append_to_setlist {
            self.snapshot.setlist.push(SetlistEntry {
                id: Uuid::new_v4().to_string(),
                chart,
                completed: false,
            });
            if self.snapshot.current_setlist_index.is_none() {
                self.snapshot.current_setlist_index = Some(0);
                self.snapshot.chart = Some(self.snapshot.setlist[0].chart.clone());
                self.reset_for_locked_chart();
            } else if self.snapshot.lifecycle == RoomLifecycle::SetComplete {
                // Extending a completed set makes the newly appended entry a
                // valid "next chart" without discarding the visible results.
                self.snapshot.lifecycle = RoomLifecycle::Results;
            }
            self.touch();
            return Ok(());
        }

        // Selecting a chart replaces an existing set with a single-chart run.
        // Keeping a stale ordered set beside an unrelated active chart makes
        // Results/Advance semantics ambiguous and previously skipped entries.
        self.snapshot.setlist.clear();
        self.snapshot.current_setlist_index = None;
        self.snapshot.chart = Some(chart);
        self.reset_for_locked_chart();
        self.touch();
        Ok(())
    }

    pub fn set_verified(
        &mut self,
        session_id: &str,
        verified: bool,
        reason: Option<String>,
    ) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Forming | RoomLifecycle::ChartLocked | RoomLifecycle::Ready
        ) {
            bail!("chart verification is locked in the current room state");
        }
        let participant = self.participant_mut(session_id)?;
        participant.verified = verified;
        if !verified {
            participant.ready = false;
            participant.invalid_reason = reason;
        }
        self.refresh_ready_lifecycle();
        self.touch();
        Ok(())
    }

    pub fn set_ready(&mut self, session_id: &str, ready: bool) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Forming | RoomLifecycle::ChartLocked | RoomLifecycle::Ready
        ) {
            bail!("ready state is locked");
        }
        let participant = self.participant_mut(session_id)?;
        if participant.role != ParticipantRole::Spectator && ready && !participant.verified {
            bail!("matching chart verification is required before readying");
        }
        participant.ready = ready;
        self.refresh_ready_lifecycle();
        self.touch();
        Ok(())
    }

    pub fn schedule_start(&mut self, force: bool, delay_ms: u64) -> Result<u64> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::ChartLocked | RoomLifecycle::Ready
        ) {
            bail!("the room cannot start from its current lifecycle");
        }
        if self.snapshot.chart.is_none() {
            bail!("select a chart first");
        }
        if !self.snapshot.participants.iter().any(|participant| {
            participant.admitted
                && participant.connected
                && matches!(
                    participant.role,
                    ParticipantRole::Player | ParticipantRole::Host
                )
        }) {
            bail!("assign at least one player before starting");
        }
        if !force && !self.all_assigned_ready() {
            bail!("all assigned players must verify and ready; use Force Start to override");
        }
        let start = unix_ms().saturating_add(delay_ms.clamp(2_000, 15_000));
        self.snapshot.force_start = force;
        self.snapshot.scheduled_start_time_ms = Some(start);
        self.snapshot.lifecycle = RoomLifecycle::Countdown;
        for participant in &mut self.snapshot.participants {
            if participant.role != ParticipantRole::Spectator {
                participant.validity = RunValidity::Pending;
                participant.invalid_reason = None;
                participant.totals = ScoreTotals::default();
                participant.accuracy = 100.0;
                participant.progress = 0.0;
                participant.rank = None;
                participant.last_sequence = None;
            }
        }
        self.finalized_runs.clear();
        self.started_runs.clear();
        self.active_run_ids.clear();
        self.retired_run_ids.clear();
        self.touch();
        Ok(start)
    }

    pub fn start_run(&mut self, session_id: &str, run_id: &str, max_hits: u64) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("room is not accepting run starts");
        }
        let expected_max_hits = self
            .snapshot
            .chart
            .as_ref()
            .context("room has no locked chart")?
            .expected_max_hits;
        if max_hits != expected_max_hits {
            bail!(
                "run note count does not match the locked chart: expected {expected_max_hits}, got {max_hits}"
            );
        }
        validate_run_id(run_id)?;
        if self
            .retired_run_ids
            .get(session_id)
            .is_some_and(|retired| retired.iter().any(|value| value == run_id))
        {
            bail!("stale run attempt was already replaced");
        }
        let previous_run_id = self.active_run_ids.get(session_id).cloned();
        let new_attempt = previous_run_id
            .as_deref()
            .is_some_and(|previous| previous != run_id);
        let checks_enabled = self.snapshot.validity_checks_enabled;
        let participant = self.participant_mut(session_id)?;
        if !participant.admitted
            || !matches!(
                participant.role,
                ParticipantRole::Player | ParticipantRole::Host
            )
        {
            bail!("only admitted players can start a run");
        }
        if !participant.verified {
            bail!("the locked chart must be verified before starting a run");
        }
        if new_attempt {
            // A new native attempt owns an independent cumulative counter
            // stream. Competitive rooms retain (or create) the INVALID verdict;
            // casual rooms replace the unfinished attempt outright.
            participant.totals = ScoreTotals::default();
            participant.progress = 0.0;
            participant.accuracy = 100.0;
            participant.rank = None;
            participant.last_sequence = None;
            if checks_enabled {
                participant.validity = RunValidity::Invalid;
                if participant.invalid_reason.is_none() {
                    participant.invalid_reason =
                        Some("A new attempt started during the competitive race".into());
                }
            } else {
                participant.validity = RunValidity::Pending;
                participant.invalid_reason = None;
            }
        }
        participant.totals.max_hits = max_hits;
        if let Some(previous) = previous_run_id.filter(|_| new_attempt) {
            let retired = self
                .retired_run_ids
                .entry(session_id.to_owned())
                .or_default();
            if retired.len() >= 32 {
                retired.pop_front();
            }
            retired.push_back(previous);
        }
        self.active_run_ids
            .insert(session_id.to_owned(), run_id.to_owned());
        self.started_runs.insert(session_id.to_owned());
        self.touch();
        Ok(())
    }

    /// Finalizes assigned players that never reached the game after a bounded
    /// launch grace period. This prevents Force Start or a failed client load
    /// from leaving the room permanently stuck in Countdown/Playing.
    pub fn expire_unstarted_runs(&mut self) -> bool {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            return false;
        }
        let mut changed = false;
        for participant in &mut self.snapshot.participants {
            let assigned = participant.admitted
                && matches!(
                    participant.role,
                    ParticipantRole::Player | ParticipantRole::Host
                );
            if assigned
                && !self.started_runs.contains(&participant.session_id)
                && !self.finalized_runs.contains(&participant.session_id)
            {
                participant.validity = RunValidity::Dnf;
                participant.accuracy = 0.0;
                participant.invalid_reason =
                    Some("Game did not start within the 30-second launch grace period".into());
                self.finalized_runs.insert(participant.session_id.clone());
                changed = true;
            }
        }
        if changed {
            self.rank();
            self.try_finish_chart();
            self.touch();
        }
        changed
    }

    pub fn expire_due_unstarted_runs(&mut self, now_ms: u64) -> bool {
        let Some(start_ms) = self.snapshot.scheduled_start_time_ms else {
            return false;
        };
        if now_ms < start_ms.saturating_add(30_000) {
            return false;
        }
        self.expire_unstarted_runs()
    }

    pub fn mark_playing(&mut self) {
        if self.snapshot.lifecycle == RoomLifecycle::Countdown {
            self.snapshot.lifecycle = RoomLifecycle::Playing;
            self.touch();
        }
    }

    pub fn ingest_score(
        &mut self,
        session_id: &str,
        run_id: &str,
        sequence: u64,
        payload: &Value,
    ) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("room is not playing");
        }
        let validity_checks_enabled = self.snapshot.validity_checks_enabled;
        let expected_max_hits = self
            .snapshot
            .chart
            .as_ref()
            .context("room has no locked chart")?
            .expected_max_hits;
        let totals_value = payload.get("totals").unwrap_or(payload);
        let totals: ScoreTotals = serde_json::from_value(totals_value.clone())?;
        // A cumulative score is sufficient proof of a started attempt. This
        // also makes a lost run.started envelope recoverable and applies the
        // same one-time retry reset as an explicit start boundary.
        self.start_run(session_id, run_id, totals.max_hits)?;
        let participant = self
            .player(session_id)
            .context("unknown participant session")?;
        let previous_sequence = participant.last_sequence;
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                return Ok(());
            }
        }
        validate_score_totals(&participant.totals, &totals, expected_max_hits)?;

        self.mark_playing();
        {
            let participant = self.participant_mut(session_id)?;
            let gap = match previous_sequence {
                Some(previous) => sequence != previous.saturating_add(1),
                None => sequence != 0,
            };
            if gap && validity_checks_enabled {
                let expected = previous_sequence
                    .map(|previous| previous.saturating_add(1))
                    .unwrap_or(0);
                participant.validity = RunValidity::Invalid;
                participant.invalid_reason = Some(format!(
                    "Unrecoverable event sequence gap: expected {expected}, received {sequence}"
                ));
            }
            participant.last_sequence = Some(sequence);
            participant.totals = totals;
            participant.accuracy = participant.totals.accuracy();
            participant.progress = payload
                .get("progress")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| {
                    participant.totals.current_max_hits as f64 / participant.totals.max_hits as f64
                })
                .clamp(0.0, 1.0);
            if participant.validity == RunValidity::Pending {
                participant.validity = RunValidity::Valid;
            }
        }
        // A structurally valid score mutation from a verified competitor is
        // proof that the game entered the run. This keeps journal recovery
        // tolerant of a lost run.started envelope while still rejecting a bare
        // run.finished event.
        self.started_runs.insert(session_id.to_owned());
        self.rank();
        self.touch();
        Ok(())
    }

    pub fn invalidate(
        &mut self,
        session_id: &str,
        run_id: &str,
        reason: String,
        dnf: bool,
    ) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("room is not accepting run invalidations");
        }
        validate_run_id(run_id)?;
        if reason.trim().is_empty() || reason.chars().count() > 512 {
            bail!("run invalidation reason must contain 1-512 characters");
        }
        if self
            .active_run_ids
            .get(session_id)
            .is_some_and(|active| active != run_id)
        {
            return Ok(());
        }
        let checks_enabled = self.snapshot.validity_checks_enabled;
        if !self.player(session_id).is_some_and(is_competitor) {
            bail!("only admitted players can invalidate a run");
        }
        // Casual rooms still enforce completion and disconnect DNF handling,
        // but client-side retry/integrity signals do not disqualify an attempt.
        if !checks_enabled && !dnf {
            return Ok(());
        }
        self.active_run_ids
            .entry(session_id.to_owned())
            .or_insert_with(|| run_id.to_owned());
        self.started_runs.insert(session_id.to_owned());
        let participant = self.participant_mut(session_id)?;
        participant.validity = if dnf {
            RunValidity::Dnf
        } else {
            RunValidity::Invalid
        };
        participant.invalid_reason = Some(reason);
        self.rank();
        self.touch();
        Ok(())
    }

    /// Records a rejected gameplay event without letting malformed or
    /// impossible telemetry tear down the game/runtime IPC connection.
    pub fn reject_run_event(&mut self, session_id: &str, kind: &str, reason: &str) -> bool {
        if !self.snapshot.validity_checks_enabled
            || !matches!(
                self.snapshot.lifecycle,
                RoomLifecycle::Countdown | RoomLifecycle::Playing
            )
            || self.finalized_runs.contains(session_id)
            || !self.player(session_id).is_some_and(is_competitor)
        {
            return false;
        }
        let bounded = reason.chars().take(420).collect::<String>();
        let Ok(participant) = self.participant_mut(session_id) else {
            return false;
        };
        participant.validity = RunValidity::Invalid;
        participant.invalid_reason = Some(format!("Rejected {kind}: {bounded}"));
        self.rank();
        self.touch();
        true
    }

    pub fn finish_run(&mut self, session_id: &str, run_id: &str) -> Result<()> {
        // One participant contributes once per scheduled chart. Trusting the
        // client-provided run id here allowed duplicate ids to finish a room
        // before the other participants had reported results.
        let participant = self
            .player(session_id)
            .context("unknown participant session")?;
        if !is_competitor(participant) {
            bail!("only admitted players can finish a run");
        }
        if self.finalized_runs.contains(session_id) {
            return Ok(());
        }
        validate_run_id(run_id)?;
        if self
            .active_run_ids
            .get(session_id)
            .is_some_and(|active| active != run_id)
        {
            return Ok(());
        }
        if !self.started_runs.contains(session_id) {
            bail!("participant cannot finish a run that did not start");
        }
        let participant = self.participant_mut(session_id)?;
        let complete = participant.totals.max_hits > 0
            && participant.totals.current_max_hits == participant.totals.max_hits;
        let contribution = if participant.validity == RunValidity::Valid && complete {
            participant.accuracy
        } else {
            // Invalid is an integrity verdict and should survive finalization.
            // DNF is reserved for an otherwise pending/valid run that ended
            // without reporting every scoring opportunity.
            if participant.validity != RunValidity::Invalid {
                participant.validity = RunValidity::Dnf;
                participant.accuracy = 0.0;
                if participant.invalid_reason.is_none() {
                    participant.invalid_reason = Some(format!(
                        "Run ended before completion: received {} of {} scoring opportunities",
                        participant.totals.current_max_hits, participant.totals.max_hits
                    ));
                }
            }
            0.0
        };
        participant.set_total = ((participant.set_total + contribution) * 100.0).floor() / 100.0;
        self.finalized_runs.insert(session_id.to_owned());
        self.try_finish_chart();
        self.touch();
        Ok(())
    }

    pub fn close(&mut self) {
        self.snapshot.lifecycle = RoomLifecycle::Closed;
        self.snapshot.scheduled_start_time_ms = None;
        self.disconnect_deadlines.clear();
        self.touch();
    }

    pub fn player(&self, session_id: &str) -> Option<&Participant> {
        self.snapshot
            .participants
            .iter()
            .find(|participant| participant.session_id == session_id)
    }

    fn participant_mut(&mut self, session_id: &str) -> Result<&mut Participant> {
        self.snapshot
            .participants
            .iter_mut()
            .find(|participant| participant.session_id == session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown participant session"))
    }

    fn all_assigned_ready(&self) -> bool {
        let mut assigned = self.snapshot.participants.iter().filter(|p| {
            p.admitted
                && p.connected
                && matches!(p.role, ParticipantRole::Player | ParticipantRole::Host)
        });
        let Some(first) = assigned.next() else {
            return false;
        };
        first.ready && first.verified && assigned.all(|p| p.ready && p.verified)
    }

    fn refresh_ready_lifecycle(&mut self) {
        if self.snapshot.chart.is_some()
            && matches!(
                self.snapshot.lifecycle,
                RoomLifecycle::Forming | RoomLifecycle::ChartLocked | RoomLifecycle::Ready
            )
        {
            self.snapshot.lifecycle = if self.all_assigned_ready() {
                RoomLifecycle::Ready
            } else {
                RoomLifecycle::ChartLocked
            };
        }
    }

    fn try_finish_chart(&mut self) {
        let active = self
            .snapshot
            .participants
            .iter()
            .filter(|participant| is_competitor(participant))
            .map(|participant| participant.session_id.as_str())
            .collect::<Vec<_>>();
        if active.is_empty()
            || !active
                .iter()
                .all(|session_id| self.finalized_runs.contains(*session_id))
        {
            return;
        }
        self.snapshot.lifecycle = RoomLifecycle::Results;
        self.snapshot.scheduled_start_time_ms = None;
        if let Some(index) = self.snapshot.current_setlist_index {
            if let Some(entry) = self.snapshot.setlist.get_mut(index) {
                entry.completed = true;
            }
            if index + 1 >= self.snapshot.setlist.len() && !self.snapshot.setlist.is_empty() {
                self.snapshot.lifecycle = RoomLifecycle::SetComplete;
            }
        }
    }

    fn require_setlist_editable(&self) -> Result<()> {
        if matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("cannot edit the setlist during countdown or play");
        }
        Ok(())
    }

    fn normalized_role(&self, role: ParticipantRole) -> ParticipantRole {
        if role == ParticipantRole::Host {
            ParticipantRole::Player
        } else {
            role
        }
    }

    fn require_role_capacity(&self, session_id: &str, role: ParticipantRole) -> Result<()> {
        let count = self
            .snapshot
            .participants
            .iter()
            .filter(|participant| participant.session_id != session_id)
            .filter(|participant| match role {
                ParticipantRole::Player | ParticipantRole::Host => matches!(
                    participant.role,
                    ParticipantRole::Player | ParticipantRole::Host
                ),
                ParticipantRole::Spectator => participant.role == ParticipantRole::Spectator,
            })
            .count();
        match role {
            ParticipantRole::Player | ParticipantRole::Host if count >= MAX_PLAYERS => {
                bail!("room has reached its 16 player limit")
            }
            ParticipantRole::Spectator if count >= MAX_SPECTATORS => {
                bail!("room has reached its 32 spectator limit")
            }
            _ => Ok(()),
        }
    }

    fn reset_for_locked_chart(&mut self) {
        self.snapshot.lifecycle = RoomLifecycle::ChartLocked;
        self.snapshot.scheduled_start_time_ms = None;
        self.snapshot.force_start = false;
        for participant in &mut self.snapshot.participants {
            participant.ready = participant.role == ParticipantRole::Spectator;
            participant.verified = participant.role == ParticipantRole::Spectator;
            participant.validity = RunValidity::Pending;
            participant.invalid_reason = None;
        }
    }

    fn rank(&mut self) {
        let mut order = self
            .snapshot
            .participants
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.admitted && matches!(p.role, ParticipantRole::Player | ParticipantRole::Host)
            })
            .map(|(index, p)| {
                (
                    index,
                    p.validity,
                    p.accuracy,
                    p.progress,
                    p.totals.max_combo,
                    p.display_name.clone(),
                )
            })
            .collect::<Vec<_>>();
        order.sort_by(|a, b| {
            validity_order(b.1)
                .cmp(&validity_order(a.1))
                .then_with(|| b.2.total_cmp(&a.2))
                .then_with(|| b.3.total_cmp(&a.3))
                .then_with(|| b.4.cmp(&a.4))
                .then_with(|| a.5.cmp(&b.5))
        });
        for participant in &mut self.snapshot.participants {
            participant.rank = None;
        }
        for (rank, (index, ..)) in order.into_iter().enumerate() {
            self.snapshot.participants[index].rank = Some((rank + 1) as u32);
        }
    }

    fn unique_name(&self, name: &str) -> String {
        if !self
            .snapshot
            .participants
            .iter()
            .any(|p| p.display_name.eq_ignore_ascii_case(name))
        {
            return name.into();
        }
        for suffix in 2..=999 {
            let candidate = format!("{name} ({suffix})");
            if !self
                .snapshot
                .participants
                .iter()
                .any(|p| p.display_name.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
        }
        format!("{name} ({})", Uuid::new_v4().simple())
    }

    fn touch(&mut self) {
        self.snapshot.updated_at_ms = unix_ms();
    }
}

fn validity_order(value: RunValidity) -> u8 {
    match value {
        RunValidity::Valid => 3,
        RunValidity::Pending => 2,
        RunValidity::Invalid => 1,
        RunValidity::Dnf => 0,
    }
}

fn is_competitor(participant: &Participant) -> bool {
    participant.admitted
        && matches!(
            participant.role,
            ParticipantRole::Player | ParticipantRole::Host
        )
}

fn validate_run_id(run_id: &str) -> Result<()> {
    if run_id.trim().is_empty() || run_id.chars().count() > 128 {
        bail!("run id must contain 1-128 characters");
    }
    Ok(())
}

fn validate_score_totals(
    previous: &ScoreTotals,
    next: &ScoreTotals,
    expected_max_hits: u64,
) -> Result<()> {
    if next.max_hits != expected_max_hits {
        bail!(
            "score note count does not match the locked chart: expected {expected_max_hits}, got {}",
            next.max_hits
        );
    }
    let decisions = next.hits.saturating_add(next.misses);
    if next.current_max_hits > next.max_hits
        || next.hits > next.max_hits
        || next.barelies > next.hits
        || next.combo > next.max_combo
        || next.max_combo > next.hits
        || next.mine_hits > next.hits
        || next.mine_hits > next.max_hits
        || decisions < next.current_max_hits
    {
        bail!("score totals violate the locked chart counter bounds");
    }
    if next.current_max_hits < previous.current_max_hits
        || next.hits < previous.hits
        || next.misses < previous.misses
        || next.barelies < previous.barelies
        || next.max_combo < previous.max_combo
        || next.mine_hits < previous.mine_hits
    {
        bail!("score totals moved backwards");
    }
    Ok(())
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChartTransferMode;
    use serde_json::json;

    fn chart() -> ChartLock {
        ChartLock {
            hash: "a".repeat(64),
            package_name: "Chart".into(),
            song_name: "Signal".into(),
            variant: "Default".into(),
            expected_max_hits: 100,
            official: false,
            transfer_mode: ChartTransferMode::VerifyOnly,
        }
    }

    fn named_chart(name: &str, hash_digit: char) -> ChartLock {
        ChartLock {
            hash: hash_digit.to_string().repeat(64),
            package_name: name.into(),
            song_name: name.into(),
            ..chart()
        }
    }

    fn scheduled_room(validity_checks_enabled: bool) -> (RoomEngine, String) {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.set_validity_checks(validity_checks_enabled).unwrap();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        (room, host)
    }

    #[test]
    fn derives_accuracy_and_cumulative_set_total() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), true).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        room.ingest_score(&host, "run-1", 0, &json!({"progress":1,"totals":{"hits":99,"misses":1,"barelies":0,"combo":0,"maxCombo":99,"currentMaxHits":100,"maxHits":100,"mineHits":0}})).unwrap();
        room.finish_run(&host, "run-1").unwrap();
        assert_eq!(room.player(&host).unwrap().accuracy, 99.0);
        assert_eq!(room.player(&host).unwrap().set_total, 99.0);
    }

    #[test]
    fn sequence_gap_remains_invalid_and_scores_zero_after_finish() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        let totals = json!({"hits":1,"misses":0,"barelies":0,"combo":1,"maxCombo":1,"currentMaxHits":1,"maxHits":100,"mineHits":0});
        room.ingest_score(&host, "run-gap", 0, &totals).unwrap();
        room.ingest_score(&host, "run-gap", 2, &totals).unwrap();
        room.finish_run(&host, "run-gap").unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Invalid);
        assert!(room
            .player(&host)
            .unwrap()
            .invalid_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("sequence gap")));
        assert_eq!(room.player(&host).unwrap().set_total, 0.0);
    }

    #[test]
    fn casual_checks_tolerate_sequence_gaps_and_replace_retried_attempts() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.set_validity_checks(false).unwrap();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        room.start_run(&host, "casual-first", 100).unwrap();
        room.ingest_score(&host, "casual-first", 4, &json!({"hits":1,"misses":0,"barelies":0,"combo":1,"maxCombo":1,"currentMaxHits":1,"maxHits":100,"mineHits":0})).unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Valid);

        room.invalidate(&host, "casual-first", "Retry requested".into(), false)
            .unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Valid);
        room.start_run(&host, "casual-retry", 100).unwrap();
        let retried = room.player(&host).unwrap();
        assert_eq!(retried.validity, RunValidity::Pending);
        assert_eq!(retried.last_sequence, None);
        assert_eq!(retried.totals.current_max_hits, 0);

        room.ingest_score(&host, "casual-retry", 0, &json!({"hits":100,"misses":0,"barelies":0,"combo":100,"maxCombo":100,"currentMaxHits":100,"maxHits":100,"mineHits":0})).unwrap();
        room.finish_run(&host, "casual-retry").unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Valid);
        assert_eq!(room.player(&host).unwrap().set_total, 100.0);
    }

    #[test]
    fn casual_rooms_ignore_retry_invalidations_but_still_honor_dnf() {
        let (mut room, host) = scheduled_room(false);
        room.start_run(&host, "casual-run", 100).unwrap();

        room.invalidate(&host, "casual-run", "Retry requested".into(), false)
            .unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Pending);

        room.invalidate(&host, "casual-run", "Player quit the run".into(), true)
            .unwrap();
        let participant = room.player(&host).unwrap();
        assert_eq!(participant.validity, RunValidity::Dnf);
        assert_eq!(
            participant.invalid_reason.as_deref(),
            Some("Player quit the run")
        );
    }

    #[test]
    fn native_mine_and_mine_hold_counter_order_remains_valid() {
        let (mut room, host) = scheduled_room(true);
        let score = |hits, misses, current, mine_hits| {
            json!({
                "hits": hits,
                "misses": misses,
                "barelies": 0,
                "combo": hits,
                "maxCombo": hits,
                "currentMaxHits": current,
                "maxHits": 100,
                "mineHits": mine_hits
            })
        };

        // A safely avoided Mine increments hits/mineHits before addMineToTotal.
        room.ingest_score(&host, "mines", 0, &score(1, 0, 0, 1))
            .unwrap();
        room.ingest_score(&host, "mines", 1, &score(1, 0, 1, 1))
            .unwrap();
        // A hit Mine increments misses first; a MineHold may then add many
        // penalties while its scoring opportunity stays de-duplicated.
        room.ingest_score(&host, "mines", 2, &score(1, 1, 1, 1))
            .unwrap();
        room.ingest_score(&host, "mines", 3, &score(1, 1, 2, 1))
            .unwrap();
        room.ingest_score(&host, "mines", 4, &score(1, 150, 3, 1))
            .unwrap();
        room.ingest_score(&host, "mines", 5, &score(98, 150, 100, 1))
            .unwrap();
        room.finish_run(&host, "mines").unwrap();

        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Valid);
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
    }

    #[test]
    fn run_ids_make_duplicate_starts_and_lost_retry_starts_idempotent() {
        let (mut room, host) = scheduled_room(false);
        let score = |current| json!({"hits":current,"misses":0,"barelies":0,"combo":current,"maxCombo":current,"currentMaxHits":current,"maxHits":100,"mineHits":0});

        room.ingest_score(&host, "attempt-1", 0, &score(10))
            .unwrap();
        room.start_run(&host, "attempt-1", 100).unwrap();
        assert_eq!(room.player(&host).unwrap().totals.current_max_hits, 10);

        // The new score establishes attempt-2 even when run.started was lost.
        room.ingest_score(&host, "attempt-2", 0, &score(1)).unwrap();
        assert_eq!(room.player(&host).unwrap().totals.current_max_hits, 1);
        assert!(room
            .ingest_score(&host, "attempt-1", 1, &score(11))
            .is_err());
        room.finish_run(&host, "attempt-1").unwrap();
        assert!(!room.finalized_runs.contains(&host));
        room.ingest_score(&host, "attempt-2", 1, &score(100))
            .unwrap();
        room.finish_run(&host, "attempt-2").unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Valid);
        assert_eq!(room.player(&host).unwrap().set_total, 100.0);
    }

    #[test]
    fn competitive_retry_remains_invalid_even_if_invalidation_was_lost() {
        let (mut room, host) = scheduled_room(true);
        let score = |current| json!({"hits":current,"misses":0,"barelies":0,"combo":current,"maxCombo":current,"currentMaxHits":current,"maxHits":100,"mineHits":0});
        room.ingest_score(&host, "attempt-1", 0, &score(10))
            .unwrap();
        room.ingest_score(&host, "attempt-2", 0, &score(100))
            .unwrap();
        room.finish_run(&host, "attempt-2").unwrap();

        let participant = room.player(&host).unwrap();
        assert_eq!(participant.validity, RunValidity::Invalid);
        assert_eq!(participant.set_total, 0.0);
        assert!(participant
            .invalid_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("new attempt")));
    }

    #[test]
    fn reconnect_preserves_complete_and_invalid_run_verdicts() {
        let (mut valid_room, valid_host) = scheduled_room(true);
        let complete = json!({"hits":100,"misses":0,"barelies":0,"combo":100,"maxCombo":100,"currentMaxHits":100,"maxHits":100,"mineHits":0});
        valid_room
            .ingest_score(&valid_host, "valid-run", 0, &complete)
            .unwrap();
        valid_room.disconnect(&valid_host);
        valid_room
            .request_join_with_id(valid_host.clone(), "Host", ParticipantRole::Host)
            .unwrap();
        valid_room.finish_run(&valid_host, "valid-run").unwrap();
        assert_eq!(
            valid_room.player(&valid_host).unwrap().validity,
            RunValidity::Valid
        );
        assert_eq!(valid_room.player(&valid_host).unwrap().set_total, 100.0);

        let (mut invalid_room, invalid_host) = scheduled_room(true);
        invalid_room
            .ingest_score(&invalid_host, "invalid-run", 1, &complete)
            .unwrap();
        let reason = invalid_room
            .player(&invalid_host)
            .unwrap()
            .invalid_reason
            .clone();
        invalid_room.disconnect(&invalid_host);
        invalid_room
            .request_join_with_id(invalid_host.clone(), "Host", ParticipantRole::Host)
            .unwrap();
        invalid_room
            .finish_run(&invalid_host, "invalid-run")
            .unwrap();
        let participant = invalid_room.player(&invalid_host).unwrap();
        assert_eq!(participant.validity, RunValidity::Invalid);
        assert_eq!(participant.invalid_reason, reason);
        assert_eq!(participant.set_total, 0.0);
    }

    #[test]
    fn disconnect_expiry_finalizes_without_overwriting_invalid_reason() {
        let (mut room, host) = scheduled_room(true);
        let partial = json!({"hits":1,"misses":0,"barelies":0,"combo":1,"maxCombo":1,"currentMaxHits":1,"maxHits":100,"mineHits":0});
        room.ingest_score(&host, "invalid-run", 1, &partial)
            .unwrap();
        let reason = room.player(&host).unwrap().invalid_reason.clone();
        room.disconnect(&host);
        assert!(room.expire_disconnect(&host));
        let participant = room.player(&host).unwrap();
        assert_eq!(participant.validity, RunValidity::Invalid);
        assert_eq!(participant.invalid_reason, reason);
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
    }

    #[test]
    fn invalidation_reason_uses_protocol_character_limits() {
        let (mut room, host) = scheduled_room(true);
        room.start_run(&host, "unicode", 100).unwrap();
        room.invalidate(&host, "unicode", "界".repeat(512), false)
            .unwrap();
        assert!(room
            .invalidate(&host, "unicode", "界".repeat(513), false)
            .is_err());
    }

    #[test]
    fn incomplete_run_records_a_visible_dnf_reason() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        room.start_run(&host, "incomplete", 100).unwrap();
        room.ingest_score(&host, "incomplete", 0, &json!({"hits":75,"misses":0,"barelies":0,"combo":75,"maxCombo":75,"currentMaxHits":75,"maxHits":100,"mineHits":0})).unwrap();
        room.finish_run(&host, "incomplete").unwrap();

        let result = room.player(&host).unwrap();
        assert_eq!(result.validity, RunValidity::Dnf);
        assert_eq!(
            result.invalid_reason.as_deref(),
            Some("Run ended before completion: received 75 of 100 scoring opportunities")
        );
    }

    #[test]
    fn multi_chart_set_advances_exactly_once_after_results() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(named_chart("First", 'a'), true).unwrap();
        room.lock_chart(named_chart("Second", 'b'), true).unwrap();

        assert_eq!(room.snapshot.current_setlist_index, Some(0));
        assert_eq!(room.snapshot.chart.as_ref().unwrap().song_name, "First");
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        room.ingest_score(
            &host,
            "run-first",
            0,
            &json!({"progress":1,"totals":{"hits":100,"misses":0,"barelies":0,"combo":100,"maxCombo":100,"currentMaxHits":100,"maxHits":100,"mineHits":0}}),
        )
        .unwrap();
        room.finish_run(&host, "run-first").unwrap();

        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
        assert_eq!(room.snapshot.current_setlist_index, Some(0));
        assert_eq!(room.snapshot.chart.as_ref().unwrap().song_name, "First");
        assert!(room.snapshot.setlist[0].completed);
        assert!(!room.snapshot.setlist[1].completed);

        assert!(room.remove_setlist(0).is_err());
        assert!(room.move_setlist(1, 0).is_err());
        room.advance_setlist().unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::ChartLocked);
        assert_eq!(room.snapshot.current_setlist_index, Some(1));
        assert_eq!(room.snapshot.chart.as_ref().unwrap().song_name, "Second");
        assert!(!room.player(&host).unwrap().ready);
        assert!(!room.player(&host).unwrap().verified);
    }

    #[test]
    fn setlist_reordering_preserves_the_active_entry() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        room.lock_chart(named_chart("First", 'a'), true).unwrap();
        room.lock_chart(named_chart("Second", 'b'), true).unwrap();
        room.lock_chart(named_chart("Third", 'c'), true).unwrap();

        room.move_setlist(2, 1).unwrap();
        assert_eq!(room.snapshot.current_setlist_index, Some(0));
        assert_eq!(room.snapshot.chart.as_ref().unwrap().song_name, "First");
        room.remove_setlist(1).unwrap();
        assert_eq!(room.snapshot.current_setlist_index, Some(0));
        assert_eq!(room.snapshot.chart.as_ref().unwrap().song_name, "First");
    }

    #[test]
    fn role_changes_cannot_overfill_player_capacity() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        for index in 1..MAX_PLAYERS {
            room.request_join(&format!("Player {index}"), ParticipantRole::Player)
                .unwrap();
        }
        let spectator = room
            .request_join("Spectator", ParticipantRole::Spectator)
            .unwrap();
        let error = room
            .set_role(&spectator, ParticipantRole::Player)
            .unwrap_err();
        assert!(error.to_string().contains("16 player limit"));
        assert_eq!(
            room.player(&spectator).unwrap().role,
            ParticipantRole::Spectator
        );
    }

    #[test]
    fn authenticated_reconnect_restores_identity_before_disconnect_expiry() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let player = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        room.disconnect(&player);
        assert!(!room.player(&player).unwrap().connected);

        room.request_join_with_id(player.clone(), "Ignored rename", ParticipantRole::Spectator)
            .unwrap();
        let reconnected = room.player(&player).unwrap();
        assert!(reconnected.connected);
        assert_eq!(reconnected.display_name, "Player");
        assert_eq!(reconnected.role, ParticipantRole::Player);
    }

    #[test]
    fn commentator_grant_survives_reconnect_but_not_role_change() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let spectator = room
            .request_join("Caster", ParticipantRole::Spectator)
            .unwrap();
        room.set_commentator_access(&spectator, true).unwrap();
        room.disconnect(&spectator);
        room.request_join_with_id(spectator.clone(), "Caster", ParticipantRole::Spectator)
            .unwrap();
        assert!(room.player(&spectator).unwrap().commentator_access);

        room.set_role(&spectator, ParticipantRole::Player).unwrap();
        assert!(!room.player(&spectator).unwrap().commentator_access);
        assert!(room.set_commentator_access(&spectator, true).is_err());
    }

    #[test]
    fn reconnect_replaces_the_old_disconnect_deadline() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let player = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        room.disconnect_at(&player, 1_000);
        room.request_join_with_id(player.clone(), "Player", ParticipantRole::Player)
            .unwrap();
        room.disconnect_at(&player, 2_000);

        assert!(room.expire_due_disconnects(31_000).is_empty());
        assert_eq!(room.expire_due_disconnects(32_000), vec![player.clone()]);
        assert!(room.player(&player).is_none());
    }

    #[test]
    fn disconnect_expiry_marks_dnf_and_allows_results_to_complete() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        let player = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        for session in [&host, &player] {
            room.set_verified(session, true, None).unwrap();
            room.set_ready(session, true).unwrap();
        }
        room.schedule_start(false, 2_000).unwrap();
        room.mark_playing();
        room.start_run(&host, "host-run", 100).unwrap();
        room.finish_run(&host, "host-run").unwrap();
        room.disconnect(&player);
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Playing);

        assert!(room.expire_disconnect(&player));
        assert_eq!(room.player(&player).unwrap().validity, RunValidity::Dnf);
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
    }

    #[test]
    fn pregame_disconnect_expiry_releases_capacity_and_recomputes_ready_state() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        let player = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.disconnect(&player);

        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Ready);
        assert!(room.expire_disconnect(&player));
        assert!(room.player(&player).is_none());
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Ready);
    }

    #[test]
    fn role_changes_recompute_chart_readiness() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        let player = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        for session in [&host, &player] {
            room.set_verified(session, true, None).unwrap();
            room.set_ready(session, true).unwrap();
        }
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Ready);

        room.set_role(&player, ParticipantRole::Spectator).unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Ready);
        room.set_role(&player, ParticipantRole::Player).unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::ChartLocked);
        assert!(!room.player(&player).unwrap().verified);
    }

    #[test]
    fn host_can_direct_without_consuming_a_racer_slot_and_rejoin_as_host() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();

        room.set_host_participating(false).unwrap();
        assert_eq!(room.player(&host).unwrap().role, ParticipantRole::Spectator);
        assert!(room.player(&host).unwrap().ready);
        assert!(room.player(&host).unwrap().verified);
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::ChartLocked);
        assert!(room.schedule_start(true, 2_000).is_err());

        room.set_host_participating(true).unwrap();
        assert_eq!(room.player(&host).unwrap().role, ParticipantRole::Host);
        assert!(!room.player(&host).unwrap().ready);
        assert!(!room.player(&host).unwrap().verified);
    }

    #[test]
    fn host_participation_is_locked_once_countdown_begins() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();

        assert!(room.set_host_participating(false).is_err());
        assert_eq!(room.player(&host).unwrap().role, ParticipantRole::Host);
    }

    #[test]
    fn completed_room_rejects_ready_verification_and_restart_actions() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        room.mark_playing();
        room.start_run(&host, "finished", 100).unwrap();
        room.finish_run(&host, "finished").unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);

        assert!(room.set_ready(&host, false).is_err());
        assert!(room.set_verified(&host, true, None).is_err());
        assert!(room.schedule_start(true, 2_000).is_err());
        assert!(room
            .invalidate(&host, "host-run", "late mutation".into(), false)
            .is_err());
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
    }

    #[test]
    fn active_race_rejects_new_join_and_admission_but_allows_authenticated_resume() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::HostApproval);
        let host = room.snapshot.host_session_id.clone();
        let pending = room
            .request_join("Pending", ParticipantRole::Player)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();

        assert!(room.request_join("Late", ParticipantRole::Player).is_err());
        assert!(room.admit(&pending, true, ParticipantRole::Player).is_err());
        room.disconnect(&pending);
        room.request_join_with_id(pending.clone(), "Ignored", ParticipantRole::Spectator)
            .unwrap();
        assert!(room.player(&pending).unwrap().connected);
        assert!(!room.player(&pending).unwrap().admitted);
    }

    #[test]
    fn active_race_allows_rejecting_pending_request_but_not_kicking_racer() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::HostApproval);
        let host = room.snapshot.host_session_id.clone();
        let racer = room.request_join("Racer", ParticipantRole::Player).unwrap();
        room.admit(&racer, true, ParticipantRole::Player).unwrap();
        let pending = room
            .request_join("Pending", ParticipantRole::Player)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        for session in [&host, &racer] {
            room.set_verified(session, true, None).unwrap();
            room.set_ready(session, true).unwrap();
        }
        room.schedule_start(false, 2_000).unwrap();

        assert!(room.kick(&racer).is_err());
        room.kick(&pending).unwrap();
        assert!(room.player(&pending).is_none());
    }

    #[test]
    fn finish_is_participant_scoped_and_unstarted_watchdog_completes_room() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        let player = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        for session in [&host, &player] {
            room.set_verified(session, true, None).unwrap();
            room.set_ready(session, true).unwrap();
        }
        let scheduled = room.schedule_start(false, 2_000).unwrap();
        room.start_run(&host, "first-id", 100).unwrap();
        room.finish_run(&host, "first-id").unwrap();
        room.finish_run(&host, "different-id").unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Countdown);

        assert!(!room.expire_due_unstarted_runs(scheduled + 29_999));
        assert!(room.expire_due_unstarted_runs(scheduled + 30_000));
        assert_eq!(room.player(&player).unwrap().validity, RunValidity::Dnf);
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
    }

    #[test]
    fn setlist_cannot_advance_before_results() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        room.lock_chart(named_chart("First", 'a'), true).unwrap();
        room.lock_chart(named_chart("Second", 'b'), true).unwrap();
        assert!(room.advance_setlist().is_err());
        assert_eq!(room.snapshot.current_setlist_index, Some(0));
        assert_eq!(room.snapshot.chart.as_ref().unwrap().song_name, "First");
    }

    #[test]
    fn appending_to_a_completed_set_restores_the_next_chart_transition() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(named_chart("First", 'a'), true).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        room.start_run(&host, "finished", 100).unwrap();
        room.finish_run(&host, "finished").unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::SetComplete);

        room.lock_chart(named_chart("Encore", 'b'), true).unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
        room.advance_setlist().unwrap();
        assert_eq!(room.snapshot.chart.as_ref().unwrap().song_name, "Encore");
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::ChartLocked);
    }

    #[test]
    fn spectators_and_pending_players_cannot_submit_run_events() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::HostApproval);
        let host = room.snapshot.host_session_id.clone();
        let spectator = room
            .request_join("Spectator", ParticipantRole::Spectator)
            .unwrap();
        room.admit(&spectator, true, ParticipantRole::Spectator)
            .unwrap();
        let pending = room
            .request_join("Pending", ParticipantRole::Player)
            .unwrap();
        let unverified = room
            .request_join("Unverified", ParticipantRole::Player)
            .unwrap();
        room.admit(&unverified, true, ParticipantRole::Player)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(true, 2_000).unwrap();
        let score = json!({
            "progress": 0.01,
            "totals": {
                "hits": 1,
                "misses": 0,
                "barelies": 0,
                "combo": 1,
                "maxCombo": 1,
                "currentMaxHits": 1,
                "maxHits": 100,
                "mineHits": 0
            }
        });

        for session_id in [&spectator, &pending] {
            assert!(room
                .ingest_score(session_id, "not-a-run", 0, &score)
                .is_err());
            assert!(room
                .invalidate(session_id, "not-a-run", "not a competitor".into(), false)
                .is_err());
            assert!(room.finish_run(session_id, "not-a-run").is_err());
        }
        assert!(room
            .ingest_score(&unverified, "not-a-run", 0, &score)
            .is_err());
        assert!(room.finish_run(&unverified, "not-a-run").is_err());
        assert!(!room.started_runs.contains(&unverified));
        room.disconnect_at(&spectator, 0);
        assert!(!room.expire_disconnect(&spectator));
        assert_eq!(
            room.player(&spectator).unwrap().validity,
            RunValidity::Pending
        );
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Countdown);
    }

    #[test]
    fn only_finalized_competitor_ids_can_complete_the_chart() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        let player = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        let spectator = room
            .request_join("Spectator", ParticipantRole::Spectator)
            .unwrap();
        room.lock_chart(chart(), false).unwrap();
        for session_id in [&host, &player] {
            room.set_verified(session_id, true, None).unwrap();
            room.set_ready(session_id, true).unwrap();
        }
        room.schedule_start(false, 2_000).unwrap();
        room.mark_playing();

        // Completion must use the active competitor-ID subset, even if stale
        // noncompetitor state somehow survives from an older snapshot.
        room.finalized_runs.insert(spectator);
        let complete = json!({
            "progress": 1.0,
            "totals": {
                "hits": 100,
                "misses": 0,
                "barelies": 0,
                "combo": 100,
                "maxCombo": 100,
                "currentMaxHits": 100,
                "maxHits": 100,
                "mineHits": 0
            }
        });
        room.ingest_score(&host, "host-run", 0, &complete).unwrap();
        room.finish_run(&host, "host-run").unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Playing);

        room.ingest_score(&player, "player-run", 0, &complete)
            .unwrap();
        room.finish_run(&player, "player-run").unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);
    }

    #[test]
    fn finish_requires_start_and_incomplete_valid_runs_become_dnf() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();

        assert!(room.finish_run(&host, "never-started").is_err());
        room.ingest_score(
            &host,
            "early-finish",
            0,
            &json!({
                "progress": 0.01,
                "totals": {
                    "hits": 1,
                    "misses": 0,
                    "barelies": 0,
                    "combo": 1,
                    "maxCombo": 1,
                    "currentMaxHits": 1,
                    "maxHits": 100,
                    "mineHits": 0
                }
            }),
        )
        .unwrap();
        room.finish_run(&host, "early-finish").unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Dnf);
        assert_eq!(room.player(&host).unwrap().set_total, 0.0);
    }

    #[test]
    fn score_totals_preserve_the_lock_and_detect_initial_or_counter_gaps() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();

        let score = |current: u64, max_hits: u64| {
            json!({
                "progress": current as f64 / max_hits as f64,
                "totals": {
                    "hits": current,
                    "misses": 0,
                    "barelies": 0,
                    "combo": current,
                    "maxCombo": current,
                    "currentMaxHits": current,
                    "maxHits": max_hits,
                    "mineHits": 0
                }
            })
        };
        assert!(room
            .ingest_score(&host, "score-run", 0, &score(1, 99))
            .is_err());
        room.ingest_score(&host, "score-run", 1, &score(2, 100))
            .unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Invalid);
        assert!(room
            .player(&host)
            .unwrap()
            .invalid_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("expected 0, received 1")));
        assert!(room
            .ingest_score(
                &host,
                "score-run",
                2,
                &json!({
                    "progress": 0.03,
                    "totals": {
                        "hits": 2,
                        "misses": 0,
                        "barelies": 0,
                        "combo": 2,
                        "maxCombo": 2,
                        "currentMaxHits": 3,
                        "maxHits": 100,
                        "mineHits": 0
                    }
                }),
            )
            .is_err());
        assert!(room
            .ingest_score(&host, "score-run", 2, &score(1, 100))
            .is_err());
        assert_eq!(room.player(&host).unwrap().last_sequence, Some(1));
        assert_eq!(room.player(&host).unwrap().totals.max_hits, 100);
    }
}
