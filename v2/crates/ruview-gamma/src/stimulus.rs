//! Stimulus parameters and the safety envelope (ADR-250 §5, §12).
//!
//! The [`SafetyEnvelope`] is the load-bearing safety primitive of the whole
//! crate: **no recommendation, calibration step, or closed-loop nudge may ever
//! produce a [`StimulusParameters`] that fails [`SafetyEnvelope::validate`].**
//! Every code path that emits a stimulus setting routes through
//! [`SafetyEnvelope::clamp`] (best-effort coercion) and is asserted against
//! [`SafetyEnvelope::contains`] in tests.

use serde::{Deserialize, Serialize};

use crate::math::clamp_safe;

/// Stimulation modality (ADR-250 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Sound only.
    Audio,
    /// Light only.
    Visual,
    /// Combined audio-visual — GENUS-style, the preferred protocol.
    AudioVisual,
}

impl Modality {
    /// Canonical lowercase tag used in the session witness.
    pub fn tag(self) -> &'static str {
        match self {
            Modality::Audio => "audio",
            Modality::Visual => "visual",
            Modality::AudioVisual => "audio_visual",
        }
    }
}

/// Duty-cycle shape (ADR-250 §5). Conservative ordering: `Continuous` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DutyCycle {
    /// Steady stimulation for the whole session.
    Continuous,
    /// Amplitude ramps up/down — gentlest onset.
    Ramped,
    /// On/off pulsing — explored only after tolerance is established.
    Pulsed,
}

impl DutyCycle {
    pub fn tag(self) -> &'static str {
        match self {
            DutyCycle::Continuous => "continuous",
            DutyCycle::Ramped => "ramped",
            DutyCycle::Pulsed => "pulsed",
        }
    }
}

/// A concrete stimulation setting. All intensity fields are normalized `[0,1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StimulusParameters {
    /// Carrier/flicker frequency in Hz (search band 36–44, prior 40).
    pub frequency_hz: f64,
    /// Stimulation modality.
    pub modality: Modality,
    /// Visual brightness in `[0,1]` (capped well below unsafe flicker intensity).
    pub brightness_level: f64,
    /// Audio volume in `[0,1]` (comfort-bounded).
    pub volume_level: f64,
    /// Duty-cycle shape.
    pub duty_cycle: DutyCycle,
    /// Inter-modality phase offset in milliseconds.
    pub phase_offset_ms: f64,
    /// Session duration in minutes.
    pub duration_minutes: f64,
}

impl StimulusParameters {
    /// The evidence-based starting prior: 40 Hz combined audio-visual, gentle
    /// intensities, continuous, short (ADR-250 §5 "Starting prior").
    pub fn prior() -> Self {
        Self {
            frequency_hz: 40.0,
            modality: Modality::AudioVisual,
            brightness_level: 0.30,
            volume_level: 0.28,
            duty_cycle: DutyCycle::Continuous,
            phase_offset_ms: 0.0,
            duration_minutes: 10.0,
        }
    }
}

/// Reasons a [`StimulusParameters`] is rejected by the envelope. Each variant
/// is logged verbatim into the RuFlo safety trail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum EnvelopeViolation {
    /// Frequency outside `[min_hz, max_hz]`.
    Frequency { value: f64, min: f64, max: f64 },
    /// Brightness above the hard cap.
    Brightness { value: f64, cap: f64 },
    /// Volume above the hard cap.
    Volume { value: f64, cap: f64 },
    /// Duration above the per-stage maximum.
    Duration { value: f64, max: f64 },
    /// A non-finite (NaN/Inf) field was supplied.
    NonFinite { field: &'static str },
}

/// Compiled-in **absolute** bounds — the floor under every envelope (Finding 3,
/// 2026-06-11 safety review). No `SafetyEnvelope` can be constructed (by code,
/// builder, or deserialization) that violates these. They make safety a
/// property of *construction*, not of default values, mirroring the firmware's
/// compiled-in stance.
pub mod absolute {
    /// Hard frequency floor (Hz). Chosen **>= 30 Hz** so the entire envelope —
    /// including its lowest edge — sits above the photosensitive/provocative
    /// 15–25 Hz flicker band with margin. A config can never push stimulation
    /// into that band.
    pub const MIN_HZ: f64 = 30.0;
    /// Hard frequency ceiling (Hz). Above the 36–44 Hz search band with room
    /// for future programs, bounded so the actuator is never driven absurdly fast.
    pub const MAX_HZ: f64 = 60.0;
    /// Hard brightness cap — no envelope may uncap flicker brightness toward 1.0.
    pub const BRIGHTNESS_CAP: f64 = 0.6;
    /// Hard volume cap — comfort ceiling.
    pub const VOLUME_CAP: f64 = 0.6;
    /// Hard maximum single-session duration (minutes) — no `1e6`-minute sessions.
    pub const MAX_DURATION_MINUTES: f64 = 30.0;
    /// Hard maximum absolute inter-modality phase offset (ms).
    pub const MAX_PHASE_OFFSET_MS: f64 = 20.0;
}

