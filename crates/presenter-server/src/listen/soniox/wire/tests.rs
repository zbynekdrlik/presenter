use super::*;

fn context() -> ContextTerms {
    ContextTerms {
        general: vec![
            ("domain".to_string(), "Christian church sermon".to_string()),
            (
                "topic".to_string(),
                "preacher cites Bible verses".to_string(),
            ),
        ],
        terms: vec!["Genesis".to_string(), "Romans 8:37".to_string()],
    }
}

fn config(endpoint_delay_ms: Option<u32>) -> SonioxConfig {
    SonioxConfig {
        model: DEFAULT_MODEL.to_string(),
        context: context(),
        endpoint_delay_ms,
    }
}

#[test]
fn the_config_matches_the_measured_one() {
    let message = config_message(&config(None));
    assert_eq!(
        message,
        concat!(
            r#"{"model":"stt-rt-v6","audio_format":"pcm_s16le","sample_rate":16000,"#,
            r#""num_channels":1,"language_hints":["en"],"context":{"general":["#,
            r#"{"key":"domain","value":"Christian church sermon"},"#,
            r#"{"key":"topic","value":"preacher cites Bible verses"}],"#,
            r#""terms":["Genesis","Romans 8:37"]}}"#
        )
    );
    assert!(!message.contains("api_key"));
    assert!(!message.contains("enable_endpoint_detection"));
}

