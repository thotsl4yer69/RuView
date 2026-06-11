//! Tests for the RuFlo governor. Split out of `ruflo.rs` (kept under 500 lines);
//! this is a child module of `ruflo`, so it retains access to private fields.

use super::*;
use crate::safety::AdverseEvent;
use crate::simulator::{FaultSchedule, InjectedFault};

fn governor() -> RufloGovernor {
    RufloGovernor::enroll(
        "subject-A",
        SafetyEnvelope::conservative(),
        &[],
        Consent::Granted,
    )
    .unwrap()
}

#[test]
fn enroll_refuses_without_consent() {
    let r = RufloGovernor::enroll("x", SafetyEnvelope::conservative(), &[], Consent::Withdrawn);
    assert_eq!(r.err(), Some(GovernanceError::NoConsent));
}

#[test]
fn enroll_refuses_excluded_condition() {
    let r = RufloGovernor::enroll(
        "x",
        SafetyEnvelope::conservative(),
        &[ExclusionCondition::EpilepsyOrSeizureHistory],
        Consent::Granted,
    );
    assert!(matches!(r, Err(GovernanceError::Excluded(_))));
}

#[test]
fn enroll_requires_supervision_for_migraine() {
    let r = RufloGovernor::enroll(
        "x",
        SafetyEnvelope::conservative(),
        &[ExclusionCondition::SevereMigraineSensitivity],
        Consent::Granted,
    );
    assert!(matches!(r, Err(GovernanceError::SupervisionRequired(_))));
}

#[test]
fn run_session_refuses_out_of_envelope_stimulus() {
    let mut g = governor();
    let sim = ResponseSimulator::new(1);
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    let mut bad = StimulusParameters::prior();
    bad.frequency_hz = 60.0;
    let r = g.run_session(&sim, &latent, &state, &bad, 0);
    assert_eq!(r.err(), Some(GovernanceError::OutsideEnvelope));
    assert!(g.audit_log().is_empty());
}

#[test]
fn withdrawn_consent_blocks_further_sessions() {
    let mut g = governor();
    let sim = ResponseSimulator::new(1);
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    g.run_session(&sim, &latent, &state, &StimulusParameters::prior(), 0)
        .unwrap();
    g.withdraw_consent();
    let r = g.run_session(&sim, &latent, &state, &StimulusParameters::prior(), 1);
    assert_eq!(r.err(), Some(GovernanceError::NoConsent));
}

#[test]
fn calibration_then_recommendation_lands_near_latent_peak() {
    let mut g = governor();
    let sim = ResponseSimulator::new(99);
    let latent = LatentPerson::from_id("subject-peak");
    let state = RuViewState::calm_baseline();
    g.run_calibration(&sim, &latent, &state, 5.0, 0).unwrap();
    let rec = g.recommend(&StimulusParameters::prior());
    assert!(g.envelope.contains(&rec.stimulus));
    // Optimizer should prefer a frequency within ±2 Hz of the true peak
    // (calibration is short/noisy; ±2 Hz is a robust bound for the test).
    assert!((rec.stimulus.frequency_hz - latent.peak_hz).abs() <= 2.0);
}

#[test]
fn every_session_is_witnessed_and_logged() {
    let mut g = governor();
    let sim = ResponseSimulator::new(5);
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    g.run_calibration(&sim, &latent, &state, 5.0, 0).unwrap();
    assert_eq!(g.audit_log().len(), 9); // 36..44 Hz
    for rec in g.audit_log() {
        assert_eq!(rec.session_hash.len(), 64); // hex SHA-256
    }
}

#[test]
fn sham_mode_suppresses_entrainment() {
    let latent = LatentPerson::from_id("subject-strong");
    let state = RuViewState::calm_baseline();
    let sim = ResponseSimulator::new(11);
    let mut peak = StimulusParameters::prior();
    peak.frequency_hz = (latent.peak_hz * 10.0).round() / 10.0;
    peak.frequency_hz = peak.frequency_hz.clamp(36.0, 44.0);

    let mut open = governor();
    let open_rec = open.run_session(&sim, &latent, &state, &peak, 0).unwrap();

    let mut sham = governor();
    sham.set_mode(TrialMode::Sham);
    let sham_rec = sham.run_session(&sim, &latent, &state, &peak, 0).unwrap();

    let open_g = open_rec.eeg_optional.unwrap().gamma_power_gain;
    let sham_g = sham_rec.eeg_optional.unwrap().gamma_power_gain;
    assert!(sham_g < open_g);
}

