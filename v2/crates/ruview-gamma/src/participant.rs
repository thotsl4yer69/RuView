//! Cross-session participant safety state — the latched governor lock and the
//! daily-dose / cooldown ledger (Findings 2 & 4, 2026-06-11 safety review).
//!
//! ADR-250 §8 mandates a **hard terminate-and-lock on adverse events**. A
//! [`SafetyMonitor`](crate::safety::SafetyMonitor) latches *within* a session,
//! but a fresh monitor is built per session, so on its own it cannot stop a new
//! session from starting after a seizure-like stop. [`ParticipantSafetyState`]
//! closes that gap: it is the **persisted** safety record for one participant,
//! holding a latched [`LockRecord`] and the session ledger that backs the dose
//! cap and cooldown. Serialize it into the session store and a *new*
//! [`RufloGovernor`](crate::ruflo::RufloGovernor) for the same participant will
//! still refuse until [`ParticipantSafetyState::unlock`] is called with an
//! operator acknowledgment (which itself writes an audit record).
//!
//! ## Lock-class → lock mapping (documented invariant)
//!
//! | Stop reason | Lock? | Class |
//! |-------------|-------|-------|
//! | adverse: seizure-like | **yes** | [`LockClass::SeizureLike`] |
//! | adverse: abnormal distress | **yes** | [`LockClass::Distress`] |
//! | adverse: headache / dizziness / nausea / agitation / visual discomfort | **yes** | [`LockClass::AdverseEvent`] |
//! | adverse: user-stop request | no | — (the participant chose to stop; not a clinical lock) |
//! | sensor confidence below floor | no | — (unverifiable, retryable) |
//! | protocol outside envelope | no | — (pre-empted before delivery) |
//! | completed | no | — |
//!
//! A locked participant is a *fail-closed* state: every lock-warranting stop
//! engages the lock; only an explicit human acknowledgment lifts it.

use serde::{Deserialize, Serialize};

use crate::safety::{AdverseEvent, StopReason};

/// The severity class of a latched lock (drives operator messaging and audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockClass {
    /// A general adverse event (headache, dizziness, nausea, agitation, visual
    /// discomfort).
    AdverseEvent,
    /// A seizure-like symptom — the most serious class.
    SeizureLike,
    /// Abnormal distress.
    Distress,
}

impl LockClass {
    pub fn tag(self) -> &'static str {
        match self {
            LockClass::AdverseEvent => "adverse_event",
            LockClass::SeizureLike => "seizure_like",
            LockClass::Distress => "distress",
        }
    }
}

/// The lock class a safety stop warrants, or `None` if the stop does not lock.
/// This is the single source of truth for the table in the module docs.
pub fn lock_class_for(reason: &StopReason) -> Option<LockClass> {
    match reason {
        StopReason::AdverseEvent(ev) => match ev {
            AdverseEvent::SeizureLikeSymptom => Some(LockClass::SeizureLike),
            AdverseEvent::AbnormalDistress => Some(LockClass::Distress),
            // The participant choosing to stop is not a clinical adverse event
            // requiring human acknowledgment to ever resume.
            AdverseEvent::UserStopRequest => None,
            AdverseEvent::Headache
            | AdverseEvent::Dizziness
            | AdverseEvent::Nausea
            | AdverseEvent::Agitation
            | AdverseEvent::VisualDiscomfort => Some(LockClass::AdverseEvent),
        },
        // Operational stops fail the session closed but do not latch a
        // cross-session lock (they are retryable / pre-empted, not adverse).
        StopReason::SensorConfidenceBelowFloor { .. }
        | StopReason::ProtocolOutsideEnvelope
        | StopReason::Completed => None,
    }
}

/// A short, stable tag for a stop reason (for audit / error messages).
pub fn stop_tag(reason: &StopReason) -> String {
    match reason {
        StopReason::Completed => "completed".to_string(),
        StopReason::AdverseEvent(ev) => format!("adverse:{}", ev.tag()),
        StopReason::SensorConfidenceBelowFloor { .. } => {
            "sensor_confidence_below_floor".to_string()
        }
        StopReason::ProtocolOutsideEnvelope => "protocol_outside_envelope".to_string(),
    }
}

