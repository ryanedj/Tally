use crate::common::checked;
use tally_core::{Decision, Parsed, Refusal, RefuseReason, decide, parse_request_head};

const SETTINGS: &str = include_str!("fixtures/two-keys.toml");
const SECRETS: &str = include_str!("fixtures/two-keys-secrets.toml");

/// What "decide" says about "request", with the settings above.
fn decision(request: &[u8]) -> Decision {
    let (settings, _) = checked(SETTINGS, SECRETS);
    let head = match parse_request_head(request) {
        Ok(Parsed::Complete(head, _)) => head,
        other => panic!("expected a complete head, got {other:?}"),
    };
    decide(&head, &settings)
}

fn refused(reason: RefuseReason, keys: &[&str], host: &str) -> Decision {
    Decision::Refuse(Refusal {
        reason,
        keys: keys.iter().map(|key| key.to_string()).collect(),
        host: Some(host.to_string()),
    })
}

// Case 1: host_mismatch

#[test]
fn case_1_a_host_header_that_disagrees() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: maps.local:8080\r\n\r\n"),
        refused(RefuseReason::HostMismatch, &[], "api.weather.local:80")
    );
}

#[test]
fn case_1_no_host_header() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\n\r\n"),
        refused(RefuseReason::HostMismatch, &[], "api.weather.local:80")
    );
}

#[test]
fn case_1_two_host_headers() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nHost: api.weather.local\r\n\r\n"),
        refused(RefuseReason::HostMismatch, &[], "api.weather.local:80")
    );
}

#[test]
fn case_1_a_host_header_the_parser_refuses() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api_weather.local\r\n\r\n"),
        refused(RefuseReason::HostMismatch, &[], "api.weather.local:80")
    );
}

#[test]
fn case_1_a_host_header_with_another_port() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local:8080\r\n\r\n"),
        refused(RefuseReason::HostMismatch, &[], "api.weather.local:80")
    );
}

// A "Host" without a port means ":80", which isn't the URL's ":8080".
#[test]
fn case_1_a_host_header_without_the_urls_port() {
    assert_eq!(
        decision(b"GET http://maps.local:8080/ HTTP/1.1\r\nHost: maps.local\r\n\r\n"),
        refused(RefuseReason::HostMismatch, &[], "maps.local:8080")
    );
}

#[test]
fn case_1_keeps_the_keys_found() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: maps.local:8080\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n"),
        refused(
            RefuseReason::HostMismatch,
            &["weather_key"],
            "api.weather.local:80"
        )
    );
}

#[test]
fn a_host_header_spelled_differently_agrees() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: API.Weather.Local:80\r\n\r\n"),
        Decision::Pass {
            host: "api.weather.local:80".to_string()
        }
    );
}

// Case 2: malformed_request (the bound header of a key found, twice)

#[test]
fn case_2_a_bound_header_twice() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nAuthorization: Basic x\r\n\r\n"),
        refused(
            RefuseReason::MalformedRequest,
            &["weather_key"],
            "api.weather.local:80"
        )
    );
}

#[test]
fn a_bound_header_twice_without_its_placeholder_is_not_counted() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: a\r\nAuthorization: b\r\n\r\n"),
        Decision::Pass {
            host: "api.weather.local:80".to_string()
        }
    );
}

// Case 3: host_not_allowed (a key not allowed for this host)

#[test]
fn case_3_a_key_sent_to_another_host() {
    assert_eq!(
        decision(b"GET http://maps.local:8080/ HTTP/1.1\r\nHost: maps.local:8080\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n"),
        refused(
            RefuseReason::HostNotAllowed,
            &["weather_key"],
            "maps.local:8080"
        )
    );
}

// Done when: a request with two keys, one allowed for the host and one not, sends nothing.
#[test]
fn case_3_two_keys_one_not_allowed() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nX-Api-Key: TALLY_fedcba9876543210fedcba9876543210\r\n\r\n"),
        refused(
            RefuseReason::HostNotAllowed,
            &["maps_key", "weather_key"],
            "api.weather.local:80"
        )
    );
}

// Case 4: host_not_allowed (no key, host not listed)

#[test]
fn case_4_no_key_to_an_unlisted_host() {
    assert_eq!(
        decision(b"GET http://evil.local/ HTTP/1.1\r\nHost: evil.local\r\n\r\n"),
        refused(RefuseReason::HostNotAllowed, &[], "evil.local:80")
    );
}

// Case 5: pass

#[test]
fn case_5_no_key_to_a_listed_host() {
    assert_eq!(
        decision(
            b"GET http://api.weather.local/forecast HTTP/1.1\r\nHost: api.weather.local\r\n\r\n"
        ),
        Decision::Pass {
            host: "api.weather.local:80".to_string()
        }
    );
}

#[test]
fn a_placeholder_outside_its_bound_header_does_not_count() {
    assert_eq!(
        decision(b"GET http://api.weather.local/?k=TALLY_0123456789abcdef0123456789abcdef HTTP/1.1\r\nHost: api.weather.local\r\nX-Other: TALLY_0123456789abcdef0123456789abcdef\r\nX-Api-Key: TALLY_0123456789abcdef0123456789abcdef\r\n\r\n"),
        Decision::Pass {
            host: "api.weather.local:80".to_string()
        }
    );
}

// Case 6: release

#[test]
fn case_6_a_key_in_its_bound_header() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n"),
        Decision::Release {
            keys: vec!["weather_key".to_string()],
            host: "api.weather.local:80".to_string()
        }
    );
}

#[test]
fn a_bound_header_in_any_case() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nAUTHORIZATION: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n"),
        Decision::Release {
            keys: vec!["weather_key".to_string()],
            host: "api.weather.local:80".to_string()
        }
    );
}

// Which case wins: the first that applies, in protocol 3's order

#[test]
fn case_1_comes_before_case_3() {
    assert_eq!(
        decision(b"GET http://maps.local:8080/ HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n"),
        refused(
            RefuseReason::HostMismatch,
            &["weather_key"],
            "maps.local:8080"
        )
    );
}

#[test]
fn case_2_comes_before_case_3() {
    assert_eq!(
        decision(b"GET http://maps.local:8080/ HTTP/1.1\r\nHost: maps.local:8080\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nAuthorization: Basic x\r\n\r\n"),
        refused(
            RefuseReason::MalformedRequest,
            &["weather_key"],
            "maps.local:8080"
        )
    );
}

#[test]
fn case_1_comes_before_case_2() {
    assert_eq!(
        decision(b"GET http://api.weather.local/ HTTP/1.1\r\nHost: maps.local:8080\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nAuthorization: Basic x\r\n\r\n"),
        refused(
            RefuseReason::HostMismatch,
            &["weather_key"],
            "api.weather.local:80"
        )
    );
}

#[test]
fn a_key_on_a_port_written_out() {
    assert_eq!(
        decision(b"GET http://maps.local:8080/tiles HTTP/1.1\r\nHost: maps.local:8080\r\nX-Api-Key: TALLY_fedcba9876543210fedcba9876543210\r\n\r\n"),
        Decision::Release {
            keys: vec!["maps_key".to_string()],
            host: "maps.local:8080".to_string()
        }
    );
}