#[test]
fn clinician_report_uses_only_allowed_claim() {
    let g = governor();
    assert_eq!(g.clinician_report().claim, PRODUCT_CLAIM);
    assert!(!PRODUCT_CLAIM.to_lowercase().contains("alzheimer"));
    assert!(!PRODUCT_CLAIM.to_lowercase().contains("treat"));
}

// ====================================================================
// Safety-hardening adversarial tests (2026-06-11 review).
// ====================================================================

/// Finding 1: a safety stop is a control-flow event — the sweep halts at the
/// stopping step rather than running every remaining frequency.
#[test]
fn sweep_halts_mid_calibration_on_safety_stop() {
    let env = SafetyEnvelope::conservative();
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    // Low sensor confidence at calibration step 3 (tick 0) → confidence stop.
    // (A confidence stop is retryable and does NOT lock the participant.)
    let sim = ResponseSimulator::with_fault(
        1,
        FaultSchedule {
            at_session_index: 3,
            at_tick: 0,
            fault: InjectedFault::LowConfidence(0.1),
        },
    );
    let mut g = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted).unwrap();
    g.run_calibration(&sim, &latent, &state, 5.0, 0).unwrap();
    // Steps 0,1,2 passed, step 3 stopped → 4 records (not the full 9-step sweep).
    assert_eq!(g.audit_log().len(), 4);
    assert!(!g.audit_log().last().unwrap().outcome.safety_pass);
    assert!(!g.is_locked());
}

/// Findings 1 & 2: an adverse-event stop terminates the session AND latches the
/// governor lock; a fresh governor loaded with the persisted state still
/// refuses until an explicit operator acknowledgment (which is itself audited).
#[test]
fn adverse_event_terminates_and_locks_across_instances_until_acknowledged() {
    let env = SafetyEnvelope::conservative();
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    let stim = StimulusParameters::prior(); // 10 min → 10 ticks
    let sim = ResponseSimulator::with_fault(
        1,
        FaultSchedule {
            at_session_index: 0,
            at_tick: 3,
            fault: InjectedFault::Adverse(AdverseEvent::SeizureLikeSymptom),
        },
    );
    let mut g = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted).unwrap();
    // The session still returns its witnessed record (Finding 1), marked stopped.
    let rec = g.run_session(&sim, &latent, &state, &stim, 0).unwrap();
    assert!(!rec.outcome.safety_pass);
    // …and the governor is now latched-locked (Finding 2 terminate-and-lock).
    assert!(g.is_locked());
    assert_eq!(
        g.safety_state().lock_record().unwrap().class,
        LockClass::SeizureLike
    );

    // Same governor refuses to start any further session.
    let clean = ResponseSimulator::new(2);
    let err = g
        .run_session(&clean, &latent, &state, &stim, 4_000_000)
        .unwrap_err();
    assert!(matches!(err, GovernanceError::ParticipantLocked { .. }));

    // Persistence: a NEW governor loaded with the serialized state also refuses.
    let persisted = g.safety_state().clone();
    let mut g2 = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted)
        .unwrap()
        .with_safety_state(persisted);
    assert!(matches!(
        g2.run_session(&clean, &latent, &state, &stim, 4_000_000),
        Err(GovernanceError::ParticipantLocked { .. })
    ));

    // Unlock with acknowledgment writes an audit record and lifts the lock.
    assert!(matches!(
        g2.unlock_with_acknowledgment("not yet locked?", 0),
        Ok(())
    ));
    assert!(!g2.is_locked());
    assert_eq!(g2.safety_state().unlock_audit().len(), 1);
    assert_eq!(
        g2.safety_state().unlock_audit()[0].cleared_class,
        LockClass::SeizureLike
    );
    // A redundant unlock is refused (nothing locked).
    assert!(matches!(
        g2.unlock_with_acknowledgment("again", 1),
        Err(GovernanceError::NotLocked)
    ));
    // Now a clean session runs (far enough out to clear the cooldown).
    assert!(g2
        .run_session(&clean, &latent, &state, &stim, 4_000_000)
        .is_ok());
}

