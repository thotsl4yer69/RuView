//! Hardware-in-the-loop (HIL) acceptance contract — Milestone "device harness"
//! (ADR-250 §17, §21 M2).
//!
//! The software core is proven against the deterministic simulator; the next
//! acceptance milestone is **real hardware**: an LED + speaker actuator (e.g.
//! driven by an ESP32) plus the safety stop path. This module defines the
//! *contract* every actuator integration must satisfy and a verifier that
//! grades a captured [`HilMeasurement`] against fixed targets. It does **not**
//! talk to hardware — the firmware/driver records the measurements and submits
//! them here, keeping this crate a deterministic, dependency-light leaf.
//!
//! | Test | Target |
//! |------|--------|
//! | LED frequency accuracy | ±0.1 Hz |
//! | Worst-case frequency error over the session window | ±0.1 Hz |
//! | Worst-case half-period jitter over the session window | ≤ 500 µs |
//! | Audio-visual sync drift | < 5 ms |
//! | Stop signal → actuator off | < 100 ms |
//! | Session-hash reproducibility | 100% |
//! | EEG entrainment lift vs fixed 40 Hz | ≥ 20% |
//!
//! Every criterion fails closed: a NaN/non-finite measurement, an impossible
//! hash count (`reproduced > total`), or an empty replay set all grade as
//! FAIL, never as "unknown".

use serde::{Deserialize, Serialize};

/// Fixed HIL targets (ADR-250 §17 acceptance + §18). Constants, not config:
/// these are the bar a device must clear to be called validated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HilTargets {
    /// Max |measured − commanded| LED/flicker frequency, Hz.
    pub max_frequency_error_hz: f64,
    /// Max audio-visual onset skew, milliseconds.
    pub max_av_sync_drift_ms: f64,
    /// Max latency from stop assertion to actuator-off, milliseconds.
    pub max_stop_latency_ms: f64,
    /// Min fraction of replayed sessions whose witness hash reproduced.
    pub min_hash_reproducibility: f64,
    /// Min EEG entrainment lift over fixed 40 Hz, as a fraction.
    pub min_eeg_lift: f64,
    /// Max worst-case half-period jitter over the session window, µs.
    /// Grades delivered regularity, not a point estimate: a bench run must
    /// report the worst half-period deviation it saw, captured at the sync
    /// pin across the whole session.
    pub max_half_period_jitter_us: f64,
}

impl Default for HilTargets {
    fn default() -> Self {
        Self {
            max_frequency_error_hz: 0.1,
            max_av_sync_drift_ms: 5.0,
            max_stop_latency_ms: 100.0,
            min_hash_reproducibility: 1.0,
            min_eeg_lift: 0.20,
            // 500 µs is ~4% of the shortest half-period (11.4 ms @ 44 Hz):
            // generous for ISR latency, tight enough to catch a wobbling timer.
            max_half_period_jitter_us: 500.0,
        }
    }
}

/// A captured bench measurement from a real actuator run. Populated by the
/// firmware/driver test harness (e.g. measuring LED frequency with a photodiode
/// and a logic analyzer, sync with a dual-channel capture, stop latency from
/// GPIO assert to PWM-off).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HilMeasurement {
    /// Commanded frequency the controller asked for (Hz).
    pub commanded_frequency_hz: f64,
    /// Frequency actually measured at the LED (Hz) — session mean.
    pub measured_frequency_hz: f64,
    /// Worst-case |measured − commanded| frequency over the whole session
    /// window (Hz). Graded against the same ±0.1 Hz budget as the mean, so
    /// the bench measures delivered regularity, not a point estimate.
    pub max_abs_freq_error_hz: f64,
    /// Worst-case half-period jitter over the session window (µs), measured
    /// at the sync pin.
    pub max_half_period_jitter_us: f64,
    /// Measured audio-visual onset skew (ms).
    pub av_sync_drift_ms: f64,
    /// Measured stop-assert → actuator-off latency (ms).
    pub stop_latency_ms: f64,
    /// Replayed sessions whose witness hash matched / total replayed.
    pub hashes_reproduced: u32,
    pub hashes_total: u32,
    /// Mean EEG entrainment under the adaptive protocol.
    pub eeg_entrainment_adaptive: f64,
    /// Mean EEG entrainment under fixed 40 Hz (the control arm).
    pub eeg_entrainment_fixed_40hz: f64,
}