/// A latched lock on a participant (ADR-250 §8 terminate-and-lock). Persisted so
/// a new governor instance also honors it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockRecord {
    pub class: LockClass,
    /// The stop reason that engaged the lock (see [`stop_tag`]).
    pub reason_tag: String,
    /// Caller-supplied epoch milliseconds at which the lock engaged.
    pub locked_at_ms: u64,
}

/// An auditable unlock acknowledgment (ADR-250 §11 audit trail).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlockRecord {
    /// The operator's free-text justification (e.g. clinician sign-off).
    pub operator_note: String,
    /// Caller-supplied epoch milliseconds at which the unlock occurred.
    pub unlocked_at_ms: u64,
    /// The class that was cleared (for the audit record).
    pub cleared_class: LockClass,
}

/// Daily-dose and cooldown policy (Finding 4). Conservative consts by default;
/// the absolute *envelope* bounds are non-negotiable, but dosing is a wall-clock
/// policy the deterministic core can only enforce against caller-supplied
/// timestamps, so it is configurable (with a conservative default).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DoseLimits {
    /// Maximum distinct **sittings** in any rolling 24 h window.
    pub max_sittings_per_24h: usize,
    /// Minimum gap between distinct sittings, in minutes.
    pub min_inter_sitting_minutes: f64,
}

impl DoseLimits {
    /// Conservative default cap (Finding 4 example): ≤ 4 sittings/day.
    pub const MAX_SITTINGS_PER_24H: usize = 4;
    /// Conservative default cooldown (Finding 4 example): ≥ 60 min between sittings.
    pub const MIN_INTER_SITTING_MINUTES: f64 = 60.0;
    /// The rolling-window width in milliseconds (24 h).
    pub const ROLLING_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;

    /// The enforced-by-default conservative policy.
    pub fn conservative() -> Self {
        Self {
            max_sittings_per_24h: Self::MAX_SITTINGS_PER_24H,
            min_inter_sitting_minutes: Self::MIN_INTER_SITTING_MINUTES,
        }
    }

    /// Research/simulation policy that disables dose + cooldown. **Not for real
    /// deployments** — simulation compresses many sessions into a tiny time
    /// window, which is incompatible with wall-clock dosing; the conservative
    /// default governs the clinic/hardware path.
    pub fn permissive_for_simulation() -> Self {
        Self {
            max_sittings_per_24h: usize::MAX,
            min_inter_sitting_minutes: 0.0,
        }
    }
}

impl Default for DoseLimits {
    fn default() -> Self {
        Self::conservative()
    }
}

/// Why an admission (start-of-session) check refused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DoseViolation {
    /// Too many distinct sittings within the rolling 24 h window.
    DailyLimit { sittings: usize, max: usize },
    /// Not enough time has elapsed since the previous sitting.
    Cooldown {
        elapsed_minutes: f64,
        required_minutes: f64,
    },
}

/// The persisted per-participant safety state: a latched lock plus the sitting
/// ledger that backs dose and cooldown. `Default` is the clean state.
///
/// A **sitting** is identified by its `timestamp_ms`. Sub-sessions delivered at
/// the *same* timestamp (a calibration sweep) are one sitting and one dose unit;
/// they never trip the inter-sitting cooldown against each other. Distinct
/// timestamps are distinct sittings, each counted toward the daily cap — so
/// calibration is **not** a backdoor around the dose budget.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParticipantSafetyState {
    lock: Option<LockRecord>,
    /// Every session timestamp that has run (calibration steps included).
    sittings: Vec<u64>,
    /// Every unlock acknowledgment, oldest first (audit trail).
    unlock_audit: Vec<UnlockRecord>,
}

impl ParticipantSafetyState {
    /// `true` if the participant is latched-locked.
    pub fn is_locked(&self) -> bool {
        self.lock.is_some()
    }

