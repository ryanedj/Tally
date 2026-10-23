// 1. Proxy startup, serving and shutting down, in-process: its row "Shutdown signal", and
// protocol 4's shutdown rows. The binary's own signal handling is in "startup".

use crate::common::{
    KEY, LIMITS, Program, SECRETS, arrivals, checked, get, settings_for, start_fake_api,
    start_proxy, summary,
};
use proxy::{Limits, Stopped, log_queue, serve};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

// DESIGN, Shutdown, step 2: the requests in progress finish, with "connection: close"
#[tokio::test]
async fn a_shutdown_lets_the_request_in_progress_finish() {
    let (server, _seen) = start_fake_api().await;
    let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
    let mut program = Program::connect(proxy.address).await;

    // The Server pauses 500 ms after its 103
    program.send(&get("/early-hints", "")).await;
    assert!(
        program
            .head()
            .await
            .starts_with("HTTP/1.1 103 Early Hints\r\n")
    );
    let stopping = tokio::spawn(proxy.stop());

    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
    );
    let (served, lines) = stopping.await.expect("stop doesn't panic");
    assert_eq!(served, Ok(()));
    assert_eq!(
        summary(&lines),
        ["start", "pass api.weather.local:80", "stop"]
    );
}

// The Writer answers Error before it sets log_failed. A connection that gets that answer during a
// shutdown, as the last one, must still end the run as a failed log: no stop line
#[tokio::test]
async fn a_log_failure_seen_during_a_shutdown_ends_the_run_as_failed() {
    let (server, _seen) = start_fake_api().await;
    let (settings, secrets) = checked(&settings_for(server), SECRETS);
    // The test takes the Writer's place, and never sets log_failed itself
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
    let (_, answer_to) = timeout(Duration::from_secs(5), queue.recv())
        .await
        .expect("the release line arrives within 5 s")
        .expect("the queue is open");
    shutdown.send(()).expect("serve is running");
    sleep(Duration::from_millis(100)).await;
    // The Error answer
    drop(answer_to);

    assert_eq!(
        program.rest().await,
        b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    );
    let ended = timeout(Duration::from_secs(5), served)
        .await
        .expect("serve ends within 5 s")
        .expect("serve doesn't panic");
    assert_eq!(ended.0, Err(Stopped::LogFailed));
}

// 1. Proxy startup's row "The log doesn't answer within 4 s of the cut-off". A connection's wait
// for its line is never cut off, so without the limit a stuck disk would hold the run forever
#[tokio::test]
async fn a_log_that_stops_answering_ends_the_run_at_its_limit() {
    let (server, _seen) = start_fake_api().await;
    let (settings, secrets) = checked(&settings_for(server), SECRETS);
    // The test takes the Writer's place, and never answers
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
    let (_, answer_to) = timeout(Duration::from_secs(5), queue.recv())
        .await
        .expect("the release line arrives within 5 s")
        .expect("the queue is open");
    let started = Instant::now();
    shutdown.send(()).expect("serve is running");

    let ended = timeout(Duration::from_secs(10), served)
        .await
        .expect("serve ends within 10 s")
        .expect("serve doesn't panic");
    assert_eq!(ended.0, Err(Stopped::LogTimedOut));
    let limit = LIMITS.shutdown + LIMITS.end_of_run;
    assert!(
        started.elapsed() >= limit && started.elapsed() < limit + Duration::from_secs(1),
        "the shutdown limit, then the end-of-run limit: {:?}",
        started.elapsed()
    );
    drop(answer_to);
}

#[tokio::test]
async fn a_shutdown_closes_a_connection_before_its_requests_first_byte() {
    let (server, _seen) = start_fake_api().await;
    let limits = Limits {
        idle: Duration::from_secs(10),
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;
    // Let the proxy accept it
    sleep(Duration::from_millis(100)).await;

    let started = Instant::now();
    let stopping = tokio::spawn(proxy.stop());
    assert_eq!(program.rest().await, b"", "no reply");
    assert!(
        started.elapsed() < LIMITS.shutdown,
        "closed at once, not at the cut-off"
    );

    let (served, lines) = stopping.await.expect("stop doesn't panic");
    assert_eq!(served, Ok(()));
    assert_eq!(summary(&lines), ["start", "stop"], "no line");
}

#[tokio::test]
async fn the_shutdown_limit_cuts_off_a_wait_for_the_response() {
    let (server, seen) = start_fake_api().await;
    let limits = Limits {
        server_silence: Duration::from_secs(10),
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/silent", "")).await;
    while arrivals(&seen).is_empty() {
        sleep(Duration::from_millis(10)).await;
    }
    let (served, lines) = proxy.stop().await;
    assert_eq!(program.rest().await, b"", "no reply");

    assert_eq!(served, Ok(()));
    assert_eq!(
        summary(&lines),
        ["start", "pass api.weather.local:80", "stop"],
        "nothing new: the request was sent in full"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn the_shutdown_limit_cuts_off_a_connect() {
    let (listener, _queued) = crate::common::server_that_never_accepts().await;
    let server = listener.local_addr().expect("the listener has an address");
    let limits = Limits {
        connect: Duration::from_secs(10),
        ..LIMITS
    };
    let proxy = start_proxy(&settings_for(server), SECRETS, limits).await;
    let mut program = Program::connect(proxy.address).await;

    program.send(&get("/ok", "")).await;
    let lines = proxy.lines(2).await;
    assert_eq!(summary(&lines), ["start", "pass api.weather.local:80"]);
    let (served, lines) = proxy.stop().await;
    assert_eq!(program.rest().await, b"", "no reply");

    assert_eq!(served, Ok(()));
    assert_eq!(
        summary(&lines),
        [
            "start",
            "pass api.weather.local:80",
            "upstream_error [] api.weather.local:80 about 1 shutdown",
            "stop"
        ]
    );
}
