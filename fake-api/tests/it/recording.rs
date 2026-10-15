use crate::common::{arrivals, exchange, get, request_connections, requests, start, wait_until};
use fake_api::Arrival;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{sleep, timeout},
};

const OK_REPLY: &[u8] = b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok";

#[tokio::test]
async fn a_request_is_recorded_exactly() {
    let (addr, seen) = start().await;
    let request = b"GET /ok?x=1 HTTP/1.1\r\nHost: test\r\nX-Key: abc\r\n\r\n";
    exchange(addr, request).await;

    let requests = requests(&arrivals(&seen));
    assert_eq!(requests.len(), 1);
    let received = requests.first().expect("one request was recorded");
    assert_eq!(received.head.method, "GET");
    assert_eq!(received.head.target, "/ok?x=1");
    assert_eq!(
        received.head.headers,
        vec![
            ("Host".to_string(), b"test".to_vec()),
            ("X-Key".to_string(), b"abc".to_vec()),
        ]
    );
    assert_eq!(received.raw, request);
    assert!(received.body.is_empty());
}

#[tokio::test]
async fn a_body_is_recorded() {
    let (addr, seen) = start().await;
    let request = b"POST /ok HTTP/1.1\r\nHost: test\r\nContent-Length: 5\r\n\r\nhello";
    exchange(addr, request).await;

    let requests = requests(&arrivals(&seen));
    let received = requests.first().expect("one request was recorded");
    assert_eq!(received.body, b"hello");
    assert_eq!(received.raw, request);
}

#[tokio::test]
async fn a_body_sent_after_its_head_is_one_request() {
    let (addr, seen) = start().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"POST /ok HTTP/1.1\r\nHost: test\r\nContent-Length: 5\r\n\r\n")
        .await
        .expect("send the head");
    // Long enough for the fake API to read the head on its own.
    sleep(Duration::from_millis(50)).await;
    stream.write_all(b"hello").await.expect("send the body");

    let mut reply = Vec::new();
    timeout(Duration::from_secs(5), stream.read_to_end(&mut reply))
        .await
        .expect("the fake API closes within 5 s")
        .expect("read the reply");
    assert_eq!(reply, OK_REPLY);
    let arrived = arrivals(&seen);
    assert!(
        matches!(arrived.as_slice(), [Arrival::Request(r)] if r.body == b"hello"),
        "expected one request, got {arrived:?}"
    );
}

#[tokio::test]
async fn a_head_that_arrives_in_two_pieces_is_one_request() {
    let (addr, seen) = start().await;
    let request = get("/ok");
    let (first, second) = request.split_at(10);
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(first).await.expect("send the first piece");
    sleep(Duration::from_millis(50)).await;
    stream
        .write_all(second)
        .await
        .expect("send the second piece");
    let mut reply = Vec::new();
    timeout(Duration::from_secs(5), stream.read_to_end(&mut reply))
        .await
        .expect("the fake API closes within 5 s")
        .expect("read the reply");

    assert_eq!(reply, OK_REPLY);
    assert_eq!(requests(&arrivals(&seen)).len(), 1);
}

#[tokio::test]
async fn glued_requests_are_recorded_in_order_and_only_the_first_is_answered() {
    let (addr, seen) = start().await;
    let glued = [get("/ok"), get("/redirect")].concat();
    let reply = exchange(addr, &glued).await;

    assert_eq!(reply, OK_REPLY);
    let arrived = wait_until(&seen, |arrived| requests(arrived).len() == 2).await;
    let requests = requests(&arrived);
    let targets: Vec<&str> = requests.iter().map(|r| r.head.target.as_str()).collect();
    assert_eq!(targets, vec!["/ok", "/redirect"]);
    // Both came on the same connection, which is what a test at L5 must be able to see.
    assert_eq!(request_connections(&seen), vec![1, 1]);
}