    /// The active lock record, if any.
    pub fn lock_record(&self) -> Option<&LockRecord> {
        self.lock.as_ref()
    }

    /// The unlock-acknowledgment audit trail.
    pub fn unlock_audit(&self) -> &[UnlockRecord] {
        &self.unlock_audit
    }

    /// Engage the lock (idempotent / latched): the *first* lock-warranting stop
    /// wins and is never overwritten or downgraded by later events.
    pub fn engage_lock(&mut self, class: LockClass, reason_tag: String, ts_ms: u64) {
        if self.lock.is_none() {
            self.lock = Some(LockRecord {
                class,
                reason_tag,
                locked_at_ms: ts_ms,
            });
        }
    }

    /// Lift the lock with an explicit operator acknowledgment, writing an audit
    /// record. No-op (but still audited as a redundant ack) if already unlocked?
    /// — No: we only audit a real unlock. Returns the cleared class, or `None`
    /// if there was nothing locked.
    pub fn unlock(&mut self, operator_note: impl Into<String>, ts_ms: u64) -> Option<LockClass> {
        let cleared = self.lock.take()?;
        self.unlock_audit.push(UnlockRecord {
            operator_note: operator_note.into(),
            unlocked_at_ms: ts_ms,
            cleared_class: cleared.class,
        });
        Some(cleared.class)
    }

    /// Record a session that actually ran (for dose / cooldown accounting).
    pub fn record_session(&mut self, ts_ms: u64) {
        self.sittings.push(ts_ms);
    }

    /// Distinct sittings recorded within the rolling 24 h window ending at `now_ms`.
    pub fn sittings_in_window(&self, now_ms: u64) -> usize {
        let lo = now_ms.saturating_sub(DoseLimits::ROLLING_WINDOW_MS);
        self.sittings
            .iter()
            .copied()
            .filter(|&t| t >= lo && t <= now_ms)
            .collect::<std::collections::BTreeSet<u64>>()
            .len()
    }

