// 5. Relay response: protocol 4's exits rows from "Bad response" to "Program stops reading",
// against the fake API's endpoints.

use crate::common::{
    KEY, LIMITS, Program, SECRETS, SHORT, arrivals, assert_closes_gently, get, settings_for,
    start_fake_api, start_proxy, summary,
};
use proxy::Limits;
use std::time::Duration;
use tokio::time::{sleep, timeout};

/// "path" gets a 502 with "connection: close", then the connection closes, and an
/// upstream_error, bad_response line.
async fn assert_bad_response(path: &str) {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get(path, "")).await;
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 bad_response",
            "stop"
        ]
    );
}

#[tokio::test]
async fn a_101_is_a_bad_response() {
    assert_bad_response("/switch").await;
}

#[tokio::test]
async fn a_1xx_with_content_length_is_a_bad_response() {
    assert_bad_response("/bad-interim").await;
}

#[tokio::test]
async fn a_head_cut_off_by_the_servers_close_is_a_bad_response() {
    assert_bad_response("/cut-head").await;
}

// Protocol 4's row "Bad response" ends with a gentle close, through step 7's Close
#[tokio::test]
async fn a_bad_response_is_closed_gently() {
    assert_closes_gently(&get("/switch", ""), "HTTP/1.1 502 Bad Gateway\r\n").await;
}

#[tokio::test]
async fn silence_before_the_response_head() {
    let (server, _seen) = start_fake_api().await;
    let limits = Limits {
        server_silence: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/silent", KEY)).await;
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 504 Gateway Timeout\r\ncontent-length: 0\r\n\r\n".to_string(),
            Vec::new()
        )
    );
    // The connection stays open
    program.send(&get("/ok", "")).await;
    assert_eq!(program.response().await.1, b"ok");

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "release [weather_key] api.weather.local:80",
            "upstream_error [weather_key] api.weather.local:80 about 1 response_timeout",
            "pass api.weather.local:80",
            "stop"
        ]
    );
}

#[tokio::test]
async fn a_response_cut_short_stops_where_it_was_cut() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/cut-body", "")).await;
    // Fewer bytes than announced, then the close: the program knows it's incomplete
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabcd"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 response_incomplete",
            "stop"
        ]
    );
}

/// "path" sends a head and part of its body, then goes silent: the program gets what came, then
/// the close, and the log an upstream_error, response_incomplete line.
async fn assert_cut_short_by_silence(path: &str, relayed: &[u8]) {
    let (server, _seen) = start_fake_api().await;
    let limits = Limits {
        server_silence: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get(path, "")).await;
    assert_eq!(
        String::from_utf8_lossy(&program.rest().await),
        String::from_utf8_lossy(relayed)
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 response_incomplete",
            "stop"
        ]
    );
}

#[tokio::test]
async fn silence_inside_a_body_with_a_length_cuts_the_response_short() {
    assert_cut_short_by_silence(
        "/stall-body",
        b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabcd",
    )
    .await;
}

#[tokio::test]
async fn silence_inside_a_chunked_body_cuts_the_response_short() {
    assert_cut_short_by_silence(
        "/stall-chunked",
        b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n",
    )
    .await;
}

// RFC 9112 §8: a chunked body "is incomplete if the zero-sized chunk that terminates the encoding
// has not been received"
#[tokio::test]
async fn a_chunked_body_closed_before_its_last_chunk_is_cut_short() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/cut-chunked", "")).await;
    assert_eq!(
        String::from_utf8_lossy(&program.rest().await),
        "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 response_incomplete",
            "stop"
        ]
    );
}

// The bytes that break the framing never reach the program, so its body lacks its last chunk
#[tokio::test]
async fn a_chunked_body_with_broken_framing_is_cut_short() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/bad-chunked", "")).await;
    let head = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n";
    let relayed = program.rest().await;
    assert!(
        relayed.starts_with(head) && relayed.len() <= head.len() + b"3\r\nabc\r\n".len(),
        "{:?}",
        String::from_utf8_lossy(&relayed)
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 response_incomplete",
            "stop"
        ]
    );
}

// A response with no length ends with the close: RFC 9112 §8 counts it complete "unless an error
// was indicated by the underlying connection". The limits stay long, so only the reset can cut it.
#[tokio::test]
async fn a_reset_inside_a_body_that_ends_with_the_close_cuts_the_response_short() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/reset-body", "")).await;
    assert_eq!(
        String::from_utf8_lossy(&program.rest().await),
        "HTTP/1.1 200 OK\r\nconnection: close\r\n\r\nthe body is cut by a reset"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 response_incomplete",
            "stop"
        ]
    );
}

