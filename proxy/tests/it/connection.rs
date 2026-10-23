// 4. One program connection: its exits rows up to the Server, and the Done-when checks of what
// reaches the fake API. The refusal rows are in "refusals", the response rows in "relay", the
// shutdown rows in "serving".

use crate::common::{
    KEY, LIMITS, Program, SECRETS, SHORT, arrivals, assert_closes_gently, checked, get, requests,
    settings_for, start_failing_proxy, start_fake_api, start_proxy, summary,
};
use proxy::{Limits, Stopped, log_queue, serve};
use std::time::{Duration, Instant};
use tally_core::Entry;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn the_program_closes_before_a_requests_first_byte() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    drop(Program::connect(proxy.address).await);
    sleep(Duration::from_millis(200)).await;

    let (served, lines) = proxy.stop().await;
    assert_eq!(served, Ok(()));
    assert_eq!(summary(&lines), ["start", "stop"], "no line");
    assert!(arrivals(&seen).is_empty());
}

#[tokio::test]
async fn silence_before_a_requests_first_byte_closes_the_connection() {
    let (server, _seen) = start_fake_api().await;
    let limits = Limits {
        idle: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    let started = Instant::now();
    assert_eq!(program.rest().await, b"", "no reply");
    assert!(started.elapsed() >= SHORT);

    let (_, lines) = proxy.stop().await;
    assert_eq!(summary(&lines), ["start", "stop"], "no line");
}

#[tokio::test]
async fn the_program_closes_part_way_through_the_head() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(b"GET http://api.weather.local/ok HTTP/1.1\r\nHost: api.wea")
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
        ["start", "refuse [] null incomplete_request"]
    );
    assert_eq!(proxy.stop().await.0, Ok(()));
}

// An empty line belongs to the head that follows it (DESIGN, Parser), so a program that closes
// after one has closed part-way through a head
#[tokio::test]
async fn the_program_closes_after_an_empty_line() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(b"\r\n").await;
    program.close_writing().await;
    assert_eq!(
        program.rest().await,
        b"",
        "no reply: the program has closed"
    );

    let lines = proxy.lines(2).await;
    assert_eq!(
        summary(&lines),
        ["start", "refuse [] null incomplete_request"]
    );
    assert_eq!(proxy.stop().await.0, Ok(()));
}

#[tokio::test]
async fn a_head_not_complete_in_time() {
    let (server, _seen) = start_fake_api().await;
    let limits = Limits {
        head: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(b"GET http://api.weather.local/ok HTTP/1.1\r\nHost: api.wea")
        .await;
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 408 Request Timeout\r\ncontent-length: 18\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\nincomplete_request"
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        ["start", "refuse [] null incomplete_request", "stop"]
    );
}

#[tokio::test]
async fn a_body_not_complete_in_time() {
    let (server, seen) = start_fake_api().await;
    let limits = Limits {
        body: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(b"POST http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nContent-Length: 10\r\n\r\nabcd")
        .await;
    let reply = program.rest().await;
    assert!(
        reply.starts_with(b"HTTP/1.1 408 Request Timeout\r\n"),
        "{}",
        String::from_utf8_lossy(&reply)
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "refuse [weather_key] api.weather.local:80 incomplete_request",
            "stop"
        ],
        "the line keeps the keys and host the decision found"
    );
    assert!(arrivals(&seen).is_empty());
}

// Protocol 1's row "The log fails while the proxy runs", and Done when: "a log that fails on
// write gives 503 and stops the proxy"
#[tokio::test]
async fn the_log_cant_be_written() {
    let (server, seen) = start_fake_api().await;
    // The start line is written, the next line fails
    let proxy = start_failing_proxy(&settings_for(server), SECRETS, LIMITS, 1).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/ok", KEY)).await;
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    );

    // The proxy stops by itself, without a stop line
    let (served, lines) = proxy.ended().await;
    assert_eq!(served, Err(Stopped::LogFailed));
    assert_eq!(summary(&lines), ["start"]);
    assert!(arrivals(&seen).is_empty(), "nothing is sent");
}

// The same row's "unless a final response head was relayed": a 503 after the head would read as
// the end of the body, hiding that the response was cut short
#[tokio::test]
async fn a_log_that_fails_after_the_response_head_sends_no_503() {
    let (server, _seen) = start_fake_api().await;
    // The start and pass lines are written, the upstream_error line fails
    let proxy = start_failing_proxy(&settings_for(server), SECRETS, LIMITS, 2).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/cut-body", "")).await;
    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabcd"
    );

    let (served, lines) = proxy.ended().await;
    assert_eq!(served, Err(Stopped::LogFailed));
    assert_eq!(summary(&lines), ["start", "pass api.weather.local:80"]);
}

