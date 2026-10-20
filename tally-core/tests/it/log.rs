use nonempty::NonEmpty;
use tally_core::{LogLine, LogLineError, RefuseReason, UpstreamReason, parse_log_line};

const RUN: &str = "8c1f0a3e9b7d4c62a5e01f9d3b7c4e18";
const TIME: &str = "2026-09-26T10:15:00Z";

/// Checks that "line" is written as "text", and that "text" reads back as "line".
fn written_as(line: LogLine, text: &str) {
    assert_eq!(
        serde_json::to_string(&line).expect("a line serializes"),
        text
    );
    assert_eq!(parse_log_line(text).expect("the line parses"), line);
}

/// The message of the serde error "text" gets.
fn json_error(text: &str) -> String {
    match parse_log_line(text) {
        Err(LogLineError::Json(error)) => error.to_string(),
        other => panic!("expected a serde error, got {other:?}"),
    }
}

// Each event, as written

#[test]
fn a_start_line() {
    written_as(
        LogLine::Start {
            seq: 0,
            run: RUN.to_string(),
            client: "weather-app".to_string(),
            settings_sha256: "9f2c".to_string(),
            time: TIME.to_string(),
        },
        r#"{"event":"start","seq":0,"run":"8c1f0a3e9b7d4c62a5e01f9d3b7c4e18","client":"weather-app","settings_sha256":"9f2c","time":"2026-09-26T10:15:00Z"}"#,
    );
}

#[test]
fn a_release_line() {
    written_as(
        LogLine::Release {
            seq: 1,
            client: "weather-app".to_string(),
            keys: NonEmpty::new("weather_key".to_string()),
            host: "api.weather.local:80".to_string(),
            time: TIME.to_string(),
        },
        r#"{"event":"release","seq":1,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","time":"2026-09-26T10:15:00Z"}"#,
    );
}

#[test]
fn a_release_line_with_two_keys() {
    written_as(
        LogLine::Release {
            seq: 1,
            client: "weather-app".to_string(),
            keys: NonEmpty::from_vec(vec!["maps_key".to_string(), "weather_key".to_string()])
                .expect("two keys"),
            host: "api.weather.local:80".to_string(),
            time: TIME.to_string(),
        },
        r#"{"event":"release","seq":1,"client":"weather-app","keys":["maps_key","weather_key"],"host":"api.weather.local:80","time":"2026-09-26T10:15:00Z"}"#,
    );
}

#[test]
fn a_pass_line() {
    written_as(
        LogLine::Pass {
            seq: 2,
            client: "weather-app".to_string(),
            keys: Vec::new(),
            host: "api.weather.local:80".to_string(),
            time: TIME.to_string(),
        },
        r#"{"event":"pass","seq":2,"client":"weather-app","keys":[],"host":"api.weather.local:80","time":"2026-09-26T10:15:00Z"}"#,
    );
}

#[test]
fn a_refuse_line() {
    written_as(
        LogLine::Refuse {
            seq: 3,
            client: "weather-app".to_string(),
            keys: vec!["weather_key".to_string()],
            host: Some("evil.local:80".to_string()),
            reason: RefuseReason::HostNotAllowed,
            time: TIME.to_string(),
        },
        r#"{"event":"refuse","seq":3,"client":"weather-app","keys":["weather_key"],"host":"evil.local:80","reason":"host_not_allowed","time":"2026-09-26T10:15:00Z"}"#,
    );
}

#[test]
fn a_refuse_line_with_a_null_host() {
    written_as(
        LogLine::Refuse {
            seq: 4,
            client: "weather-app".to_string(),
            keys: Vec::new(),
            host: None,
            reason: RefuseReason::MalformedRequest,
            time: TIME.to_string(),
        },
        r#"{"event":"refuse","seq":4,"client":"weather-app","keys":[],"host":null,"reason":"malformed_request","time":"2026-09-26T10:15:00Z"}"#,
    );
}