    /// Admission gate for a session about to start at `now_ms`. Same-timestamp
    /// sub-sessions (calibration sweep) pass freely; a *new* sitting must clear
    /// both the cooldown and the daily cap.
    pub fn check_admission(&self, now_ms: u64, limits: &DoseLimits) -> Result<(), DoseViolation> {
        // Cooldown vs the most recent *earlier* sitting (same-timestamp siblings
        // do not count — they are the same sitting).
        if limits.min_inter_sitting_minutes > 0.0 {
            if let Some(prev) = self.sittings.iter().copied().filter(|&t| t < now_ms).max() {
                let elapsed_minutes = (now_ms - prev) as f64 / 60_000.0;
                if elapsed_minutes < limits.min_inter_sitting_minutes {
                    return Err(DoseViolation::Cooldown {
                        elapsed_minutes,
                        required_minutes: limits.min_inter_sitting_minutes,
                    });
                }
            }
        }
        // Daily cap: distinct sittings in the window, including this prospective one.
        if limits.max_sittings_per_24h != usize::MAX {
            let lo = now_ms.saturating_sub(DoseLimits::ROLLING_WINDOW_MS);
            let mut distinct: std::collections::BTreeSet<u64> = self
                .sittings
                .iter()
                .copied()
                .filter(|&t| t >= lo && t <= now_ms)
                .collect();
            distinct.insert(now_ms);
            if distinct.len() > limits.max_sittings_per_24h {
                return Err(DoseViolation::DailyLimit {
                    sittings: distinct.len(),
                    max: limits.max_sittings_per_24h,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::AdverseEvent;

    #[test]
    fn seizure_and_distress_map_to_their_classes() {
        assert_eq!(
            lock_class_for(&StopReason::AdverseEvent(AdverseEvent::SeizureLikeSymptom)),
            Some(LockClass::SeizureLike)
        );
        assert_eq!(
            lock_class_for(&StopReason::AdverseEvent(AdverseEvent::AbnormalDistress)),
            Some(LockClass::Distress)
        );
        assert_eq!(
            lock_class_for(&StopReason::AdverseEvent(AdverseEvent::Headache)),
            Some(LockClass::AdverseEvent)
        );
    }

    #[test]
    fn user_stop_and_operational_stops_do_not_lock() {
        assert_eq!(
            lock_class_for(&StopReason::AdverseEvent(AdverseEvent::UserStopRequest)),
            None
        );
        assert_eq!(
            lock_class_for(&StopReason::SensorConfidenceBelowFloor {
                value: 0.1,
                floor: 0.5
            }),
            None
        );
        assert_eq!(lock_class_for(&StopReason::ProtocolOutsideEnvelope), None);
        assert_eq!(lock_class_for(&StopReason::Completed), None);
    }

    #[test]
    fn lock_is_latched_first_wins() {
        let mut s = ParticipantSafetyState::default();
        s.engage_lock(LockClass::Distress, "adverse:abnormal_distress".into(), 100);
        s.engage_lock(LockClass::AdverseEvent, "adverse:headache".into(), 200);
        assert_eq!(s.lock_record().unwrap().class, LockClass::Distress);
        assert_eq!(s.lock_record().unwrap().locked_at_ms, 100);
    }

    #[test]
    fn unlock_clears_and_audits() {
        let mut s = ParticipantSafetyState::default();
        s.engage_lock(LockClass::SeizureLike, "adverse:seizure_like".into(), 10);
        assert!(s.is_locked());
        let cleared = s.unlock("clinician reviewed; cleared to resume", 999);
        assert_eq!(cleared, Some(LockClass::SeizureLike));
        assert!(!s.is_locked());
        assert_eq!(s.unlock_audit().len(), 1);
        assert_eq!(s.unlock_audit()[0].cleared_class, LockClass::SeizureLike);
        // A redundant unlock is a no-op and is not audited.
        assert_eq!(s.unlock("again", 1000), None);
        assert_eq!(s.unlock_audit().len(), 1);
    }

    #[test]
    fn same_timestamp_sweep_is_one_sitting_no_cooldown() {
        let mut s = ParticipantSafetyState::default();
        let limits = DoseLimits::conservative();
        let t0 = 1_700_000_000_000u64;
        for _ in 0..9 {
            assert!(s.check_admission(t0, &limits).is_ok());
            s.record_session(t0);
        }
        assert_eq!(s.sittings_in_window(t0), 1);
    }

    #[test]
    fn cooldown_blocks_a_too_soon_second_sitting() {
        let mut s = ParticipantSafetyState::default();
        let limits = DoseLimits::conservative();
        let t0 = 0u64;
        s.check_admission(t0, &limits).unwrap();
        s.record_session(t0);
        // 30 min later — inside the 60 min cooldown.
        let t1 = 30 * 60 * 1000;
        assert!(matches!(
            s.check_admission(t1, &limits),
            Err(DoseViolation::Cooldown { .. })
        ));
        // 61 min later — clears the cooldown.
        let t2 = 61 * 60 * 1000;
        assert!(s.check_admission(t2, &limits).is_ok());
    }

    #[test]
    fn daily_cap_blocks_the_fifth_sitting() {
        let mut s = ParticipantSafetyState::default();
        let limits = DoseLimits::conservative();
        let hour = 60 * 60 * 1000u64;
        // Four sittings, each > 60 min apart, all within 24 h.
        for i in 0..4u64 {
            let t = i * 2 * hour;
            s.check_admission(t, &limits).unwrap();
            s.record_session(t);
        }
        // A fifth within the same 24 h window is over the cap.
        let t5 = 9 * hour;
        assert!(matches!(
            s.check_admission(t5, &limits),
            Err(DoseViolation::DailyLimit {
                sittings: 5,
                max: 4
            })
        ));
    }

    #[test]
    fn state_roundtrips_through_json() {
        let mut s = ParticipantSafetyState::default();
        s.engage_lock(LockClass::Distress, "adverse:abnormal_distress".into(), 7);
        s.record_session(7);
        let json = serde_json::to_string(&s).unwrap();
        let back: ParticipantSafetyState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        assert!(back.is_locked());
    }
}