#[tokio::test]
async fn a_log_that_fails_after_the_program_closed_sends_nothing() {
    let (server, _seen) = start_fake_api().await;
    // The start line is written, the refuse line fails
    let proxy = start_failing_proxy(&settings_for(server), SECRETS, LIMITS, 1).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(b"GET http://api.weather.local/ok HTTP/1.1\r\nHost: api.wea")
        .await;
    program.close_writing().await;
    assert_eq!(program.rest().await, b"", "no reply");

    let (served, lines) = proxy.ended().await;
    assert_eq!(served, Err(Stopped::LogFailed));
    assert_eq!(summary(&lines), ["start"]);
}

#[tokio::test]
async fn the_connection_to_the_server_fails() {
    // A port nobody listens on
    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let server = closed.local_addr().expect("the listener has an address");
    drop(closed);
    // Windows tries a refused connection again for about 2 s
    let limits = Limits {
        connect: Duration::from_secs(10),
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/ok", "")).await;
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n".to_string(),
            Vec::new()
        )
    );
    // The connection stays open
    program.send(&get("/ok", "")).await;
    assert!(
        program
            .head()
            .await
            .starts_with("HTTP/1.1 502 Bad Gateway\r\n")
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 connect_failed",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 3 connect_failed",
            "stop"
        ]
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn no_connection_to_the_server_in_time() {
    let (listener, _queued) = crate::common::server_that_never_accepts().await;
    let server = listener.local_addr().expect("the listener has an address");
    let limits = Limits {
        connect: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/ok", "")).await;
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 504 Gateway Timeout\r\ncontent-length: 0\r\n\r\n".to_string(),
            Vec::new()
        )
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 connect_timeout",
            "stop"
        ]
    );
}

// The row "Sending to the Server fails" is a unit test in "proxy/src/connection/tests.rs": over
// loopback, the system takes the whole request before a Server can break the connection.

// A Server that never reads is "nothing from the Server" (DESIGN, Timeouts). The system may take
// the whole request, so the proxy may wait for the response rather than on its send; the outcome
// is the same either way. The stalled send itself is a unit test in
// "proxy/src/connection/tests.rs".
#[tokio::test]
async fn a_server_that_doesnt_read_the_request() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let server = listener.local_addr().expect("the listener has an address");
    let holding = tokio::spawn(async move { listener.accept().await });
    let limits = Limits {
        server_silence: SHORT,
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    // The largest body the proxy forwards
    let mut request = b"POST http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nContent-Length: 1048576\r\n\r\n".to_vec();
    request.resize(request.len() + 1024 * 1024, b'x');
    program.send(&request).await;
    assert_eq!(
        program.response().await,
        (
            "HTTP/1.1 504 Gateway Timeout\r\ncontent-length: 0\r\n\r\n".to_string(),
            Vec::new()
        )
    );

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 response_timeout",
            "stop"
        ]
    );
    drop(holding);
}

// Done when: a valid request reaches the fake API with the real key, and its release line is
// in the log
#[tokio::test]
async fn the_real_key_reaches_the_server_in_its_bound_header() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/echo-headers", KEY)).await;
    let (_, echoed) = program.response().await;
    let lines = proxy.lines(2).await;

    let expected = "GET /echo-headers HTTP/1.1\r\nhost: api.weather.local\r\nauthorization: Bearer the-real-key\r\nconnection: close\r\n\r\n";
    assert_eq!(String::from_utf8_lossy(&echoed), expected);
    assert_eq!(
        summary(&lines),
        ["start", "release [weather_key] api.weather.local:80"]
    );
    assert_eq!(proxy.stop().await.0, Ok(()));
    assert_eq!(requests(&seen).len(), 1);
}

// Done when: a request whose Host names another host reaches the fake API with the URL's host as
// its Host. The proxy ignores it (RFC 9112 §3.2.2), so it can't steer a key either
#[tokio::test]
async fn a_host_header_naming_another_host_is_replaced_by_the_urls() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(
            format!(
                "GET http://api.weather.local/echo-headers HTTP/1.1\r\nHost: other.local\r\n{KEY}\r\n"
            )
            .as_bytes(),
        )
        .await;
    let (_, echoed) = program.response().await;

    let expected = "GET /echo-headers HTTP/1.1\r\nhost: api.weather.local\r\nauthorization: Bearer the-real-key\r\nconnection: close\r\n\r\n";
    assert_eq!(String::from_utf8_lossy(&echoed), expected);
    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        [
            "start",
            "release [weather_key] api.weather.local:80",
            "stop"
        ]
    );
    assert_eq!(requests(&seen).len(), 1);
}