#[test]
fn an_upstream_error_line() {
    written_as(
        LogLine::UpstreamError {
            seq: 5,
            client: "weather-app".to_string(),
            keys: vec!["weather_key".to_string()],
            host: "api.weather.local:80".to_string(),
            about: 1,
            reason: UpstreamReason::ResponseTimeout,
            time: TIME.to_string(),
        },
        r#"{"event":"upstream_error","seq":5,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","about":1,"reason":"response_timeout","time":"2026-09-26T10:15:00Z"}"#,
    );
}

#[test]
fn a_stop_line() {
    written_as(
        LogLine::Stop {
            seq: 6,
            client: "weather-app".to_string(),
            time: TIME.to_string(),
        },
        r#"{"event":"stop","seq":6,"client":"weather-app","time":"2026-09-26T10:15:00Z"}"#,
    );
}

// Reasons, as named in DESIGN's table

#[test]
fn refuse_reasons_are_written_as_in_design() {
    let reasons = [
        (RefuseReason::IncompleteRequest, "incomplete_request"),
        (RefuseReason::HeadTooLarge, "head_too_large"),
        (RefuseReason::BodyTooLarge, "body_too_large"),
        (RefuseReason::MalformedRequest, "malformed_request"),
        (RefuseReason::HostMismatch, "host_mismatch"),
        (RefuseReason::UnsupportedRequest, "unsupported_request"),
        (RefuseReason::HostNotAllowed, "host_not_allowed"),
    ];
    for (reason, name) in reasons {
        assert_eq!(
            serde_json::to_string(&reason).expect("a reason serializes"),
            format!("\"{name}\"")
        );
    }
}

#[test]
fn upstream_reasons_are_written_as_in_design() {
    let reasons = [
        (UpstreamReason::ConnectFailed, "connect_failed"),
        (UpstreamReason::ConnectTimeout, "connect_timeout"),
        (UpstreamReason::SendFailed, "send_failed"),
        (UpstreamReason::Shutdown, "shutdown"),
        (UpstreamReason::BadResponse, "bad_response"),
        (UpstreamReason::ResponseTimeout, "response_timeout"),
        (UpstreamReason::ResponseIncomplete, "response_incomplete"),
    ];
    for (reason, name) in reasons {
        assert_eq!(
            serde_json::to_string(&reason).expect("a reason serializes"),
            format!("\"{name}\"")
        );
    }
}

#[test]
fn the_design_example_parses() {
    let example = [
        r#"{"event":"start","seq":0,"run":"8c1f0a3e9b7d4c62a5e01f9d3b7c4e18","client":"weather-app","settings_sha256":"9f2c…","time":"2026-09-26T10:15:00Z"}"#,
        r#"{"event":"release","seq":1,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","time":"2026-09-26T10:15:02Z"}"#,
        r#"{"event":"refuse","seq":2,"client":"weather-app","keys":["weather_key"],"host":"evil.local:80","reason":"host_not_allowed","time":"2026-09-26T10:15:09Z"}"#,
        r#"{"event":"refuse","seq":3,"client":"weather-app","keys":[],"host":null,"reason":"malformed_request","time":"2026-09-26T10:15:11Z"}"#,
        r#"{"event":"upstream_error","seq":4,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","about":1,"reason":"response_timeout","time":"2026-09-26T10:15:32Z"}"#,
        r#"{"event":"stop","seq":5,"client":"weather-app","time":"2026-09-26T18:00:00Z"}"#,
    ];
    for line in example {
        parse_log_line(line).expect("each line of DESIGN's example parses");
    }
}

// Log injection

#[test]
fn a_quote_or_newline_in_the_host_cant_forge_a_line() {
    let line = LogLine::Release {
        seq: 1,
        client: "weather-app".to_string(),
        keys: NonEmpty::new("weather_key".to_string()),
        host: "a\"\n{\"event\":\"stop\"}".to_string(),
        time: TIME.to_string(),
    };
    let text = serde_json::to_string(&line).expect("a line serializes");
    assert_eq!(text.lines().count(), 1);
    assert_eq!(parse_log_line(&text).expect("the line parses"), line);
}

