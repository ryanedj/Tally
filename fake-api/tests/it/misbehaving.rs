use crate::common::{arrivals, exchange, get, requests, start};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

#[tokio::test]
async fn switch_answers_with_a_101() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/switch")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: tally\r\nConnection: Upgrade\r\n\r\n"
    );
}

#[tokio::test]
async fn bad_interim_is_a_1xx_with_a_content_length() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/bad-interim")).await;
    assert!(reply.starts_with(b"HTTP/1.1 103 Early Hints\r\nContent-Length: 0\r\n\r\n"));
}

#[tokio::test]
async fn cut_head_stops_in_the_middle_of_a_header() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/cut-head")).await;
    assert_eq!(reply, b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Le");
}

#[tokio::test]
async fn cut_body_is_shorter_than_its_content_length() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/cut-body")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 10\r\n\r\nabcd"
    );
}

/// "path" sends exactly "expected", then nothing more within 300 ms.
async fn assert_stalls_after(path: &str, expected: &[u8]) {
    let (addr, _seen) = start().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(&get(path))
        .await
        .expect("send the request");
    let mut sent = vec![0u8; expected.len()];
    timeout(Duration::from_secs(5), stream.read_exact(&mut sent))
        .await
        .expect("the first bytes arrive within 5 s")
        .expect("read the first bytes");
    assert_eq!(sent, expected);
    let mut buf = [0u8; 16];
    let waited = timeout(Duration::from_millis(300), stream.read(&mut buf)).await;
    assert!(
        waited.is_err(),
        "the fake API should stay silent, but: {waited:?}"
    );
}

#[tokio::test]
async fn stall_body_sends_part_of_its_body_then_nothing() {
    assert_stalls_after(
        "/stall-body",
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 10\r\n\r\nabcd",
    )
    .await;
}

#[tokio::test]
async fn stall_chunked_sends_a_first_chunk_then_nothing() {
    assert_stalls_after(
        "/stall-chunked",
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n",
    )
    .await;
}

#[tokio::test]
async fn reset_body_sends_its_bytes_then_a_reset() {
    let (addr, _seen) = start().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(&get("/reset-body"))
        .await
        .expect("send the request");
    let expected = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nthe body is cut by a reset";
    let mut sent = vec![0u8; expected.len()];
    timeout(Duration::from_secs(5), stream.read_exact(&mut sent))
        .await
        .expect("the bytes arrive within 5 s")
        .expect("read the bytes");
    assert_eq!(sent, expected);
    let mut buf = [0u8; 16];
    let next = timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("the reset arrives within 5 s");
    assert!(next.is_err(), "a reset, not a close: {next:?}");
}

#[tokio::test]
async fn silent_sends_nothing_but_still_records_the_request() {
    let (addr, seen) = start().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(&get("/silent"))
        .await
        .expect("send the request");
    let mut buf = [0u8; 16];
    let waited = timeout(Duration::from_millis(300), stream.read(&mut buf)).await;
    assert!(
        waited.is_err(),
        "the fake API should stay silent, but: {waited:?}"
    );
    assert_eq!(requests(&arrivals(&seen)).len(), 1);
}