// Done when: a test holds the Writer's answer and checks that nothing reaches the fake API until
// it's released
#[tokio::test]
async fn nothing_reaches_the_server_before_the_writer_answers() {
    let (server, seen) = start_fake_api().await;
    let (settings, secrets) = checked(&settings_for(server), SECRETS);
    // The test takes the Writer's place
    let (writer, mut queue) = log_queue(16);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let address = listener.local_addr().expect("the listener has an address");
    let (shutdown, signal) = oneshot::channel::<()>();
    let served = tokio::spawn(serve(
        listener,
        settings,
        secrets,
        LIMITS,
        writer,
        CancellationToken::new(),
        async move {
            let _ = signal.await;
        },
    ));
    let mut program = Program::connect(address).await;

    program.send(&get("/ok", KEY)).await;
    let (entry, answer_to) = timeout(Duration::from_secs(5), queue.recv())
        .await
        .expect("the release line arrives within 5 s")
        .expect("the queue is open");
    assert!(
        matches!(&entry, Entry::Release { keys, host }
            if keys.len() == 1 && keys.head == "weather_key" && host == "api.weather.local:80"),
        "{entry:?}"
    );
    sleep(Duration::from_millis(300)).await;
    assert!(arrivals(&seen).is_empty(), "nothing before the answer");

    answer_to
        .send(1)
        .expect("the connection waits for its answer");
    assert_eq!(program.response().await.1, b"ok");
    assert_eq!(requests(&seen).len(), 1, "the request after the answer");

    shutdown.send(()).expect("serve is running");
    let ended = timeout(Duration::from_secs(5), served)
        .await
        .expect("serve ends within 5 s")
        .expect("serve doesn't panic");
    assert_eq!(ended.0, Ok(()));
}

// Protocol 4's row "Refused by decide" ends with a gentle close
#[tokio::test]
async fn a_refused_request_is_closed_gently() {
    assert_closes_gently(
        b"GET http://evil.local/ HTTP/1.1\r\nHost: evil.local\r\n\r\n",
        "HTTP/1.1 403 Forbidden\r\n",
    )
    .await;
}

// Done when: a request without a placeholder to a listed host is forwarded and logged as pass
#[tokio::test]
async fn a_request_without_a_key_to_a_listed_host_passes() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/ok", "")).await;
    assert_eq!(program.response().await.1, b"ok");

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        ["start", "pass api.weather.local:80", "stop"]
    );
    assert_eq!(requests(&seen).len(), 1);
}

// Done when: a placeholder outside its key's bound header reaches the fake API unchanged
#[tokio::test]
async fn a_placeholder_outside_its_bound_header_reaches_the_server_unchanged() {
    let (server, seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(&get(
            "/ok?key=TALLY_0123456789abcdef0123456789abcdef",
            "X-Api-Key: TALLY_0123456789abcdef0123456789abcdef\r\n",
        ))
        .await;
    assert_eq!(program.response().await.1, b"ok");

    let (_, lines) = proxy.stop().await;
    assert_eq!(
        summary(&lines),
        ["start", "pass api.weather.local:80", "stop"],
        "no key in its bound header"
    );
    let reached = requests(&seen);
    assert_eq!(
        String::from_utf8_lossy(&reached[0].1.raw),
        "GET /ok?key=TALLY_0123456789abcdef0123456789abcdef HTTP/1.1\r\nhost: api.weather.local\r\nx-api-key: TALLY_0123456789abcdef0123456789abcdef\r\nconnection: close\r\n\r\n"
    );
}

// Done when: a request with two keys, one allowed for the host and one not, sends nothing
#[tokio::test]
async fn two_keys_one_not_allowed_sends_nothing() {
    let (server, seen) = start_fake_api().await;
    let settings = include_str!("../../../tally-core/tests/it/fixtures/two-keys.toml")
        .replace("127.0.0.1:9001", &server.to_string());
    let secrets = include_str!("../../../tally-core/tests/it/fixtures/two-keys-secrets.toml");
    let proxy = start_proxy(&settings, secrets, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    program
        .send(&get(
            "/ok",
            "Authorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nX-Api-Key: TALLY_fedcba9876543210fedcba9876543210\r\n",
        ))
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
            "refuse [maps_key,weather_key] api.weather.local:80 host_not_allowed",
            "stop"
        ]
    );
    assert!(arrivals(&seen).is_empty());
}
