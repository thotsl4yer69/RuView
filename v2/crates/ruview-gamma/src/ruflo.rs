//! RuFlo governance layer (ADR-250 §11).
//!
//! The governor is the only public entry point that *runs* sessions. It
//! enforces, in order: consent → inclusion/exclusion screen → envelope check →
//! simulated/observed session → safety monitor → objective score → RuVector
//! update → witnessed audit record. It also owns trial-mode separation (sham /
//! blinding) and the **claim-discipline** statement (ADR-250 §18: "no disease
//! treatment claim").

use crate::objective::{SafeEntrainmentObjective, ScoreInputs};
use crate::optimizer::{BayesianOptimizer, CalibrationPlan, Recommendation};
use crate::participant::{
    lock_class_for, stop_tag, DoseLimits, DoseViolation, LockClass, ParticipantSafetyState,
};
use crate::program::NeuroProgram;
use crate::response::{
    PersonResponseVector, RuViewState, SessionObservation, SleepState, SubjectiveReport,
};
use crate::ruvector::{DriftDetector, DriftStatus};
use crate::safety::{
    ExclusionCondition, ExclusionScreen, SafetyMonitor, SafetyTick, ScreenOutcome, StopReason,
};
use crate::session::{Outcome, SessionBuilder, SessionRecord, VersionTriple};
use crate::simulator::{LatentPerson, ResponseSimulator};
use crate::stimulus::{SafetyEnvelope, StimulusParameters};

/// The single, immutable product claim (ADR-250 §22). Exposed so any UI/report
/// can render exactly this and nothing stronger.
pub const PRODUCT_CLAIM: &str = "personalized entrainment optimization";

/// Consent state (ADR-250 §11 RuFlo responsibility 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    Granted,
    Withdrawn,
}

/// Trial mode for controlled studies (ADR-250 §21 Milestone 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialMode {
    /// Normal operation: real stimulation, adaptive optimization.
    Open,
    /// Sham: the participant-facing protocol is logged, but no entrainment is
    /// delivered (blinding). Outcomes show the no-treatment baseline.
    Sham,
}

/// Governance refusals — every one is a *safe* refusal (fail closed).
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GovernanceError {
    #[error("participant is excluded from unsupervised use: {0:?}")]
    Excluded(Vec<ExclusionCondition>),
    #[error("clinical supervision required for: {0:?}")]
    SupervisionRequired(Vec<ExclusionCondition>),
    #[error("consent not granted (or withdrawn)")]
    NoConsent,
    #[error("requested stimulus is outside the approved safety envelope")]
    OutsideEnvelope,
    /// Finding 2: the participant is latched-locked after an adverse-event /
    /// distress / seizure-like stop (ADR-250 §8 terminate-and-lock). Only
    /// [`RufloGovernor::unlock_with_acknowledgment`] clears it.
    #[error(
        "participant is locked ({class:?} — {reason}); operator acknowledgment required to resume"
    )]
    ParticipantLocked { class: LockClass, reason: String },
    /// Finding 4: the rolling-24h daily-dose cap is exhausted.
    #[error("daily dose limit reached: {sittings} sittings in 24h exceeds the cap of {max}")]
    DailyDoseLimit { sittings: usize, max: usize },
    /// Finding 4: the minimum inter-session cooldown has not yet elapsed.
    #[error("inter-session cooldown active: {elapsed_minutes:.1} min elapsed, {required_minutes:.1} min required")]
    CooldownActive {
        elapsed_minutes: f64,
        required_minutes: f64,
    },
    /// [`RufloGovernor::unlock_with_acknowledgment`] was called but the
    /// participant was not locked.
    #[error("participant is not locked")]
    NotLocked,
}

// Clinician export (`ClinicianReport` + `clinician_report`) lives in the child
// module `report` (split out to keep this file under 500 lines); it is a
// descendant module, so it retains access to the governor's private fields.
#[path = "ruflo_report.rs"]
mod report;
pub use report::ClinicianReport;

