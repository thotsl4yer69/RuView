//! Unit tests for `stimulus` — split out to keep stimulus.rs under 500 lines.
use super::*;

#[test]
fn prior_is_inside_conservative_envelope() {
    let env = SafetyEnvelope::conservative();
    assert!(env.contains(&StimulusParameters::prior()));
}

#[test]
fn frequency_outside_band_is_rejected() {
    let env = SafetyEnvelope::conservative();
    let mut s = StimulusParameters::prior();
    s.frequency_hz = 50.0;
    let err = env.validate(&s).unwrap_err();
    assert!(matches!(err[0], EnvelopeViolation::Frequency { .. }));
}

#[test]
fn brightness_above_cap_is_rejected() {
    let env = SafetyEnvelope::conservative();
    let mut s = StimulusParameters::prior();
    s.brightness_level = 0.9;
    assert!(!env.contains(&s));
}

#[test]
fn clamp_output_is_always_contained() {
    let env = SafetyEnvelope::conservative();
    let hostile = StimulusParameters {
        frequency_hz: 1000.0,
        modality: Modality::Visual,
        brightness_level: 5.0,
        volume_level: -2.0,
        duty_cycle: DutyCycle::Pulsed,
        phase_offset_ms: 999.0,
        duration_minutes: 1e6,
    };
    assert!(env.contains(&env.clamp(hostile)));
}

#[test]
fn clamp_neutralizes_nan() {
    let env = SafetyEnvelope::conservative();
    let mut s = StimulusParameters::prior();
    s.frequency_hz = f64::NAN;
    s.brightness_level = f64::INFINITY;
    let c = env.clamp(s);
    assert!(env.contains(&c));
    assert_eq!(c.frequency_hz, env.min_hz());
    assert_eq!(c.brightness_level, 0.0);
}

// ---- Finding 3: absolute bounds are a property of construction ----

#[test]
fn conservative_is_within_absolute_bounds() {
    let e = SafetyEnvelope::conservative();
    assert!(e.min_hz() >= absolute::MIN_HZ);
    assert!(e.max_hz() <= absolute::MAX_HZ);
    assert!(e.brightness_cap() <= absolute::BRIGHTNESS_CAP);
    assert!(e.volume_cap() <= absolute::VOLUME_CAP);
    assert!(e.max_duration_minutes() <= absolute::MAX_DURATION_MINUTES);
    // The whole conservative band clears the 15–25 Hz photosensitive zone.
    assert!(e.min_hz() > 25.0);
}

#[test]
fn try_new_rejects_photosensitive_band() {
    // An 18–22 Hz envelope is squarely inside the provocative band.
    let err = SafetyEnvelope::try_new(18.0, 22.0, 0.3, 0.3, 10.0, 0.0).unwrap_err();
    assert!(matches!(err, EnvelopeError::FrequencyFloor { .. }));
}

#[test]
fn try_new_rejects_uncapped_brightness_and_volume() {
    assert!(matches!(
        SafetyEnvelope::try_new(36.0, 44.0, 1.0, 0.3, 10.0, 0.0).unwrap_err(),
        EnvelopeError::BrightnessCap { .. }
    ));
    assert!(matches!(
        SafetyEnvelope::try_new(36.0, 44.0, 0.3, 1.0, 10.0, 0.0).unwrap_err(),
        EnvelopeError::VolumeCap { .. }
    ));
}

#[test]
fn try_new_rejects_million_minute_session() {
    assert!(matches!(
        SafetyEnvelope::try_new(36.0, 44.0, 0.3, 0.3, 1e6, 0.0).unwrap_err(),
        EnvelopeError::Duration { .. }
    ));
}

#[test]
fn try_new_rejects_ceiling_and_inverted_band() {
    assert!(matches!(
        SafetyEnvelope::try_new(36.0, 80.0, 0.3, 0.3, 10.0, 0.0).unwrap_err(),
        EnvelopeError::FrequencyCeiling { .. }
    ));
    assert!(matches!(
        SafetyEnvelope::try_new(44.0, 36.0, 0.3, 0.3, 10.0, 0.0).unwrap_err(),
        EnvelopeError::BandInverted { .. }
    ));
}

#[test]
fn deserialization_cannot_build_an_18_22hz_envelope() {
    // Hostile config places the band in the photosensitive zone — serde must
    // route through try_new and FAIL, not silently accept it.
    let hostile = r#"{
            "min_hz": 18.0, "max_hz": 22.0,
            "brightness_cap": 1.0, "volume_cap": 1.0,
            "max_duration_minutes": 1000000.0, "max_phase_offset_ms": 0.0
        }"#;
    let parsed: Result<SafetyEnvelope, _> = serde_json::from_str(hostile);
    assert!(
        parsed.is_err(),
        "deserialization must reject a photosensitive-band envelope"
    );
}

#[test]
fn valid_envelope_roundtrips_through_serde() {
    let e = SafetyEnvelope::conservative();
    let json = serde_json::to_string(&e).unwrap();
    let back: SafetyEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(e, back);
}

#[test]
fn with_builders_revalidate_against_absolute_bounds() {
    let e = SafetyEnvelope::conservative();
    // Legal override succeeds.
    assert!(e.with_caps(0.5, 0.5).is_ok());
    // Illegal override (above the brightness ceiling) is refused.
    assert!(e.with_caps(0.9, 0.3).is_err());
    // Re-banding into the photosensitive zone is refused.
    assert!(e.with_band(18.0, 22.0).is_err());
}

#[test]
fn calibration_grid_is_36_to_44() {
    let env = SafetyEnvelope::conservative();
    let grid = env.calibration_frequencies();
    assert_eq!(grid.first(), Some(&36.0));
    assert_eq!(grid.last(), Some(&44.0));
    assert_eq!(grid.len(), 9);
}
