use super::*;
use crate::listen::mics::fixed_dante_mics;
use serde_json::json;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

/// The switch on, the first Dante mic chosen, 240 min.
fn enabled() -> TranscriptionSetting {
    TranscriptionSetting {
        enabled: true,
        guest_mic: Some("dante-mic-1".into()),
        daily_cap_minutes: DEFAULT_DAILY_CAP_MINUTES,
    }
}

fn over(d: u32, mode: SessionMode, extra_minutes: u32) -> SessionOverride {
    SessionOverride {
        day: day(d),
        mode,
        extra_minutes,
    }
}

/// Everything allowed: published mic, key, under the cap, stage on Kázeň.
fn inputs<'a>(setting: &'a TranscriptionSetting, published: &'a [MicConfig]) -> StartInputs<'a> {
    StartInputs {
        setting,
        published,
        key: true,
        today: day(15),
        over: None,
        preach: true,
        connected_ms_today: 0,
    }
}

#[test]
fn absent_rows_mean_off() {
    let setting: TranscriptionSetting = serde_json::from_value(json!({})).unwrap();
    assert_eq!(setting, TranscriptionSetting::default());
    assert!(!setting.enabled, "absent = the paid switch off");
    assert_eq!(setting.guest_mic, None);
    assert_eq!(setting.daily_cap_minutes, 240);
    let session: SessionOverride =
        serde_json::from_value(json!({"day": "2026-11-15", "mode": "on"})).unwrap();
    assert_eq!(session, over(15, SessionMode::On, 0));
    let usage: UsageRow = serde_json::from_value(json!({"day": "2026-11-15"})).unwrap();
    assert!(usage.mics.is_empty());
    let status = TranscriptionStatus::default();
    assert!(!status.key_configured);
    assert_eq!(status.mode, SessionMode::Auto);
    assert_eq!(
        serde_json::to_value(enabled()).unwrap(),
        json!({"enabled": true, "guestMic": "dante-mic-1", "dailyCapMinutes": 240})
    );
    let mut row = UsageRow {
        day: day(15),
        mics: BTreeMap::new(),
    };
    row.mics.insert(
        "guest".into(),
        MicUsage {
            audio_ms: 1_000,
            connected_ms: 2_000,
        },
    );
    assert_eq!(
        serde_json::to_value(&row).unwrap(),
        json!({"day": "2026-11-15", "mics": {"guest": {"audioMs": 1000, "connectedMs": 2000}}})
    );
    assert_eq!(
        serde_json::to_value(over(15, SessionMode::Off, 60)).unwrap(),
        json!({"day": "2026-11-15", "mode": "off", "extraMinutes": 60})
    );
}

#[test]
fn the_decision_checks_switch_mic_key_cap_override_then_preach_in_that_order() {
    let published = fixed_dante_mics();
    let all_on = enabled();
    assert_eq!(
        start_decision(&inputs(&all_on, &published)),
        Ok("dante-mic-1".into())
    );

    // Each case breaks one condition AND every later one: the earliest wins.
    let off_today = over(15, SessionMode::Off, 0);
    let mut later_broken = inputs(&all_on, &[]);
    later_broken.key = false;
    later_broken.preach = false;
    later_broken.over = Some(&off_today);
    later_broken.connected_ms_today = u64::MAX;

    let switch_off = TranscriptionSetting {
        enabled: false,
        guest_mic: None,
        ..enabled()
    };
    let case = StartInputs {
        setting: &switch_off,
        ..later_broken
    };
    assert_eq!(start_decision(&case), Err(OffReason::Disabled));

    let no_mic = TranscriptionSetting {
        guest_mic: None,
        ..enabled()
    };
    let case = StartInputs {
        setting: &no_mic,
        ..later_broken
    };
    assert_eq!(start_decision(&case), Err(OffReason::NoGuestMic));

    assert_eq!(
        start_decision(&later_broken),
        Err(OffReason::GuestMicMissing),
        "the chosen mic is not published"
    );

    let case = StartInputs {
        published: &published,
        ..later_broken
    };
    assert_eq!(start_decision(&case), Err(OffReason::NoKey));

    let case = StartInputs { key: true, ..case };
    assert_eq!(start_decision(&case), Err(OffReason::CapReached));

    let case = StartInputs {
        connected_ms_today: 0,
        ..case
    };
    assert_eq!(start_decision(&case), Err(OffReason::OverrideOff));

    let case = StartInputs { over: None, ..case };
    assert_eq!(start_decision(&case), Err(OffReason::NotPreach));
}