/// Finding 4: inter-session cooldown blocks a too-soon second sitting.
#[test]
fn cooldown_blocks_a_too_soon_session() {
    let env = SafetyEnvelope::conservative();
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    let stim = StimulusParameters::prior();
    let sim = ResponseSimulator::new(1);
    let mut g = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted).unwrap();
    g.run_session(&sim, &latent, &state, &stim, 0).unwrap();
    // 30 min later — inside the 60 min cooldown.
    let err = g
        .run_session(&sim, &latent, &state, &stim, 30 * 60 * 1000)
        .unwrap_err();
    assert!(matches!(err, GovernanceError::CooldownActive { .. }));
}

/// Finding 4: the daily-dose cap is enforced across governor instances via the
/// persisted ledger; calibration sittings count toward the budget.
#[test]
fn daily_dose_cap_enforced_across_governor_instances() {
    let env = SafetyEnvelope::conservative();
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    let stim = StimulusParameters::prior();
    let sim = ResponseSimulator::new(1);
    let hour = 60 * 60 * 1000u64;
    let mut g = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted).unwrap();
    // Four sittings, each 2 h apart (clears cooldown), all within 24 h.
    for i in 0..4u64 {
        g.run_session(&sim, &latent, &state, &stim, i * 2 * hour)
            .unwrap();
    }
    // Persist + reload into a NEW governor instance.
    let persisted = g.safety_state().clone();
    let mut g2 = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted)
        .unwrap()
        .with_safety_state(persisted);
    // The fifth sitting within 24 h is refused by the reloaded instance.
    let err = g2
        .run_session(&sim, &latent, &state, &stim, 9 * hour)
        .unwrap_err();
    assert!(matches!(
        err,
        GovernanceError::DailyDoseLimit {
            sittings: 5,
            max: 4
        }
    ));
}

/// A calibration sweep delivered at a single timestamp is ONE dose unit, so it
/// is not a backdoor around the daily cap (Finding 4 documented invariant).
#[test]
fn calibration_sweep_is_one_dose_sitting() {
    let env = SafetyEnvelope::conservative();
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    let sim = ResponseSimulator::new(5);
    let mut g = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted).unwrap();
    g.run_calibration(&sim, &latent, &state, 5.0, 1_700_000_000_000)
        .unwrap();
    assert_eq!(g.audit_log().len(), 9);
    assert_eq!(g.safety_state().sittings_in_window(1_700_000_000_000), 1);
}

/// Finding 5: a per-tick latch mid-session truncates the recorded (delivered)
/// stimulus to the completed fraction.
#[test]
fn mid_session_tick_latch_truncates_the_session() {
    let env = SafetyEnvelope::conservative();
    let latent = LatentPerson::from_id("subject-A");
    let state = RuViewState::calm_baseline();
    let stim = StimulusParameters::prior(); // 10 min → 10 ticks
                                            // Adverse at tick 2 → delivered fraction (2+1)/10 = 0.3 → 3.0 min.
    let sim = ResponseSimulator::with_fault(
        9,
        FaultSchedule {
            at_session_index: 0,
            at_tick: 2,
            fault: InjectedFault::Adverse(AdverseEvent::Headache),
        },
    );
    let mut g = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted).unwrap();
    let rec = g.run_session(&sim, &latent, &state, &stim, 0).unwrap();
    assert!(!rec.outcome.safety_pass);
    assert!((rec.stimulus.duration_minutes - 3.0).abs() < 1e-9);
    assert!(rec.stimulus.duration_minutes < stim.duration_minutes);
}

/// Minor: `seed_from_cohort` honors the privacy k-floor (≥ 3 distinct profiles).
#[test]
fn seed_from_cohort_respects_min_cohort_floor() {
    use crate::ruvector::{AnonymizedProfile, ProfileStore, VECTOR_DIM};
    let env = SafetyEnvelope::conservative();
    let mk = |tag: &str| {
        let mut vector = [0.5; VECTOR_DIM];
        vector[5] = 13.0; // breathing_rate in range
        vector[11] = 39.0; // stimulus_frequency
        AnonymizedProfile {
            profile_tag: tag.into(),
            vector,
            frequency_scores: vec![(39.0, 0.8)],
        }
    };
    let mut store = ProfileStore::new();
    store.upsert(mk("a"));
    store.upsert(mk("b"));
    let mut g = RufloGovernor::enroll("subject-A", env, &[], Consent::Granted).unwrap();
    // Below the floor → no priors consumed.
    assert_eq!(g.seed_from_cohort(&store, 2), 0);
    // At/above the floor → priors may be consumed.
    store.upsert(mk("c"));
    assert!(g.seed_from_cohort(&store, 3) > 0);
}