#[tokio::test]
async fn a_request_sent_after_the_reply_is_recorded_too() {
    let (addr, seen) = start().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(&get("/ok"))
        .await
        .expect("send the first request");
    let mut reply = vec![0u8; OK_REPLY.len()];
    timeout(Duration::from_secs(5), stream.read_exact(&mut reply))
        .await
        .expect("the reply arrives within 5 s")
        .expect("read the reply");
    assert_eq!(reply, OK_REPLY);

    stream
        .write_all(&get("/redirect"))
        .await
        .expect("send the second request");
    drop(stream);

    let arrived = wait_until(&seen, |arrived| requests(arrived).len() == 2).await;
    let requests = requests(&arrived);
    let targets: Vec<&str> = requests.iter().map(|r| r.head.target.as_str()).collect();
    assert_eq!(targets, vec!["/ok", "/redirect"]);
    assert_eq!(request_connections(&seen), vec![1, 1]);
}

#[tokio::test]
async fn separate_connections_get_separate_numbers() {
    let (addr, seen) = start().await;
    exchange(addr, &get("/ok")).await;
    exchange(addr, &get("/ok")).await;

    wait_until(&seen, |arrived| requests(arrived).len() == 2).await;
    assert_eq!(request_connections(&seen), vec![1, 2]);
}

#[tokio::test]
async fn leftover_bytes_that_are_not_a_request_are_reported() {
    let (addr, seen) = start().await;
    let request = [get("/ok"), b"GARBAGE".to_vec()].concat();
    exchange(addr, &request).await;

    let arrived = wait_until(&seen, |arrived| {
        matches!(arrived.last(), Some(Arrival::NotARequest(_)))
    })
    .await;
    assert_eq!(requests(&arrived).len(), 1);
    assert_eq!(
        arrived.last(),
        Some(&Arrival::NotARequest(b"GARBAGE".to_vec()))
    );
}

