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
