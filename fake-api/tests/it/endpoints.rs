use crate::common::{exchange, get, start, wait_until};
use fake_api::Arrival;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

#[tokio::test]
async fn ok_has_a_known_length() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/ok")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok"
    );
}

#[tokio::test]
async fn the_query_string_does_not_change_the_endpoint() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/ok?x=1")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok"
    );
}

#[tokio::test]
async fn chunked_ends_with_the_zero_chunk() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/chunked")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"
    );
}

#[tokio::test]
async fn no_length_has_neither_content_length_nor_chunks() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/no-length")).await;
    assert!(reply.starts_with(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n"));
    assert!(reply.ends_with(b"the body ends when the connection closes"));
}

#[tokio::test]
async fn endless_keeps_sending_until_the_client_closes() {
    let (addr, seen) = start().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    // The stray bytes are reported only once the sending loop has ended,
    // so their report shows that it stops when the client closes.
    stream
        .write_all(&[get("/endless"), b"STRAY".to_vec()].concat())
        .await
        .expect("send the request and the stray bytes");

    let mut head = [0u8; 38];
    timeout(Duration::from_secs(5), stream.read_exact(&mut head))
        .await
        .expect("the head arrives within 5 s")
        .expect("read the head");
    assert_eq!(&head, b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");

    // More than a few buffers' worth, to show it doesn't stop by itself.
    let mut body = vec![0u8; 1024 * 1024];
    timeout(Duration::from_secs(5), stream.read_exact(&mut body))
        .await
        .expect("1 MiB of body arrives within 5 s")
        .expect("read the body");
    assert!(body.iter().all(|&byte| byte == b'x'));

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
async fn no_content_has_no_body() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/no-content")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
    );
}

#[tokio::test]
async fn early_hints_come_before_the_final_response() {
    let (addr, _seen) = start().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(&get("/early-hints"))
        .await
        .expect("send the request");

    let interim = b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\n";
    let mut first = vec![0u8; interim.len()];
    timeout(Duration::from_secs(5), stream.read_exact(&mut first))
        .await
        .expect("the 103 arrives within 5 s")
        .expect("read the 103");
    assert_eq!(first, interim);

    // The final response is still being "prepared": nothing else arrives yet.
    let mut one_byte = [0u8; 1];
    let early = timeout(Duration::from_millis(200), stream.read(&mut one_byte)).await;
    assert!(early.is_err(), "the 200 came with the 103: {early:?}");

    let mut rest = Vec::new();
    timeout(Duration::from_secs(5), stream.read_to_end(&mut rest))
        .await
        .expect("the fake API closes within 5 s")
        .expect("read the final response");
    assert_eq!(
        rest,
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok"
    );
}

#[tokio::test]
async fn echo_headers_sends_the_head_back_as_it_arrived() {
    let (addr, _seen) = start().await;
    let request = b"GET /echo-headers HTTP/1.1\r\nHost: test\r\nAuthorization: Bearer abc\r\n\r\n";
    let reply = exchange(addr, request).await;
    let expected_start = format!(
        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        request.len()
    );
    assert!(reply.starts_with(expected_start.as_bytes()));
    assert!(reply.ends_with(request));
}

#[tokio::test]
async fn redirect_points_to_another_host() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/redirect")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: http://elsewhere.local/\r\nContent-Length: 0\r\n\r\n"
    );
}

#[tokio::test]
async fn extra_after_body_sends_a_second_response_past_its_length() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/extra-after-body")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nokHTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 8\r\n\r\nsmuggled"
    );
}

#[tokio::test]
async fn an_unknown_path_is_a_404() {
    let (addr, _seen) = start().await;
    let reply = exchange(addr, &get("/nothing-here")).await;
    assert_eq!(
        reply,
        b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
}