/// Why an attempted [`SafetyEnvelope`] construction was rejected. Every variant
/// is a *safe* refusal: a rejected envelope is never built, so deserialization
/// of a hostile config fails closed instead of silently widening the bounds.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum EnvelopeError {
    /// A field was NaN/Inf.
    #[error("envelope field `{field}` is not finite")]
    NonFinite { field: &'static str },
    /// `min_hz` below the compiled-in photosensitive-safe floor.
    #[error("min_hz {min_hz} is below the absolute floor {floor} Hz (photosensitive-band guard)")]
    FrequencyFloor { min_hz: f64, floor: f64 },
    /// `max_hz` above the compiled-in ceiling.
    #[error("max_hz {max_hz} exceeds the absolute ceiling {ceiling} Hz")]
    FrequencyCeiling { max_hz: f64, ceiling: f64 },
    /// Band is empty or inverted.
    #[error("frequency band is empty/inverted: min_hz {min_hz} >= max_hz {max_hz}")]
    BandInverted { min_hz: f64, max_hz: f64 },
    /// Brightness cap out of `[0, ABS]`.
    #[error("brightness_cap {cap} outside [0, {abs}]")]
    BrightnessCap { cap: f64, abs: f64 },
    /// Volume cap out of `[0, ABS]`.
    #[error("volume_cap {cap} outside [0, {abs}]")]
    VolumeCap { cap: f64, abs: f64 },
    /// Duration out of `(0, ABS]`.
    #[error("max_duration_minutes {minutes} outside (0, {abs}]")]
    Duration { minutes: f64, abs: f64 },
    /// Phase offset cap out of `[0, ABS]`.
    #[error("max_phase_offset_ms {ms} outside [0, {abs}]")]
    PhaseOffset { ms: f64, abs: f64 },
}

/// The predefined safety envelope. Optimization happens **only inside** these
/// bounds; the system "must never autonomously expand beyond the allowed safety
/// envelope" (ADR-250 §12). The envelope itself is data, never widened by the
/// optimizer — only an operator constructs a wider one deliberately.
///
/// **Safety by construction (Finding 3, 2026-06-11 review):** the bound fields
/// are private and reachable only through [`SafetyEnvelope::try_new`] (and the
/// `with_*` builders), which reject anything outside the compiled-in
/// [`absolute`] bounds. Deserialization routes through the same validator via
/// `#[serde(try_from)]`, so no config — however hostile — can place the band in
/// the 15–25 Hz photosensitive zone, uncap brightness/volume toward 1.0, or set
/// a million-minute session.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(into = "SafetyEnvelopeWire", try_from = "SafetyEnvelopeWire")]
pub struct SafetyEnvelope {
    min_hz: f64,
    max_hz: f64,
    brightness_cap: f64,
    volume_cap: f64,
    max_duration_minutes: f64,
    max_phase_offset_ms: f64,
}

/// Transparent serde mirror of [`SafetyEnvelope`]. Deserialization always passes
/// through [`SafetyEnvelope::try_new`] (`try_from`), so the absolute bounds
/// cannot be bypassed; serialization is a plain projection (`into`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SafetyEnvelopeWire {
    min_hz: f64,
    max_hz: f64,
    brightness_cap: f64,
    volume_cap: f64,
    max_duration_minutes: f64,
    max_phase_offset_ms: f64,
}

impl From<SafetyEnvelope> for SafetyEnvelopeWire {
    fn from(e: SafetyEnvelope) -> Self {
        Self {
            min_hz: e.min_hz,
            max_hz: e.max_hz,
            brightness_cap: e.brightness_cap,
            volume_cap: e.volume_cap,
            max_duration_minutes: e.max_duration_minutes,
            max_phase_offset_ms: e.max_phase_offset_ms,
        }
    }
}