// Refused lines

#[test]
fn not_json_is_refused() {
    match parse_log_line("release weather_key") {
        Err(LogLineError::Json(error)) => assert!(error.is_syntax(), "{error}"),
        other => panic!("expected a syntax error, got {other:?}"),
    }
}

#[test]
fn a_line_without_an_event_is_refused() {
    let message = json_error(r#"{"seq":6,"client":"weather-app","time":"2026-09-26T10:15:00Z"}"#);
    assert!(message.contains("missing field `event`"), "{message}");
}

#[test]
fn an_unknown_event_is_refused() {
    let message = json_error(
        r#"{"event":"allow","seq":1,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("unknown variant `allow`"), "{message}");
}

#[test]
fn an_unknown_reason_is_refused() {
    let message = json_error(
        r#"{"event":"refuse","seq":3,"client":"weather-app","keys":[],"host":null,"reason":"too_slow","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("unknown variant `too_slow`"), "{message}");
}

#[test]
fn a_refuse_line_with_an_upstream_reason_is_refused() {
    let message = json_error(
        r#"{"event":"refuse","seq":3,"client":"weather-app","keys":[],"host":null,"reason":"connect_failed","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(
        message.contains("unknown variant `connect_failed`"),
        "{message}"
    );
}

#[test]
fn an_upstream_error_line_with_a_refuse_reason_is_refused() {
    let message = json_error(
        r#"{"event":"upstream_error","seq":5,"client":"weather-app","keys":[],"host":"api.weather.local:80","about":1,"reason":"host_mismatch","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(
        message.contains("unknown variant `host_mismatch`"),
        "{message}"
    );
}

#[test]
fn an_extra_field_is_refused() {
    let message = json_error(
        r#"{"event":"release","seq":1,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","secret":"x","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("unknown field `secret`"), "{message}");
}

#[test]
fn a_release_line_with_a_reason_is_refused() {
    let message = json_error(
        r#"{"event":"release","seq":1,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","reason":"host_not_allowed","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("unknown field `reason`"), "{message}");
}

#[test]
fn a_missing_field_is_refused() {
    let message = json_error(
        r#"{"event":"release","seq":1,"client":"weather-app","keys":["weather_key"],"time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("missing field `host`"), "{message}");
}

#[test]
fn a_refuse_line_without_its_host_is_refused() {
    let message = json_error(
        r#"{"event":"refuse","seq":4,"client":"weather-app","keys":[],"reason":"malformed_request","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("missing field `host`"), "{message}");
}

#[test]
fn a_release_line_without_keys_is_refused() {
    let message = json_error(
        r#"{"event":"release","seq":1,"client":"weather-app","keys":[],"host":"api.weather.local:80","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(
        message.contains("NonEmpty needs at least one element"),
        "{message}"
    );
}

#[test]
fn a_pass_line_with_keys_is_refused() {
    let result = parse_log_line(
        r#"{"event":"pass","seq":2,"client":"weather-app","keys":["weather_key"],"host":"api.weather.local:80","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(
        matches!(result, Err(LogLineError::PassWithKeys)),
        "{result:?}"
    );
}

#[test]
fn a_repeated_field_is_refused() {
    let message = json_error(
        r#"{"event":"stop","seq":6,"seq":7,"client":"weather-app","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("duplicate field `seq`"), "{message}");
}

#[test]
fn a_repeated_event_is_refused() {
    let message = json_error(
        r#"{"event":"stop","event":"stop","seq":6,"client":"weather-app","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("duplicate field `event`"), "{message}");
}

#[test]
fn a_negative_seq_is_refused() {
    let message = json_error(
        r#"{"event":"stop","seq":-1,"client":"weather-app","time":"2026-09-26T10:15:00Z"}"#,
    );
    assert!(message.contains("invalid value: integer `-1`"), "{message}");
}