/// Per-criterion verdict for a HIL run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HilReport {
    pub frequency_error_hz: f64,
    pub frequency_pass: bool,
    /// Worst-case frequency error over the session window (echoed from the
    /// measurement) and its verdict against the same ±0.1 Hz budget.
    pub worst_frequency_error_hz: f64,
    pub worst_frequency_pass: bool,
    /// Worst-case half-period jitter verdict (delivered regularity).
    pub jitter_pass: bool,
    pub av_sync_pass: bool,
    pub stop_latency_pass: bool,
    pub hash_reproducibility: f64,
    pub hash_pass: bool,
    pub eeg_lift: f64,
    pub eeg_lift_pass: bool,
    /// True only if every HIL criterion passes — the device is bench-validated.
    pub overall_pass: bool,
}

/// Grade a [`HilMeasurement`] against [`HilTargets`].
///
/// Fails closed on bad data: any non-finite (NaN/inf) measurement and any
/// impossible hash count (`hashes_reproduced > hashes_total`) grades as FAIL.
pub fn verify_hil(m: &HilMeasurement, t: &HilTargets) -> HilReport {
    // Session-mean frequency error. A NaN in either operand yields a NaN
    // error; the explicit is_finite() makes the fail-closed intent visible
    // instead of relying on NaN comparison semantics.
    let frequency_error_hz = (m.measured_frequency_hz - m.commanded_frequency_hz).abs();
    let frequency_pass =
        frequency_error_hz.is_finite() && frequency_error_hz <= t.max_frequency_error_hz;

    // Worst-case-over-window regularity: the same ±0.1 Hz budget must hold
    // for the worst excursion, not just the mean. A negative "worst absolute
    // error" is self-contradictory bench data ⇒ fail closed.
    let worst_frequency_error_hz = m.max_abs_freq_error_hz;
    let worst_frequency_pass = worst_frequency_error_hz.is_finite()
        && worst_frequency_error_hz >= 0.0
        && worst_frequency_error_hz <= t.max_frequency_error_hz;

    let jitter_pass = m.max_half_period_jitter_us.is_finite()
        && m.max_half_period_jitter_us >= 0.0
        && m.max_half_period_jitter_us <= t.max_half_period_jitter_us;

    let av_sync_pass =
        m.av_sync_drift_ms.is_finite() && m.av_sync_drift_ms.abs() <= t.max_av_sync_drift_ms;

    // Stop latency must be finite and within bound (a missing/NaN measurement
    // fails closed).
    let stop_latency_pass =
        m.stop_latency_ms.is_finite() && m.stop_latency_ms <= t.max_stop_latency_ms;

    // hashes_reproduced > hashes_total is impossible for honest bench data;
    // treat it as an invalid measurement, never as >100% reproducibility.
    let hash_reproducibility = if m.hashes_total == 0 || m.hashes_reproduced > m.hashes_total {
        0.0 // nothing replayed, or corrupt counts ⇒ unproven ⇒ fail closed
    } else {
        m.hashes_reproduced as f64 / m.hashes_total as f64
    };
    let hash_pass = hash_reproducibility >= t.min_hash_reproducibility;

    let eeg_lift =
        if m.eeg_entrainment_adaptive.is_finite() && m.eeg_entrainment_fixed_40hz.is_finite() {
            let baseline = m.eeg_entrainment_fixed_40hz.max(1e-6);
            (m.eeg_entrainment_adaptive - baseline) / baseline
        } else {
            f64::NAN // propagate so the verdict below fails closed
        };
    let eeg_lift_pass = eeg_lift.is_finite() && eeg_lift >= t.min_eeg_lift;

    let overall_pass = frequency_pass
        && worst_frequency_pass
        && jitter_pass
        && av_sync_pass
        && stop_latency_pass
        && hash_pass
        && eeg_lift_pass;

    HilReport {
        frequency_error_hz,
        frequency_pass,
        worst_frequency_error_hz,
        worst_frequency_pass,
        jitter_pass,
        av_sync_pass,
        stop_latency_pass,
        hash_reproducibility,
        hash_pass,
        eeg_lift,
        eeg_lift_pass,
        overall_pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing() -> HilMeasurement {
        HilMeasurement {
            commanded_frequency_hz: 40.0,
            measured_frequency_hz: 40.05,     // within ±0.1 Hz (mean)
            max_abs_freq_error_hz: 0.08,      // worst case also within ±0.1 Hz
            max_half_period_jitter_us: 120.0, // < 500 µs
            av_sync_drift_ms: 2.0,            // < 5 ms
            stop_latency_ms: 40.0,            // < 100 ms
            hashes_reproduced: 100,
            hashes_total: 100, // 100%
            eeg_entrainment_adaptive: 0.36,
            eeg_entrainment_fixed_40hz: 0.30, // +20%
        }
    }

    #[test]
    fn a_good_bench_run_passes_all_criteria() {
        let r = verify_hil(&passing(), &HilTargets::default());
        assert!(r.overall_pass);
        assert!(r.frequency_error_hz <= 0.1);
        assert!((r.eeg_lift - 0.20).abs() < 1e-9);
    }

    #[test]
    fn frequency_drift_beyond_tenth_hz_fails() {
        let mut m = passing();
        m.measured_frequency_hz = 40.3; // 0.3 Hz error
        let r = verify_hil(&m, &HilTargets::default());
        assert!(!r.frequency_pass);
        assert!(!r.overall_pass);
    }

    #[test]
    fn slow_stop_fails() {
        let mut m = passing();
        m.stop_latency_ms = 250.0;
        assert!(!verify_hil(&m, &HilTargets::default()).stop_latency_pass);
    }

    #[test]
    fn missing_stop_measurement_fails_closed() {
        let mut m = passing();
        m.stop_latency_ms = f64::NAN;
        assert!(!verify_hil(&m, &HilTargets::default()).stop_latency_pass);
    }

    #[test]
    fn any_hash_mismatch_fails_reproducibility() {
        let mut m = passing();
        m.hashes_reproduced = 99; // one of 100 drifted
        let r = verify_hil(&m, &HilTargets::default());
        assert!(!r.hash_pass);
        assert!(!r.overall_pass);
    }

    #[test]
    fn no_replay_fails_closed() {
        let mut m = passing();
        m.hashes_reproduced = 0;
        m.hashes_total = 0;
        assert!(!verify_hil(&m, &HilTargets::default()).hash_pass);
    }

    #[test]
    fn insufficient_eeg_lift_fails() {
        let mut m = passing();
        m.eeg_entrainment_adaptive = 0.32; // only +6.7%
        let r = verify_hil(&m, &HilTargets::default());
        assert!(!r.eeg_lift_pass);
        assert!(!r.overall_pass);
    }

    #[test]
    fn sync_drift_beyond_5ms_fails() {
        let mut m = passing();
        m.av_sync_drift_ms = 7.5;
        assert!(!verify_hil(&m, &HilTargets::default()).av_sync_pass);
    }

    #[test]
    fn worst_case_drift_beyond_budget_fails_even_when_mean_passes() {
        let mut m = passing();
        m.max_abs_freq_error_hz = 0.25; // mean is 0.05 Hz, worst case is not
        let r = verify_hil(&m, &HilTargets::default());
        assert!(r.frequency_pass);
        assert!(!r.worst_frequency_pass);
        assert!(!r.overall_pass);
    }

    #[test]
    fn half_period_jitter_beyond_budget_fails() {
        let mut m = passing();
        m.max_half_period_jitter_us = 900.0;
        let r = verify_hil(&m, &HilTargets::default());
        assert!(!r.jitter_pass);
        assert!(!r.overall_pass);
    }

    #[test]
    fn nan_measurements_fail_closed_per_criterion() {
        let t = HilTargets::default();

        let mut m = passing();
        m.measured_frequency_hz = f64::NAN;
        assert!(!verify_hil(&m, &t).frequency_pass);

        let mut m = passing();
        m.max_abs_freq_error_hz = f64::NAN;
        assert!(!verify_hil(&m, &t).worst_frequency_pass);

        let mut m = passing();
        m.max_half_period_jitter_us = f64::NAN;
        assert!(!verify_hil(&m, &t).jitter_pass);

        let mut m = passing();
        m.av_sync_drift_ms = f64::NAN;
        assert!(!verify_hil(&m, &t).av_sync_pass);

        let mut m = passing();
        m.eeg_entrainment_fixed_40hz = f64::NAN;
        assert!(!verify_hil(&m, &t).eeg_lift_pass);

        let mut m = passing();
        m.eeg_entrainment_adaptive = f64::NAN;
        assert!(!verify_hil(&m, &t).eeg_lift_pass);
    }

    #[test]
    fn negative_worst_case_values_fail_closed() {
        let t = HilTargets::default();
        let mut m = passing();
        m.max_abs_freq_error_hz = -0.01; // impossible absolute error
        assert!(!verify_hil(&m, &t).worst_frequency_pass);
        let mut m = passing();
        m.max_half_period_jitter_us = -5.0;
        assert!(!verify_hil(&m, &t).jitter_pass);
    }

    #[test]
    fn impossible_hash_counts_fail_closed() {
        let mut m = passing();
        m.hashes_reproduced = 101; // > hashes_total: corrupt bench data
        m.hashes_total = 100;
        let r = verify_hil(&m, &HilTargets::default());
        assert!(!r.hash_pass);
        assert_eq!(r.hash_reproducibility, 0.0); // never reported as >100%
        assert!(!r.overall_pass);
    }
}
