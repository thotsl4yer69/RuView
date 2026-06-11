//! Clinician export for [`RufloGovernor`] (ADR-250 §11 responsibility 9, §17).
//!
//! Split out of `ruflo.rs` to keep that file under 500 lines. This is a child
//! module of `ruflo`, so it retains access to the governor's private fields.

use super::{RufloGovernor, PRODUCT_CLAIM};

/// Clinician-facing export summary (ADR-250 §11 responsibility 9, §17).
#[derive(Debug, Clone, PartialEq)]
pub struct ClinicianReport {
    pub person_id: String,
    pub n_sessions: usize,
    pub n_safety_stops: usize,
    pub best_frequency_hz: Option<f64>,
    pub mean_entrainment: f64,
    pub adverse_event_recorded: bool,
    /// Whether the participant is currently latched-locked (Finding 2).
    pub locked: bool,
    pub claim: &'static str,
}

impl RufloGovernor {
    /// Build the clinician export (ADR-250 §11 responsibility 9).
    pub fn clinician_report(&self) -> ClinicianReport {
        let n = self.audit.len();
        let n_stops = self
            .audit
            .iter()
            .flat_map(|r| &r.safety_events)
            .filter(|e| e.is_safety_stop())
            .count();
        let mean = if n > 0 {
            self.audit
                .iter()
                .map(|r| r.outcome.entrainment_score)
                .sum::<f64>()
                / n as f64
        } else {
            0.0
        };
        ClinicianReport {
            person_id: self.person_id.clone(),
            n_sessions: n,
            n_safety_stops: n_stops,
            best_frequency_hz: self.optimizer.best().map(|(f, _)| f),
            mean_entrainment: mean,
            adverse_event_recorded: self.response.adverse_event_flag >= 1.0,
            locked: self.safety_state.is_locked(),
            claim: PRODUCT_CLAIM,
        }
    }
}