// RuVector cohort bridge (`seed_from_cohort`, `export_anonymized_profile`),
// split out to keep this file under 500 lines; descendant module → private access.
#[path = "ruflo_ruvector.rs"]
mod ruvector_bridge;

/// The governed adaptive-gamma protocol runner for one participant.
pub struct RufloGovernor {
    person_id: String,
    envelope: SafetyEnvelope,
    objective: SafeEntrainmentObjective,
    optimizer: BayesianOptimizer,
    response: PersonResponseVector,
    versions: VersionTriple,
    consent: Consent,
    mode: TrialMode,
    confidence_floor: f64,
    audit: Vec<SessionRecord>,
    next_index: u64,
    // ADR-250 §10 item 4: per-person drift detection over the response vector.
    drift: DriftDetector,
    drift_status: DriftStatus,
    // Finding 2 & 4: persisted cross-session safety state (latched lock + the
    // dose/cooldown ledger). Serialize it into the session store so a NEW
    // governor for the same participant honors the lock and the dose history.
    safety_state: ParticipantSafetyState,
    dose_limits: DoseLimits,
    // Platform extension: the program this participant is enrolled under
    // (envelope/prior/objective/state-gating/claim). `None` for the bare
    // `enroll` path (Alzheimer's defaults), which keeps the pinned witness.
    program: Option<NeuroProgram>,
}

impl RufloGovernor {
    /// Enroll a participant. Fails closed on exclusion or missing consent.
    pub fn enroll(
        person_id: impl Into<String>,
        envelope: SafetyEnvelope,
        conditions: &[ExclusionCondition],
        consent: Consent,
    ) -> Result<Self, GovernanceError> {
        if consent != Consent::Granted {
            return Err(GovernanceError::NoConsent);
        }
        match ExclusionScreen.evaluate(conditions) {
            ScreenOutcome::Excluded(c) => return Err(GovernanceError::Excluded(c)),
            ScreenOutcome::RequiresClinicalSupervision(c) => {
                return Err(GovernanceError::SupervisionRequired(c))
            }
            ScreenOutcome::Cleared => {}
        }
        let baseline_ruview = RuViewState::calm_baseline();
        Ok(Self {
            person_id: person_id.into(),
            objective: SafeEntrainmentObjective::new(Default::default(), envelope),
            optimizer: BayesianOptimizer::default(),
            response: PersonResponseVector::baseline(0.2, 0.5, &baseline_ruview),
            versions: VersionTriple::default(),
            consent,
            mode: TrialMode::Open,
            confidence_floor: 0.5,
            envelope,
            audit: Vec::new(),
            next_index: 0,
            drift: DriftDetector::default(),
            drift_status: DriftStatus::Warmup,
            safety_state: ParticipantSafetyState::default(),
            dose_limits: DoseLimits::conservative(),
            program: None,
        })
    }

    /// Minimum number of distinct cohort profiles required before
    /// [`seed_from_cohort`](Self::seed_from_cohort) will consume their priors — a
    /// privacy k-floor so a single (or pair of) donor profile(s) cannot shape a
    /// new participant's optimizer.
    pub const MIN_COHORT_PROFILES: usize = 3;

    /// Enroll a participant under a [`NeuroProgram`] (the platform path): the
    /// program supplies the safety envelope, starting prior, and objective
    /// weighting for this use case. Same fail-closed consent/exclusion gate as
    /// [`enroll`](Self::enroll). The program's claim is only releasable through
    /// the acceptance gate (`crate::acceptance`), never directly.
    pub fn enroll_program(
        person_id: impl Into<String>,
        program: NeuroProgram,
        conditions: &[ExclusionCondition],
        consent: Consent,
    ) -> Result<Self, GovernanceError> {
        let mut gov = Self::enroll(person_id, program.envelope, conditions, consent)?;
        gov.objective = SafeEntrainmentObjective::new(program.weights, program.envelope);
        gov.versions.protocol_version = format!("adr-250-{}-v0.1", program.id);
        gov.program = Some(program);
        Ok(gov)
    }