#[test]
fn the_config_context_size_is_what_core_counts() {
    let mut c = context();
    // Escapes (quote, backslash, a control char) and multi-byte letters.
    c.terms.push("Ján \"5:1\" \\ a\u{1}b".to_string());
    c.general.push(("note".to_string(), "Žalm\t23".to_string()));
    let message = config_message(&SonioxConfig {
        model: DEFAULT_MODEL.to_string(),
        context: c.clone(),
        endpoint_delay_ms: None,
    });
    assert!(message.contains(r#"Ján \"5:1\" \\ a\u0001b"#), "{message}");
    let value: serde_json::Value = serde_json::from_str(&message).unwrap_or_default();
    let context_json = serde_json::to_string(&value["context"]).unwrap_or_default();
    assert_eq!(
        context_json.chars().count(),
        presenter_core::listen::context::context_json_chars(&c.general, &c.terms)
    );
}

#[test]
fn endpoint_delay_adds_detection_clamped_500_to_3000() {
    let cases = [
        (1_200, 1_200),
        (0, 500),
        (100, 500),
        (500, 500),
        (3_000, 3_000),
        (9_999, 3_000),
    ];
    for (asked, sent) in cases {
        let message = config_message(&config(Some(asked)));
        let value: serde_json::Value = serde_json::from_str(&message).unwrap_or_default();
        assert_eq!(
            value["enable_endpoint_detection"],
            serde_json::json!(true),
            "{asked}"
        );
        assert_eq!(
            value["max_endpoint_delay_ms"],
            serde_json::json!(sent),
            "{asked}"
        );
        assert_eq!(value["model"], serde_json::json!("stt-rt-v6"));
    }
}

#[test]
fn control_messages_are_the_documented_json() {
    assert_eq!(control_message(Control::Finalize), r#"{"type":"finalize"}"#);
    assert_eq!(
        control_message(Control::Keepalive),
        r#"{"type":"keepalive"}"#
    );
}

#[test]
fn parses_partial_final_and_marker_tokens() {
    let text = concat!(
        r#"{"tokens":[{"text":"Good","start_ms":960,"end_ms":1020,"confidence":0.95,"is_final":true},"#,
        r#"{"text":" morn","start_ms":1020,"end_ms":1080,"confidence":0.9,"is_final":false,"language":"en"},"#,
        r#"{"text":"<fin>","is_final":true}],"#,
        r#""final_audio_proc_ms":1020,"total_audio_proc_ms":1200,"speaker_extra":"ignored"}"#
    );
    let response = parse_response(text);
    assert!(response.is_ok(), "{response:?}");
    let response = response.unwrap_or_else(|_| unreachable!());
    assert_eq!(response.final_audio_proc_ms, Some(1_020));
    assert_eq!(response.total_audio_proc_ms, Some(1_200));
    assert!(!response.finished);
    assert_eq!(response.error, None);
    let tokens = to_tokens(&response);
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].text, "Good");
    assert!(tokens[0].is_final);
    assert_eq!(tokens[0].start_ms, Some(960));
    assert_eq!(tokens[0].end_ms, Some(1_020));
    assert!((tokens[0].confidence - 0.95).abs() < 1e-6);
    assert_eq!(tokens[1].text, " morn");
    assert!(!tokens[1].is_final);
    assert_eq!(tokens[1].language.as_deref(), Some("en"));
    assert_eq!(tokens[2].text, "<fin>");
    assert!(tokens[2].is_final);
    assert_eq!(tokens[2].start_ms, None);
    assert!((tokens[2].confidence - 1.0).abs() < 1e-6);
    assert!(tokens.iter().all(|t| !t.is_translation));
}

#[test]
fn a_translation_token_is_marked_and_finished_is_read() {
    let text = concat!(
        r#"{"tokens":[{"text":"Dobré","is_final":true,"translation_status":"translation"},"#,
        r#"{"text":"Good","is_final":true,"translation_status":"original"}],"finished":true}"#
    );
    let response = parse_response(text).unwrap_or_else(|e| unreachable!("{e}"));
    assert!(response.finished);
    let tokens = to_tokens(&response);
    assert!(tokens[0].is_translation);
    assert!(!tokens[1].is_translation);
}

#[test]
fn an_empty_or_bare_response_parses() {
    let response = parse_response("{}").unwrap_or_else(|e| unreachable!("{e}"));
    assert!(response.tokens.is_empty());
    assert!(!response.finished);
    assert_eq!(response.error, None);
    assert!(parse_response("not json").is_err());
    assert!(parse_response(r#"{"tokens":[{"start_ms":1}]}"#).is_err());
}

#[test]
fn parses_an_error_response() {
    let text = concat!(
        r#"{"tokens":[],"error_code":503,"error_type":"service_unavailable","#,
        r#""error_message":"Service is temporarily unavailable.","more_info":"x","request_id":"r1"}"#
    );
    let response = parse_response(text).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(
        response.error,
        Some(SonioxError {
            code: Some(503),
            kind: Some("service_unavailable".to_string()),
            message: "Service is temporarily unavailable.".to_string(),
        })
    );
    assert_eq!(
        response.error.as_ref().map(classify),
        Some(ErrorClass::Server)
    );
}

fn error(code: Option<u16>, kind: Option<&str>) -> SonioxError {
    SonioxError {
        code,
        kind: kind.map(str::to_string),
        message: String::new(),
    }
}

#[test]
fn classify_reads_the_code_first_then_the_error_type() {
    let by_code = [
        (400, ErrorClass::BadRequest),
        (401, ErrorClass::Auth),
        (402, ErrorClass::Payment),
        (403, ErrorClass::Auth),
        (408, ErrorClass::Timeout),
        (413, ErrorClass::TooLong),
        (429, ErrorClass::RateLimited),
        (500, ErrorClass::Server),
        (503, ErrorClass::Server),
        (418, ErrorClass::Unknown),
    ];
    for (code, class) in by_code {
        assert_eq!(classify(&error(Some(code), None)), class, "{code}");
    }
    // The code wins over a contradicting type.
    assert_eq!(
        classify(&error(Some(402), Some("internal_error"))),
        ErrorClass::Payment
    );
    let by_kind = [
        ("unauthenticated", ErrorClass::Auth),
        ("temp_api_key_session_expired", ErrorClass::Auth),
        ("organization_balance_exhausted", ErrorClass::Payment),
        ("project_monthly_budget_exhausted", ErrorClass::Payment),
        ("model_not_available", ErrorClass::BadRequest),
        ("request_timeout", ErrorClass::Timeout),
        ("max_duration_reached", ErrorClass::TooLong),
        (
            "max_concurrent_connections_reached",
            ErrorClass::RateLimited,
        ),
        ("internal_error", ErrorClass::Server),
        ("something_new", ErrorClass::Unknown),
    ];
    for (kind, class) in by_kind {
        assert_eq!(classify(&error(None, Some(kind))), class, "{kind}");
        assert_eq!(classify(&error(Some(418), Some(kind))), class, "418 {kind}");
    }
    assert_eq!(classify(&error(None, None)), ErrorClass::Unknown);
}

#[test]
fn classify_close_reads_the_reason_then_the_code() {
    assert_eq!(classify_close(1008, "unauthenticated"), ErrorClass::Auth);
    assert_eq!(
        classify_close(1000, "max_duration_reached"),
        ErrorClass::TooLong
    );
    assert_eq!(classify_close(1001, ""), ErrorClass::Timeout);
    assert_eq!(classify_close(1002, ""), ErrorClass::BadRequest);
    assert_eq!(classify_close(1009, "too big"), ErrorClass::BadRequest);
    assert_eq!(classify_close(1011, "internal"), ErrorClass::Server);
    assert_eq!(classify_close(1013, ""), ErrorClass::Server);
    assert_eq!(classify_close(1000, ""), ErrorClass::Unknown);
    assert_eq!(classify_close(1006, ""), ErrorClass::Unknown);
}

#[test]
fn redact_replaces_every_occurrence() {
    let key = "test-key-not-real";
    let text = format!("Bearer {key} failed; key={key}.");
    let redacted = redact(&text, key);
    assert_eq!(redacted, "Bearer <redacted> failed; key=<redacted>.");
    assert!(!redacted.contains(key));
    assert_eq!(redact("nothing here", key), "nothing here");
    assert_eq!(
        redact("an empty key changes nothing", ""),
        "an empty key changes nothing"
    );
}