#[test]
fn the_cap_is_reached_at_exactly_the_cap() {
    let published = fixed_dante_mics();
    let setting = enabled();
    let mut i = inputs(&setting, &published);
    i.connected_ms_today = 240 * 60_000 - 1;
    assert!(start_decision(&i).is_ok());
    i.connected_ms_today = 240 * 60_000;
    assert_eq!(start_decision(&i), Err(OffReason::CapReached));
}

#[test]
fn an_override_from_yesterday_is_ignored() {
    let published = fixed_dante_mics();
    let setting = enabled();
    let yesterday_off = over(14, SessionMode::Off, 60);
    let mut i = inputs(&setting, &published);
    i.over = Some(&yesterday_off);
    assert_eq!(
        start_decision(&i),
        Ok("dante-mic-1".into()),
        "yesterday's stop does not stop today"
    );
    assert_eq!(today_mode(Some(&yesterday_off), day(15)), SessionMode::Auto);
    assert_eq!(
        cap_ms(&setting, Some(&yesterday_off), day(15)),
        240 * 60_000,
        "yesterday's extra hour is not today's"
    );
    i.preach = false;
    assert_eq!(start_decision(&i), Err(OffReason::NotPreach));
}

#[test]
fn on_override_starts_without_preach() {
    let published = fixed_dante_mics();
    let setting = enabled();
    let on_today = over(15, SessionMode::On, 0);
    let mut i = inputs(&setting, &published);
    i.preach = false;
    i.over = Some(&on_today);
    assert_eq!(start_decision(&i), Ok("dante-mic-1".into()));
    let auto_today = over(15, SessionMode::Auto, 0);
    i.over = Some(&auto_today);
    assert_eq!(start_decision(&i), Err(OffReason::NotPreach));
}

#[test]
fn preach_keeps_running_sixty_seconds_after_leaving() {
    let left = Instant::now();
    assert!(preach_active("preach", None, left));
    assert!(!preach_active("worship", None, left));
    assert!(preach_active("worship", Some(left), left));
    assert!(preach_active(
        "worship",
        Some(left),
        left + Duration::from_millis(59_999)
    ));
    assert!(!preach_active(
        "worship",
        Some(left),
        left + Duration::from_secs(60)
    ));
    assert!(
        preach_active("preach", Some(left), left + Duration::from_secs(3_600)),
        "back on Kázeň runs whatever the old leave time"
    );
}

#[test]
fn a_start_after_the_cap_adds_an_hour() {
    let setting = enabled();
    let first = next_override(None, day(15), SessionMode::On, true);
    assert_eq!(first, over(15, SessionMode::On, 60));
    assert_eq!(cap_ms(&setting, Some(&first), day(15)), 300 * 60_000);
    let second = next_override(Some(&first), day(15), SessionMode::On, true);
    assert_eq!(
        second.extra_minutes, 120,
        "a second start adds another hour"
    );
    let stop = next_override(Some(&second), day(15), SessionMode::Off, true);
    assert_eq!(
        stop,
        over(15, SessionMode::Off, 120),
        "a stop keeps today's hours"
    );
    let auto = next_override(Some(&stop), day(15), SessionMode::Auto, false);
    assert_eq!(auto, over(15, SessionMode::Auto, 120));
    let start_under_cap = next_override(Some(&auto), day(15), SessionMode::On, false);
    assert_eq!(
        start_under_cap.extra_minutes, 120,
        "under the cap a start adds nothing"
    );
    let next_day = next_override(Some(&second), day(16), SessionMode::On, false);
    assert_eq!(
        next_day,
        over(16, SessionMode::On, 0),
        "a new day starts at 0"
    );
}

