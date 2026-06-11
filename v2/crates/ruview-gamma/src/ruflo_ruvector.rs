//! RuVector cohort bridge for [`RufloGovernor`] (ADR-250 §10 items 3/6).
//!
//! Split out of `ruflo.rs` to keep it under 500 lines. This is a child module
//! of `ruflo`, so it retains access to the governor's private fields.

use super::RufloGovernor;
use crate::ruvector::{AnonymizedProfile, ProfileStore};

impl RufloGovernor {
    /// Seed the optimizer from a cohort of anonymized similar responders
    /// (ADR-250 §10 item 3): the `k` nearest profiles' frequency responses enter
    /// as **down-weighted pseudo-observations**, shaping where the optimizer
    /// looks first without ever counting as this person's measured data. Returns
    /// how many priors were installed.
    ///
    /// Honors the privacy k-floor [`RufloGovernor::MIN_COHORT_PROFILES`]: a
    /// cohort smaller than that yields no priors at all.
    pub fn seed_from_cohort(&mut self, store: &ProfileStore, k: usize) -> usize {
        if store.len() < Self::MIN_COHORT_PROFILES {
            return 0;
        }
        let query = self.response.as_array();
        let priors = store.warm_start_prior(&query, k, self.optimizer.noise_var);
        for p in &priors {
            // Only frequencies inside this participant's envelope are usable.
            if p.frequency_hz >= self.envelope.min_hz() && p.frequency_hz <= self.envelope.max_hz()
            {
                self.optimizer
                    .observe_prior(p.frequency_hz, p.expected_score, p.noise_var);
            }
        }
        priors.len()
    }

    /// Export this participant as an anonymized profile for the cohort store
    /// (ADR-250 §10 items 3/6). Carries the one-way hashed tag, the response
    /// vector, and per-frequency scores from **safe sessions only** — never the
    /// `person_id`, never raw sensor data.
    pub fn export_anonymized_profile(&self) -> AnonymizedProfile {
        let frequency_scores: Vec<(f64, f64)> = self
            .audit
            .iter()
            .filter(|r| r.outcome.safety_pass)
            .map(|r| (r.stimulus.frequency_hz, r.outcome.entrainment_score))
            .collect();
        AnonymizedProfile {
            profile_tag: AnonymizedProfile::tag_for(&self.person_id),
            vector: self.response.as_array(),
            frequency_scores,
        }
    }
}