    /// The program this participant is enrolled under, if any.
    pub fn program(&self) -> Option<&NeuroProgram> {
        self.program.as_ref()
    }

    /// The program's starting prior, or the ADR-250 40 Hz prior if none.
    pub fn prior(&self) -> StimulusParameters {
        self.program
            .as_ref()
            .map(|p| p.prior)
            .unwrap_or_else(StimulusParameters::prior)
    }

    /// Whether a session in `state` fits the enrolled program's protocol
    /// (e.g. the sleep program permits `Asleep`). Programs without state
    /// constraints (the bare path) accept any state.
    pub fn state_eligible(&self, state: SleepState) -> bool {
        self.program
            .as_ref()
            .map(|p| p.state_eligible(state))
            .unwrap_or(true)
    }

    // `seed_from_cohort` + `export_anonymized_profile` (the RuVector cohort
    // bridge, ADR-250 §10) live in the child module `ruvector_bridge` to keep
    // this file under 500 lines; it retains private-field access.

    /// Latest drift judgment (ADR-250 §10 item 4). `Drifted` recommends
    /// re-running the Phase-1 calibration sweep before trusting further
    /// optimization.
    pub fn drift_status(&self) -> DriftStatus {
        self.drift_status
    }

    /// The persisted per-participant safety state (latched lock + dose ledger,
    /// Finding 2 & 4). Serialize it into the session store; reload it into a new
    /// governor with [`with_safety_state`](Self::with_safety_state) and the lock
    /// and dose history are honored across instances.
    pub fn safety_state(&self) -> &ParticipantSafetyState {
        &self.safety_state
    }

    /// This participant's safety envelope.
    pub fn envelope(&self) -> &SafetyEnvelope {
        &self.envelope
    }

    /// Whether the participant is currently latched-locked.
    pub fn is_locked(&self) -> bool {
        self.safety_state.is_locked()
    }

    /// Load a previously-persisted safety state onto a fresh governor (e.g. for a
    /// returning participant). A locked state makes this governor refuse to run
    /// sessions until [`unlock_with_acknowledgment`](Self::unlock_with_acknowledgment).
    pub fn with_safety_state(mut self, state: ParticipantSafetyState) -> Self {
        self.safety_state = state;
        self
    }

    /// Override the dose/cooldown policy (Finding 4). Defaults to
    /// [`DoseLimits::conservative`]; real deployments must keep a conservative
    /// policy — the permissive variant exists only for time-compressed
    /// simulation.
    pub fn with_dose_limits(mut self, limits: DoseLimits) -> Self {
        self.dose_limits = limits;
        self
    }

    /// Lift a latched lock with an explicit operator acknowledgment, writing an
    /// audit record into the persisted safety state (ADR-250 §8 / §11, Finding 2).
    ///
    /// # Errors
    /// [`GovernanceError::NotLocked`] if the participant was not locked.
    pub fn unlock_with_acknowledgment(
        &mut self,
        operator_note: &str,
        timestamp_ms: u64,
    ) -> Result<(), GovernanceError> {
        match self.safety_state.unlock(operator_note, timestamp_ms) {
            Some(_) => Ok(()),
            None => Err(GovernanceError::NotLocked),
        }
    }

    /// Switch trial mode (e.g., to `Sham` for a blinded arm).
    pub fn set_mode(&mut self, mode: TrialMode) {
        self.mode = mode;
    }

    /// Withdraw consent — all subsequent `run_session` calls fail closed.
    pub fn withdraw_consent(&mut self) {
        self.consent = Consent::Withdrawn;
    }

