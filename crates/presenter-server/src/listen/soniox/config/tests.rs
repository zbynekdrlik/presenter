use super::*;
use std::collections::HashMap;

fn env(vars: &[(&str, &str)]) -> SonioxEnv {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    SonioxEnv::from_vars(|name| map.get(name).cloned())
}

#[test]
fn env_defaults_and_overrides() {
    let defaults = env(&[]);
    assert_eq!(defaults.url, "wss://stt-rt.soniox.com/transcribe-websocket");
    assert_eq!(defaults.model, "stt-rt-v6");
    assert_eq!(defaults.key, None);
    assert_eq!(defaults.endpoint_delay_ms, None);

    let set = env(&[
        (URL_VAR, "ws://127.0.0.1:9/mock"),
        (KEY_VAR, " test-key-not-real\n"),
        (MODEL_VAR, "stt-rt-v7"),
        (ENDPOINT_DELAY_VAR, "1500"),
    ]);
    assert_eq!(set.url, "ws://127.0.0.1:9/mock");
    assert_eq!(set.key.as_deref(), Some("test-key-not-real"));
    assert_eq!(set.model, "stt-rt-v7");
    assert_eq!(set.endpoint_delay_ms, Some(1_500));
}

#[test]
fn empty_values_count_as_unset_and_a_bad_delay_is_ignored() {
    let blank = env(&[
        (URL_VAR, ""),
        (KEY_VAR, "   "),
        (MODEL_VAR, ""),
        (ENDPOINT_DELAY_VAR, "fast"),
    ]);
    assert_eq!(blank.url, "wss://stt-rt.soniox.com/transcribe-websocket");
    assert_eq!(blank.key, None);
    assert_eq!(blank.model, "stt-rt-v6");
    assert_eq!(blank.endpoint_delay_ms, None);
    assert_eq!(env(&[(ENDPOINT_DELAY_VAR, "-5")]).endpoint_delay_ms, None);
    // 0 is a number: it turns detection on (config_message clamps it to 500).
    assert_eq!(env(&[(ENDPOINT_DELAY_VAR, "0")]).endpoint_delay_ms, Some(0));
}

#[test]
fn debug_never_prints_the_key() {
    let set = env(&[(KEY_VAR, "test-key-not-real")]);
    let printed = format!("{set:?}");
    assert!(!printed.contains("test-key-not-real"), "{printed}");
    assert!(printed.contains("key: \"<set>\""), "{printed}");
    let unset = format!("{:?}", env(&[]));
    assert!(unset.contains("key: \"<unset>\""), "{unset}");
    assert!(unset.contains("stt-rt-v6"), "{unset}");
}