#[tokio::test]
async fn a_malformed_request_gets_a_400_and_is_reported() {
    let (addr, seen) = start().await;
    let request = b"GET /a b c HTTP/1.1\r\n\r\n";
    let reply = exchange(addr, request).await;

    assert_eq!(
        reply,
        b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    assert_eq!(
        arrivals(&seen),
        vec![Arrival::NotARequest(request.to_vec())]
    );
}

#[tokio::test]
async fn a_request_cut_short_is_reported() {
    let (addr, seen) = start().await;
    let half = b"GET /ok HTTP/1.1\r\nHost: te";
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(half).await.expect("send half a head");
    drop(stream);

    let arrived = wait_until(&seen, |arrived| !arrived.is_empty()).await;
    assert_eq!(arrived, vec![Arrival::NotARequest(half.to_vec())]);
}

#[tokio::test]
async fn a_request_cut_short_by_a_reset_is_reported() {
    let (addr, seen) = start().await;
    let half = b"GET /ok HTTP/1.1\r\nHost: te";
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(half).await.expect("send half a head");
    sleep(Duration::from_millis(50)).await;
    // An abortive close: the fake API gets a reset, not a plain close.
    stream.set_zero_linger().expect("ask for an abortive close");
    drop(stream);

    let arrived = wait_until(&seen, |arrived| !arrived.is_empty()).await;
    assert_eq!(arrived, vec![Arrival::NotARequest(half.to_vec())]);
}

#[tokio::test]
async fn stray_bytes_are_reported_when_the_reply_cannot_be_sent() {
    let (addr, seen) = start().await;
    let sent = [get("/early-hints"), b"STRAY".to_vec()].concat();
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(&sent)
        .await
        .expect("send the request and the stray bytes");
    // Reset during the 500 ms pause: the fake API's second write then fails.
    sleep(Duration::from_millis(100)).await;
    stream.set_zero_linger().expect("ask for an abortive close");
    drop(stream);

    let arrived = wait_until(&seen, |arrived| {
        matches!(arrived.last(), Some(Arrival::NotARequest(_)))
    })
    .await;
    assert_eq!(
        arrived.last(),
        Some(&Arrival::NotARequest(b"STRAY".to_vec()))
    );
}

#[tokio::test]
async fn a_body_cut_short_is_reported() {
    let (addr, seen) = start().await;
    let sent = b"POST /ok HTTP/1.1\r\nHost: test\r\nContent-Length: 10\r\n\r\nabcd";
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(sent)
        .await
        .expect("send the head and part of the body");
    drop(stream);

    let arrived = wait_until(&seen, |arrived| !arrived.is_empty()).await;
    assert_eq!(arrived, vec![Arrival::NotARequest(sent.to_vec())]);
}

#[tokio::test]
async fn bytes_after_a_malformed_head_are_reported() {
    let (addr, seen) = start().await;
    let malformed = b"GET /a b c HTTP/1.1\r\n\r\n";
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(malformed)
        .await
        .expect("send the malformed request");
    let mut reply = Vec::new();
    timeout(Duration::from_secs(5), stream.read_to_end(&mut reply))
        .await
        .expect("the fake API closes its side within 5 s")
        .expect("read the reply");
    assert_eq!(
        reply,
        b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );

    stream
        .write_all(b"more bytes")
        .await
        .expect("send more bytes after the 400");
    drop(stream);

    let arrived = wait_until(&seen, |arrived| arrived.len() == 2).await;
    assert_eq!(
        arrived,
        vec![
            Arrival::NotARequest(malformed.to_vec()),
            Arrival::NotARequest(b"more bytes".to_vec()),
        ]
    );
}

#[tokio::test]
async fn a_request_over_the_read_limit_is_recorded_and_closed() {
    let (addr, seen) = start().await;
    // 3 MiB of a 4 MiB body: past the fake API's read limit of 2 MiB, and never complete,
    // so only the limit can end the reading.
    let head = format!(
        "POST /ok HTTP/1.1\r\nHost: test\r\nContent-Length: {}\r\n\r\n",
        4 * 1024 * 1024
    );
    let sent = [head.into_bytes(), vec![b'x'; 3 * 1024 * 1024]].concat();
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    // The fake API may close before everything is written, so the write can fail.
    let _ = stream.write_all(&sent).await;

    let arrived = wait_until(&seen, |arrived| !arrived.is_empty()).await;
    assert!(
        matches!(arrived.as_slice(), [Arrival::NotARequest(bytes)]
            if bytes.len() >= 2 * 1024 * 1024 && sent.starts_with(bytes)),
        "expected at least 2 MiB of what was sent, got {} arrivals",
        arrived.len()
    );
    // A close or a reset: either way the connection ends, and nothing is sent back.
    let mut rest = Vec::new();
    let _ = timeout(Duration::from_secs(5), stream.read_to_end(&mut rest))
        .await
        .expect("the fake API closes within 5 s");
    assert!(rest.is_empty(), "the fake API sent {} bytes", rest.len());
}

#[tokio::test]
async fn bytes_after_the_reply_stop_at_the_read_limit() {
    let (addr, seen) = start().await;
    // 3 MiB after the request, past the fake API's read limit of 2 MiB.
    let sent = [get("/ok"), vec![b'x'; 3 * 1024 * 1024]].concat();
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    // The fake API may close before everything is written, so the write can fail.
    let _ = stream.write_all(&sent).await;

    let arrived = wait_until(&seen, |arrived| {
        matches!(arrived.last(), Some(Arrival::NotARequest(_)))
    })
    .await;
    assert_eq!(requests(&arrived).len(), 1);
    assert!(
        matches!(arrived.last(), Some(Arrival::NotARequest(bytes))
            if bytes.len() >= 2 * 1024 * 1024 && bytes.iter().all(|&byte| byte == b'x')),
        "expected at least 2 MiB of x after the request, got {} arrivals",
        arrived.len()
    );
}