    /// Immutable view of the audit trail (every session is witnessed).
    pub fn audit_log(&self) -> &[SessionRecord] {
        &self.audit
    }

    /// Current personal response vector (RuVector memory).
    pub fn response_vector(&self) -> &PersonResponseVector {
        &self.response
    }

    /// Run the Phase-1 calibration sweep against a simulated participant,
    /// recording every session and seeding the optimizer.
    pub fn run_calibration(
        &mut self,
        sim: &ResponseSimulator,
        latent: &LatentPerson,
        state: &RuViewState,
        session_minutes: f64,
        base_timestamp_ms: u64,
    ) -> Result<(), GovernanceError> {
        let mut plan = CalibrationPlan::new(&self.envelope);
        while let Some(stim) = plan.next_stimulus(&self.envelope, session_minutes) {
            // Finding 1: a safety stop is a control-flow event. `run_session`
            // still returns the (witnessed) record on a stop, so we must inspect
            // its safety outcome and TERMINATE the sweep — a stop in step N must
            // not let steps N+1.. proceed. The partial calibration stays in the
            // audit log, and any lock-warranting stop has already engaged the
            // governor lock (ADR-250 §8 terminate-and-lock).
            let record = self.run_session(sim, latent, state, &stim, base_timestamp_ms)?;
            if !record.outcome.safety_pass {
                break;
            }
        }
        Ok(())
    }

    /// Recommend the next protocol given the current state (ADR-250 §14).
    pub fn recommend(&self, base: &StimulusParameters) -> Recommendation {
        self.optimizer.recommend(&self.envelope, base)
    }