#[test]
fn cost_is_twelve_cents_per_connected_hour() {
    assert!((cost_usd(3_600_000) - 0.12).abs() < 1e-12);
    assert!((cost_usd(30 * 60_000) - 0.06).abs() < 1e-12);
    assert!((cost_usd(240 * 60_000) - 0.48).abs() < 1e-12);
    assert_eq!(cost_usd(0), 0.0);
}

#[test]
fn validation_refuses_an_unknown_mic_and_a_cap_outside_ten_to_six_hundred() {
    let published = fixed_dante_mics();
    assert_eq!(validate_transcription(&enabled(), &published), Ok(()));
    let none = TranscriptionSetting {
        guest_mic: None,
        ..enabled()
    };
    assert_eq!(
        validate_transcription(&none, &[]),
        Ok(()),
        "no guest mic is allowed"
    );
    let unknown = TranscriptionSetting {
        guest_mic: Some("guest".into()),
        ..enabled()
    };
    let err = validate_transcription(&unknown, &published).unwrap_err();
    assert_eq!(
        err,
        TranscriptionValidationError::UnknownMic("guest".into())
    );
    assert_eq!(
        err.to_string(),
        "Mikrofón hosťa „guest“ nie je v zozname mikrofónov."
    );
    for (cap, ok) in [
        (9, false),
        (10, true),
        (600, true),
        (601, false),
        (0, false),
    ] {
        let setting = TranscriptionSetting {
            daily_cap_minutes: cap,
            ..enabled()
        };
        let result = validate_transcription(&setting, &published);
        assert_eq!(result.is_ok(), ok, "cap {cap}");
        if !ok {
            assert_eq!(
                result,
                Err(TranscriptionValidationError::CapOutOfRange(cap))
            );
        }
    }
}

#[test]
fn a_stored_guest_mic_that_went_missing_still_lets_the_row_change() {
    let stored = enabled(); // dante-mic-1, now not published (no Dante interface)
    let switch_off = TranscriptionSetting {
        enabled: false,
        daily_cap_minutes: 120,
        ..enabled()
    };
    assert_eq!(
        validate_transcription_change(&switch_off, Some(&stored), &[]),
        Ok(()),
        "the switch can go off and the cap change while the mic is missing"
    );
    assert_eq!(
        validate_transcription(&switch_off, &[]),
        Err(TranscriptionValidationError::UnknownMic(
            "dante-mic-1".into()
        )),
        "a new row must name a published mic"
    );
    let other = TranscriptionSetting {
        guest_mic: Some("dante-mic-2".into()),
        ..enabled()
    };
    assert_eq!(
        validate_transcription_change(&other, Some(&stored), &[]),
        Err(TranscriptionValidationError::UnknownMic(
            "dante-mic-2".into()
        )),
        "a newly chosen mic must be published"
    );
    let bad_cap = TranscriptionSetting {
        daily_cap_minutes: 5,
        ..enabled()
    };
    assert_eq!(
        validate_transcription_change(&bad_cap, Some(&stored), &[]),
        Err(TranscriptionValidationError::CapOutOfRange(5))
    );
    let session: SessionOverride = serde_json::from_value(json!({"day": "2026-11-15"})).unwrap();
    assert_eq!(
        session.mode,
        SessionMode::Auto,
        "a row without a mode is auto"
    );
}

#[test]
fn every_off_reason_has_its_own_slovak_line() {
    let all = [
        OffReason::Disabled,
        OffReason::NoGuestMic,
        OffReason::GuestMicMissing,
        OffReason::NoKey,
        OffReason::CapReached,
        OffReason::NotPreach,
        OffReason::OverrideOff,
        OffReason::Auth,
        OffReason::Payment,
        OffReason::Refused,
        OffReason::TooManyConnections,
    ];
    let lines: std::collections::HashSet<_> = all.iter().map(OffReason::slovak).collect();
    assert_eq!(lines.len(), all.len());
    assert_eq!(
        OffReason::NoKey.slovak(),
        "Chýba kľúč služby prepisu (Soniox)"
    );
    assert_eq!(
        OffReason::Payment.slovak(),
        "Prepis vypnutý — problém s účtom služby"
    );
}