impl TryFrom<SafetyEnvelopeWire> for SafetyEnvelope {
    type Error = EnvelopeError;
    fn try_from(w: SafetyEnvelopeWire) -> Result<Self, Self::Error> {
        Self::try_new(
            w.min_hz,
            w.max_hz,
            w.brightness_cap,
            w.volume_cap,
            w.max_duration_minutes,
            w.max_phase_offset_ms,
        )
    }
}

impl SafetyEnvelope {
    /// Construct an envelope, enforcing the compiled-in [`absolute`] bounds. The
    /// **only** constructor; `conservative()` and the `with_*` builders all route
    /// through it, and so does deserialization. Returns [`EnvelopeError`] (a safe
    /// refusal) for any out-of-bounds request.
    pub fn try_new(
        min_hz: f64,
        max_hz: f64,
        brightness_cap: f64,
        volume_cap: f64,
        max_duration_minutes: f64,
        max_phase_offset_ms: f64,
    ) -> Result<Self, EnvelopeError> {
        for (field, v) in [
            ("min_hz", min_hz),
            ("max_hz", max_hz),
            ("brightness_cap", brightness_cap),
            ("volume_cap", volume_cap),
            ("max_duration_minutes", max_duration_minutes),
            ("max_phase_offset_ms", max_phase_offset_ms),
        ] {
            if !v.is_finite() {
                return Err(EnvelopeError::NonFinite { field });
            }
        }
        if min_hz < absolute::MIN_HZ {
            return Err(EnvelopeError::FrequencyFloor {
                min_hz,
                floor: absolute::MIN_HZ,
            });
        }
        if max_hz > absolute::MAX_HZ {
            return Err(EnvelopeError::FrequencyCeiling {
                max_hz,
                ceiling: absolute::MAX_HZ,
            });
        }
        if min_hz >= max_hz {
            return Err(EnvelopeError::BandInverted { min_hz, max_hz });
        }
        if !(0.0..=absolute::BRIGHTNESS_CAP).contains(&brightness_cap) {
            return Err(EnvelopeError::BrightnessCap {
                cap: brightness_cap,
                abs: absolute::BRIGHTNESS_CAP,
            });
        }
        if !(0.0..=absolute::VOLUME_CAP).contains(&volume_cap) {
            return Err(EnvelopeError::VolumeCap {
                cap: volume_cap,
                abs: absolute::VOLUME_CAP,
            });
        }
        if max_duration_minutes <= 0.0 || max_duration_minutes > absolute::MAX_DURATION_MINUTES {
            return Err(EnvelopeError::Duration {
                minutes: max_duration_minutes,
                abs: absolute::MAX_DURATION_MINUTES,
            });
        }
        if !(0.0..=absolute::MAX_PHASE_OFFSET_MS).contains(&max_phase_offset_ms) {
            return Err(EnvelopeError::PhaseOffset {
                ms: max_phase_offset_ms,
                abs: absolute::MAX_PHASE_OFFSET_MS,
            });
        }
        Ok(Self {
            min_hz,
            max_hz,
            brightness_cap,
            volume_cap,
            max_duration_minutes,
            max_phase_offset_ms,
        })
    }

    /// Lower band edge (Hz). Always `>= absolute::MIN_HZ`.
    pub fn min_hz(&self) -> f64 {
        self.min_hz
    }
    /// Upper band edge (Hz). Always `<= absolute::MAX_HZ`.
    pub fn max_hz(&self) -> f64 {
        self.max_hz
    }
    /// Hard brightness cap. Always `<= absolute::BRIGHTNESS_CAP`.
    pub fn brightness_cap(&self) -> f64 {
        self.brightness_cap
    }
    /// Hard volume cap. Always `<= absolute::VOLUME_CAP`.
    pub fn volume_cap(&self) -> f64 {
        self.volume_cap
    }
    /// Maximum session duration (minutes). Always `<= absolute::MAX_DURATION_MINUTES`.
    pub fn max_duration_minutes(&self) -> f64 {
        self.max_duration_minutes
    }
    /// Maximum absolute inter-modality phase offset (ms).
    pub fn max_phase_offset_ms(&self) -> f64 {
        self.max_phase_offset_ms
    }