    /// Run one governed session end-to-end. Returns the witnessed record.
    ///
    /// Fails closed if consent is absent or the stimulus is outside the
    /// envelope. Any safety stop is logged into the record (ADR-250 §18).
    pub fn run_session(
        &mut self,
        sim: &ResponseSimulator,
        latent: &LatentPerson,
        state: &RuViewState,
        stimulus: &StimulusParameters,
        timestamp_ms: u64,
    ) -> Result<SessionRecord, GovernanceError> {
        if self.consent != Consent::Granted {
            return Err(GovernanceError::NoConsent);
        }
        // Finding 2: a locked participant refuses every session (fail closed)
        // until an explicit operator acknowledgment lifts the lock. This holds
        // across governor instances because the lock lives in `safety_state`,
        // which is serialized into the session store.
        if let Some(lock) = self.safety_state.lock_record() {
            return Err(GovernanceError::ParticipantLocked {
                class: lock.class,
                reason: lock.reason_tag.clone(),
            });
        }
        if !self.envelope.contains(stimulus) {
            return Err(GovernanceError::OutsideEnvelope);
        }
        // Finding 4: daily-dose cap + inter-sitting cooldown. Same-timestamp
        // calibration sub-sessions are one sitting and pass freely; a new
        // sitting must clear both the cooldown and the daily cap.
        if let Err(v) = self
            .safety_state
            .check_admission(timestamp_ms, &self.dose_limits)
        {
            return Err(match v {
                DoseViolation::DailyLimit { sittings, max } => {
                    GovernanceError::DailyDoseLimit { sittings, max }
                }
                DoseViolation::Cooldown {
                    elapsed_minutes,
                    required_minutes,
                } => GovernanceError::CooldownActive {
                    elapsed_minutes,
                    required_minutes,
                },
            });
        }

        let idx = self.next_index;
        self.next_index += 1;
        // Commit this sitting to the dose ledger (calibration steps included).
        self.safety_state.record_session(timestamp_ms);

        // --- Observe (simulated) aggregate response (the scoring source). ---
        let mut resp = sim.simulate(latent, state, stimulus, idx);
        if self.mode == TrialMode::Sham {
            // Blinding: no entrainment is actually delivered.
            resp.eeg.gamma_power_gain *= 0.05;
            resp.eeg.phase_locking_value *= 0.05;
        }

        // --- Finding 5: per-tick safety monitor over the WHOLE session. Every
        // tick is evaluated and the monitor latches; a mid-session latch
        // truncates the session at that tick. A clean session produces no
        // events (byte-identical witness to the prior single-summary path,
        // since for a clean run the only tick signal is the unchanged sensor
        // confidence). This makes the <500 ms stop-latency contract apply to
        // the integrated path, at least in simulation.
        let mut monitor = SafetyMonitor::new(self.confidence_floor);
        let ticks = sim.session_ticks(latent, state, stimulus, idx);
        let n_ticks = ticks.len().max(1);
        let mut safety_events = Vec::new();
        let mut stop_tick: Option<usize> = None;
        for (i, t) in ticks.iter().enumerate() {
            if let Some(stop) = monitor.evaluate(SafetyTick {
                adverse: t.adverse,
                sensor_confidence: t.sensor_confidence,
                stimulus_in_envelope: true,
            }) {
                safety_events.push(stop);
                stop_tick = Some(i);
                break;
            }
        }
        let safety_pass = !safety_events.iter().any(StopReason::is_safety_stop);
        let adverse_event = resp.adverse_event
            || safety_events
                .iter()
                .any(|s| matches!(s, StopReason::AdverseEvent(_)));

        // The stimulus *as delivered*: a mid-session latch truncates the
        // duration to the completed fraction (Finding 5). For a clean session
        // this equals the planned stimulus, so the witness is unchanged.
        let recorded_stimulus = if let Some(t) = stop_tick {
            let fraction = (t + 1) as f64 / n_ticks as f64;
            let mut truncated = *stimulus;
            truncated.duration_minutes = stimulus.duration_minutes * fraction;
            self.envelope.clamp(truncated)
        } else {
            *stimulus
        };

        // Finding 2: terminate-and-lock — engage the latched lock if the stop
        // warrants it (persisted, so a new governor for this participant also
        // refuses until acknowledged).
        if let Some(stop) = safety_events.iter().find(|s| s.is_safety_stop()) {
            if let Some(class) = lock_class_for(stop) {
                self.safety_state
                    .engage_lock(class, stop_tag(stop), timestamp_ms);
            }
        }

        // --- Score the session. ---
        let subjective = SubjectiveReport {
            comfort: resp.comfort,
            fatigue: 0.2,
        };
        let score = self.objective.score(&ScoreInputs {
            stimulus,
            ruview: &resp.ruview,
            eeg: Some(&resp.eeg),
            subjective: &subjective,
            adverse_event_risk: if adverse_event { 1.0 } else { 0.0 },
        });

        // --- Feed the optimizer only when the session was safe. ---
        if safety_pass {
            self.optimizer.observe(stimulus.frequency_hz, score);
        }

        // --- Update RuVector memory + drift detection (ADR-250 §10 item 4). ---
        self.response.update(&SessionObservation {
            stimulus: recorded_stimulus,
            ruview: resp.ruview,
            eeg: Some(resp.eeg),
            subjective,
            safety_pass,
            adverse_event,
        });
        self.drift_status = self.drift.update(&self.response.as_array());

        // --- Recommend next frequency for the record. ---
        let next = self.optimizer.recommend(&self.envelope, stimulus);

        // --- Witnessed audit record. ---
        let record = SessionBuilder::new(
            self.person_id.clone(),
            self.versions.clone(),
            timestamp_ms,
            recorded_stimulus,
            resp.ruview,
            subjective,
            Outcome {
                entrainment_score: score,
                safety_pass,
                recommended_next_frequency_hz: next.stimulus.frequency_hz,
            },
        )
        .with_eeg(resp.eeg)
        .with_safety_events(safety_events)
        .finalize();

        self.audit.push(record.clone());
        Ok(record)
    }
}

#[cfg(test)]
#[path = "ruflo_tests.rs"]
mod tests;