// The fake API says "Connection: close" on every response: the proxy drops it, so the program's
// connection stays open
#[tokio::test]
async fn a_response_with_content_length_keeps_the_connection_open() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/ok", "")).await;
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n".to_string(),
            b"ok".to_vec()
        )
    );
    program.send(&get("/ok", "")).await;
    assert_eq!(program.response().await.1, b"ok");

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "pass api.weather.local:80",
            "stop"
        ]
    );
}

#[tokio::test]
async fn a_204_keeps_the_connection_open() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/no-content", "")).await;
    assert_eq!(
        program.response().await,
        ("HTTP/1.1 204 No Content\r\n\r\n".to_string(), Vec::new())
    );
    program.send(&get("/ok", "")).await;
    assert_eq!(program.response().await.1, b"ok");
    assert_eq!(proxy.stop().await.0, Ok(()));
}

// The proxy knows where a chunked response ends, so the connection stays open, as after
// Content-Length
#[tokio::test]
async fn a_chunked_response_is_relayed_to_its_last_chunk_and_the_connection_stays_open() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/chunked", "")).await;
    assert_eq!(
        program.head().await,
        "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n"
    );
    let body = b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
    assert_eq!(program.take(body.len()).await, body);
    program.send(&get("/ok", "")).await;
    assert_eq!(program.response().await.1, b"ok");

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "pass api.weather.local:80",
            "stop"
        ]
    );
}

// RFC 9112 §7.1: "chunked-body = *chunk last-chunk trailer-section CRLF"
#[tokio::test]
async fn a_chunked_response_with_a_trailer_is_relayed_whole() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/chunked-trailers", "")).await;
    program.head().await;
    let body = b"3\r\nabc\r\n0\r\nx-checksum: 1\r\n\r\n";
    assert_eq!(program.take(body.len()).await, body);
    program.send(&get("/ok", "")).await;
    assert_eq!(program.response().await.1, b"ok");
    assert_eq!(proxy.stop().await.0, Ok(()));
}

// The Server keeps the connection open after the last chunk, the next request is answered well
// before the Server-silence limit, so the response ended at its last chunk, not at a close
#[tokio::test]
async fn a_chunked_response_ends_at_its_last_chunk_without_waiting_for_the_close() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/chunked-held-open", "")).await;
    let answered = timeout(LIMITS.server_silence / 2, async {
        program.head().await;
        let body = b"3\r\nabc\r\n0\r\n\r\n";
        assert_eq!(program.take(body.len()).await, body);
        program.send(&get("/ok", "")).await;
        program.response().await
    })
    .await
    .expect("the response ends at its last chunk");
    assert_eq!(answered.1, b"ok");

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "pass api.weather.local:80",
            "stop"
        ]
    );
}

#[tokio::test]
async fn a_response_with_no_length_is_relayed_until_the_server_closes() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/no-length", "")).await;
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\nthe body ends when the connection closes"
    );
    assert_eq!(proxy.stop().await.0, Ok(()));
}

#[tokio::test]
async fn an_interim_head_is_relayed_as_it_arrives() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/early-hints", "")).await;
    let interim = timeout(Duration::from_millis(400), program.head())
        .await
        .expect("the 103 arrives before the Server's 500 ms pause ends");
    assert_eq!(
        interim,
        "HTTP/1.1 103 Early Hints\r\nlink: </style.css>; rel=preload\r\n\r\n"
    );
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n".to_string(),
            b"ok".to_vec()
        )
    );
    assert_eq!(proxy.stop().await.0, Ok(()));
}

#[tokio::test]
async fn a_program_that_stops_reading_is_cut_off() {
    let (server, _seen) = start_fake_api().await;
    let limits = Limits {
        write: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    // "/endless" never ends; the program doesn't read
    program.send(&get("/endless", "")).await;
    sleep(Duration::from_secs(1)).await;
    // What was sent before the cut-off, then the close
    let rest = program.rest().await;
    assert!(rest.starts_with(b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\nxxx"));

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        ["start", "pass api.weather.local:80", "stop"],
        "no new line: the request already has its line"
    );
}

// Done when: a redirect that takes the program's placeholder to another host is refused as
// host_not_allowed
#[tokio::test]
async fn a_redirect_to_another_host_doesnt_take_the_key_along() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/redirect", KEY)).await;
    let (head, _) = program.response().await;
    assert!(
        head.contains("location: http://elsewhere.local/\r\n"),
        "{head}"
    );
    // The program follows it with its placeholder
    program
        .send(b"GET http://elsewhere.local/ HTTP/1.1\r\nHost: elsewhere.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n")
        .await;
    assert!(
        program
            .head()
            .await
            .starts_with("HTTP/1.1 403 Forbidden\r\n")
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "release [weather_key] api.weather.local:80",
            "refuse [weather_key] elsewhere.local:80 host_not_allowed",
            "stop"
        ]
    );
    assert_eq!(
        arrivals(&seen).len(),
        1,
        "only the first request reaches it"
    );
}
