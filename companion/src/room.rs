use crate::model::{
    AdmissionMode, ChartLock, Participant, ParticipantRole, RoomLifecycle, RoomSnapshot,
    RunValidity, ScoreTotals, SetlistEntry, MAX_PLAYERS, MAX_SPECTATORS,
};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug)]
pub struct RoomEngine {
    pub snapshot: RoomSnapshot,
    finalized_runs: HashSet<String>,
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
        if self.snapshot.lifecycle == RoomLifecycle::Closed {
            bail!("room is closed");
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
        });
        self.touch();
        Ok(session_id)
    }

    pub fn admit(&mut self, session_id: &str, admit: bool, role: ParticipantRole) -> Result<()> {
        let participant = self.participant_mut(session_id)?;
        participant.admitted = admit;
        participant.connected = admit;
        participant.role = if role == ParticipantRole::Host {
            ParticipantRole::Player
        } else {
            role
        };
        if !admit {
            participant.invalid_reason = Some("Join request rejected by host".into());
        }
        self.touch();
        Ok(())
    }

    /// Host-side roster mutations used by the in-game Room page.
    pub fn set_role(&mut self, session_id: &str, role: ParticipantRole) -> Result<()> {
        let participant = self.participant_mut(session_id)?;
        participant.role = if role == ParticipantRole::Host { ParticipantRole::Player } else { role };
        participant.ready = false;
        self.touch();
        Ok(())
    }

    pub fn kick(&mut self, session_id: &str) -> Result<()> {
        if session_id == self.snapshot.host_session_id { bail!("the host cannot kick itself"); }
        let before = self.snapshot.participants.len();
        self.snapshot.participants.retain(|participant| participant.session_id != session_id);
        if before == self.snapshot.participants.len() { bail!("participant was not found"); }
        self.touch();
        Ok(())
    }

    pub fn remove_setlist(&mut self, index: usize) -> Result<()> {
        if index >= self.snapshot.setlist.len() { bail!("setlist index is out of range"); }
        self.snapshot.setlist.remove(index);
        self.snapshot.current_setlist_index = if self.snapshot.setlist.is_empty() { None } else { Some(index.min(self.snapshot.setlist.len() - 1)) };
        self.touch();
        Ok(())
    }

    pub fn move_setlist(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.snapshot.setlist.len() || to >= self.snapshot.setlist.len() { bail!("setlist index is out of range"); }
        let entry = self.snapshot.setlist.remove(from);
        self.snapshot.setlist.insert(to, entry);
        self.snapshot.current_setlist_index = Some(to);
        self.touch();
        Ok(())
    }

    pub fn advance_setlist(&mut self) -> Result<()> {
        let index = self.snapshot.current_setlist_index.context("setlist is not active")?;
        if let Some(entry) = self.snapshot.setlist.get_mut(index) { entry.completed = true; }
        let next = index + 1;
        if next >= self.snapshot.setlist.len() {
            self.snapshot.lifecycle = RoomLifecycle::SetComplete;
        } else {
            self.snapshot.current_setlist_index = Some(next);
            self.snapshot.chart = Some(self.snapshot.setlist[next].chart.clone());
            self.snapshot.lifecycle = RoomLifecycle::ChartLocked;
            for participant in &mut self.snapshot.participants {
                participant.ready = participant.role == ParticipantRole::Spectator;
                participant.verified = participant.role == ParticipantRole::Spectator;
            }
        }
        self.touch();
        Ok(())
    }

    pub fn disconnect(&mut self, session_id: &str) {
        let was_playing = self.snapshot.lifecycle == RoomLifecycle::Playing;
        if let Ok(participant) = self.participant_mut(session_id) {
            participant.connected = false;
            participant.ready = false;
            if was_playing {
                participant.validity = RunValidity::Pending;
                participant.invalid_reason = Some("Disconnected; awaiting journal recovery".into());
            }
            self.touch();
        }
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
        self.snapshot.chart = Some(chart.clone());
        if append_to_setlist {
            self.snapshot.setlist.push(SetlistEntry {
                id: Uuid::new_v4().to_string(),
                chart,
                completed: false,
            });
            if self.snapshot.current_setlist_index.is_none() {
                self.snapshot.current_setlist_index = Some(0);
            }
        }
        self.snapshot.lifecycle = RoomLifecycle::ChartLocked;
        for participant in &mut self.snapshot.participants {
            participant.ready = participant.role == ParticipantRole::Spectator;
            participant.verified = participant.role == ParticipantRole::Spectator;
            participant.validity = RunValidity::Pending;
            participant.invalid_reason = None;
        }
        self.touch();
        Ok(())
    }

    pub fn set_verified(
        &mut self,
        session_id: &str,
        verified: bool,
        reason: Option<String>,
    ) -> Result<()> {
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
        if matches!(
            self.snapshot.lifecycle,
            RoomLifecycle::Countdown | RoomLifecycle::Playing | RoomLifecycle::Closed
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
        self.touch();
        Ok(start)
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

    pub fn finish_run(&mut self, session_id: &str, run_id: &str) -> Result<()> {
        let final_key = format!("{session_id}:{run_id}");
        if self.finalized_runs.contains(&final_key) {
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
        self.finalized_runs.insert(final_key);
        self.try_finish_chart();
        self.touch();
        Ok(())
    }

    pub fn close(&mut self) {
        self.snapshot.lifecycle = RoomLifecycle::Closed;
        self.snapshot.scheduled_start_time_ms = None;
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
        if self.snapshot.chart.is_some() {
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
            let next = index + 1;
            if let Some(entry) = self.snapshot.setlist.get(next) {
                self.snapshot.current_setlist_index = Some(next);
                self.snapshot.chart = Some(entry.chart.clone());
            } else if !self.snapshot.setlist.is_empty() {
                self.snapshot.lifecycle = RoomLifecycle::SetComplete;
            }
        }
    }

    fn rank(&mut self) {
        let mut order = self
            .snapshot
            .participants
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.role, ParticipantRole::Player | ParticipantRole::Host))
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
}
