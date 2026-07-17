use crate::model::{
    AdmissionMode, ChartLock, Participant, ParticipantRole, RoomLifecycle, RoomSnapshot,
    RunValidity, ScoreTotals, SetlistEntry, MAX_PLAYERS, MAX_SPECTATORS,
};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug)]
pub struct RoomEngine {
    pub snapshot: RoomSnapshot,
    finalized_runs: HashSet<String>,
    started_runs: HashSet<String>,
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
            participant.invalid_reason = None;
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
        participant.ready = false;
        if role != ParticipantRole::Spectator {
            participant.commentator_access = false;
        }
        self.refresh_ready_lifecycle();
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
        let was_playing = matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        );
        if let Ok(participant) = self.participant_mut(session_id) {
            participant.connected = false;
            participant.ready = false;
            if was_playing {
                participant.validity = RunValidity::Pending;
                participant.invalid_reason = Some("Disconnected; awaiting journal recovery".into());
            }
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
            if playing && participant.admitted && !participant.connected {
                participant.validity = RunValidity::Dnf;
                participant.invalid_reason = Some("Disconnected for more than 30 seconds".into());
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
        self.touch();
        Ok(start)
    }

    pub fn start_run(&mut self, session_id: &str, max_hits: u64) -> Result<()> {
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
        participant.totals.max_hits = max_hits;
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

    pub fn ingest_score(&mut self, session_id: &str, sequence: u64, payload: &Value) -> Result<()> {
        if !matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing
        ) {
            bail!("room is not playing");
        }
        self.mark_playing();
        let participant = self.participant_mut(session_id)?;
        if let Some(previous) = participant.last_sequence {
            if sequence <= previous {
                return Ok(());
            }
            if sequence != previous + 1 {
                participant.validity = RunValidity::Invalid;
                participant.invalid_reason = Some(format!(
                    "Unrecoverable event sequence gap: expected {}, received {}",
                    previous + 1,
                    sequence
                ));
            }
        }
        participant.last_sequence = Some(sequence);
        let totals_value = payload.get("totals").unwrap_or(payload);
        let totals: ScoreTotals = serde_json::from_value(totals_value.clone())?;
        if totals.current_max_hits < participant.totals.current_max_hits
            || totals.misses < participant.totals.misses
            || totals.barelies < participant.totals.barelies
        {
            participant.validity = RunValidity::Invalid;
            participant.invalid_reason = Some("Score totals moved backwards".into());
        }
        participant.totals = totals;
        participant.accuracy = participant.totals.accuracy();
        participant.progress = payload
            .get("progress")
            .and_then(Value::as_f64)
            .unwrap_or_else(|| {
                if participant.totals.max_hits == 0 {
                    0.0
                } else {
                    participant.totals.current_max_hits as f64 / participant.totals.max_hits as f64
                }
            })
            .clamp(0.0, 1.0);
        if participant.validity == RunValidity::Pending {
            participant.validity = RunValidity::Valid;
        }
        self.rank();
        self.touch();
        Ok(())
    }

    pub fn invalidate(&mut self, session_id: &str, reason: String, dnf: bool) -> Result<()> {
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

    pub fn finish_run(&mut self, session_id: &str, _run_id: &str) -> Result<()> {
        // One participant contributes once per scheduled chart. Trusting the
        // client-provided run id here allowed duplicate ids to finish a room
        // before the other participants had reported results.
        if self.finalized_runs.contains(session_id) {
            return Ok(());
        }
        let participant = self.participant_mut(session_id)?;
        let contribution = if participant.validity == RunValidity::Valid {
            participant.accuracy
        } else {
            participant.validity = RunValidity::Dnf;
            participant.accuracy = 0.0;
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
        self.snapshot
            .participants
            .iter()
            .filter(|p| {
                p.admitted
                    && p.connected
                    && matches!(p.role, ParticipantRole::Player | ParticipantRole::Host)
            })
            .all(|p| p.ready && p.verified)
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
            .filter(|p| {
                p.admitted && matches!(p.role, ParticipantRole::Player | ParticipantRole::Host)
            })
            .count();
        if self.finalized_runs.len() < active {
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

    #[test]
    fn derives_accuracy_and_cumulative_set_total() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), true).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        room.ingest_score(&host, 0, &json!({"progress":1,"totals":{"hits":99,"misses":1,"barelies":0,"combo":0,"maxCombo":99,"currentMaxHits":100,"maxHits":100,"mineHits":0}})).unwrap();
        room.finish_run(&host, "run-1").unwrap();
        assert_eq!(room.player(&host).unwrap().accuracy, 99.0);
        assert_eq!(room.player(&host).unwrap().set_total, 99.0);
    }

    #[test]
    fn sequence_gap_marks_run_invalid_and_dnf_scores_zero() {
        let mut room = RoomEngine::host("Room".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(chart(), false).unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        let totals = json!({"hits":1,"misses":0,"barelies":0,"combo":1,"maxCombo":1,"currentMaxHits":1,"maxHits":100,"mineHits":0});
        room.ingest_score(&host, 0, &totals).unwrap();
        room.ingest_score(&host, 2, &totals).unwrap();
        room.finish_run(&host, "run-gap").unwrap();
        assert_eq!(room.player(&host).unwrap().validity, RunValidity::Dnf);
        assert_eq!(room.player(&host).unwrap().set_total, 0.0);
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
        room.finish_run(&host, "finished").unwrap();
        assert_eq!(room.snapshot.lifecycle, RoomLifecycle::Results);

        assert!(room.set_ready(&host, false).is_err());
        assert!(room.set_verified(&host, true, None).is_err());
        assert!(room.schedule_start(true, 2_000).is_err());
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
        room.start_run(&host, 100).unwrap();
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
}