    /// Re-band an existing envelope (re-validated against the absolute bounds).
    pub fn with_band(self, min_hz: f64, max_hz: f64) -> Result<Self, EnvelopeError> {
        Self::try_new(
            min_hz,
            max_hz,
            self.brightness_cap,
            self.volume_cap,
            self.max_duration_minutes,
            self.max_phase_offset_ms,
        )
    }
    /// Override the intensity caps (re-validated against the absolute bounds).
    pub fn with_caps(self, brightness_cap: f64, volume_cap: f64) -> Result<Self, EnvelopeError> {
        Self::try_new(
            self.min_hz,
            self.max_hz,
            brightness_cap,
            volume_cap,
            self.max_duration_minutes,
            self.max_phase_offset_ms,
        )
    }
    /// Override the maximum session duration (re-validated against the absolute bounds).
    pub fn with_max_duration_minutes(self, minutes: f64) -> Result<Self, EnvelopeError> {
        Self::try_new(
            self.min_hz,
            self.max_hz,
            self.brightness_cap,
            self.volume_cap,
            minutes,
            self.max_phase_offset_ms,
        )
    }

    /// Conservative research-grade default envelope (ADR-250 §5 default ranges,
    /// "safety_profile: conservative"). Well inside every [`absolute`] bound.
    pub fn conservative() -> Self {
        Self::try_new(36.0, 44.0, 0.40, 0.40, 15.0, 5.0)
            .expect("conservative envelope is within the compiled-in absolute bounds")
    }

    /// `true` iff every field of `s` lies inside the envelope and is finite.
    pub fn contains(&self, s: &StimulusParameters) -> bool {
        self.validate(s).is_ok()
    }

    /// Validate a setting, returning every violation found (not just the first)
    /// so the safety log is complete.
    pub fn validate(&self, s: &StimulusParameters) -> Result<(), Vec<EnvelopeViolation>> {
        let mut v = Vec::new();
        for (field, val) in [
            ("frequency_hz", s.frequency_hz),
            ("brightness_level", s.brightness_level),
            ("volume_level", s.volume_level),
            ("duration_minutes", s.duration_minutes),
            ("phase_offset_ms", s.phase_offset_ms),
        ] {
            if !val.is_finite() {
                v.push(EnvelopeViolation::NonFinite { field });
            }
        }
        if !v.is_empty() {
            return Err(v);
        }
        if s.frequency_hz < self.min_hz || s.frequency_hz > self.max_hz {
            v.push(EnvelopeViolation::Frequency {
                value: s.frequency_hz,
                min: self.min_hz,
                max: self.max_hz,
            });
        }
        if s.brightness_level > self.brightness_cap || s.brightness_level < 0.0 {
            v.push(EnvelopeViolation::Brightness {
                value: s.brightness_level,
                cap: self.brightness_cap,
            });
        }
        if s.volume_level > self.volume_cap || s.volume_level < 0.0 {
            v.push(EnvelopeViolation::Volume {
                value: s.volume_level,
                cap: self.volume_cap,
            });
        }
        if s.duration_minutes > self.max_duration_minutes || s.duration_minutes <= 0.0 {
            v.push(EnvelopeViolation::Duration {
                value: s.duration_minutes,
                max: self.max_duration_minutes,
            });
        }
        if v.is_empty() {
            Ok(())
        } else {
            Err(v)
        }
    }

    /// Best-effort coercion of `s` into the envelope. Used as a defensive final
    /// stage on any emitted recommendation; coercion can only ever *reduce*
    /// intensity / pull frequency inward, never expand it. The result always
    /// satisfies [`contains`](Self::contains).
    pub fn clamp(&self, mut s: StimulusParameters) -> StimulusParameters {
        s.frequency_hz = clamp_safe(s.frequency_hz, self.min_hz, self.max_hz);
        s.brightness_level = clamp_safe(s.brightness_level, 0.0, self.brightness_cap);
        s.volume_level = clamp_safe(s.volume_level, 0.0, self.volume_cap);
        s.phase_offset_ms = clamp_safe(
            s.phase_offset_ms,
            -self.max_phase_offset_ms,
            self.max_phase_offset_ms,
        );
        // Duration must be strictly positive; floor at 1 minute.
        s.duration_minutes = clamp_safe(s.duration_minutes, 1.0, self.max_duration_minutes);
        s
    }

    /// The calibration sweep grid (ADR-250 §8 Phase 1): integer-Hz steps across
    /// the band, intersected with the envelope. Returns frequencies in Hz.
    pub fn calibration_frequencies(&self) -> Vec<f64> {
        let lo = self.min_hz.ceil() as i32;
        let hi = self.max_hz.floor() as i32;
        (lo..=hi).map(|f| f as f64).collect()
    }
}

#[cfg(test)]
#[path = "stimulus_tests.rs"]
mod tests;
