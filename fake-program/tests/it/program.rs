use fake_program::{Placeholders, run};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket};

const WEATHER: &str = "TALLY_0123456789abcdef0123456789abcdef";
const MAPS: &str = "TALLY_fedcba9876543210fedcba9876543210";

/// Runs the program binary with "http_proxy" set to "proxy", and WEATHER_KEY to "weather" or left
/// out.
fn run_program(proxy: &str, weather: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fake-program"));
    command.env("http_proxy", proxy).env("MAPS_KEY", MAPS);
    match weather {
        Some(weather) => command.env("WEATHER_KEY", weather),
        None => command.env_remove("WEATHER_KEY"),
    };
    command.output().expect("the program binary runs")
}

#[test]
fn a_missing_placeholder_is_named() {
    let output = run_program("http://127.0.0.1:9", None);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "WEATHER_KEY isn't set\n"
    );
}

// The proxy's address is never looked up, as in the proxy's settings.
#[test]
fn a_proxy_named_instead_of_an_ip_address_is_refused() {
    let output = run_program("http://proxy.local:8080", Some(WEATHER));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("http_proxy isn't an IP address"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// A port bound but never listened on refuses connections (RFC 9293 §3.10.7.1), and no other test
// can take it while "reserved" holds it. Windows retries a refused loopback connect for about 2 s.
#[test]
fn a_proxy_that_cant_be_reached() {
    let reserved = TcpSocket::new_v4().expect("a socket");
    reserved
        .bind("127.0.0.1:0".parse().expect("an address"))
        .expect("bind a free port");
    let port = reserved
        .local_addr()
        .expect("the socket has an address")
        .port();
    let output = run_program(&format!("http://127.0.0.1:{port}"), Some(WEATHER));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .starts_with(&format!("the proxy at 127.0.0.1:{port} can't be reached")),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// A stand-in for the proxy records each connection's request, answers "/redirect" with a 302 to
// another host, and everything else with a 200.
#[tokio::test]
async fn each_request_has_its_connection_and_the_redirect_keeps_the_key() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a free port");
    let address = listener.local_addr().expect("the listener has an address");
    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&received);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = Vec::new();
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read_buf(&mut request).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let reply: &[u8] = if request.starts_with("GET http://api.weather.local/redirect ") {
                b"HTTP/1.1 302 Found\r\nlocation: http://elsewhere.local/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            } else {
                b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            };
            sink.lock().expect("the lock is not poisoned").push(request);
            let _ = stream.write_all(reply).await;
            let _ = stream.shutdown().await;
        }
    });

    let keys = Placeholders {
        weather: WEATHER.to_string(),
        maps: MAPS.to_string(),
    };
    run(address, &keys).await.expect("the stand-in is reached");

    let received = received.lock().expect("the lock is not poisoned").clone();
    assert_eq!(
        received.len(),
        14,
        "13 requests and the redirect's second hop"
    );
    assert!(
        received[7].starts_with("GET http://api.weather.local/redirect HTTP/1.1\r\n"),
        "{}",
        received[7]
    );
    assert!(
        received[8].starts_with(&format!(
            "GET http://elsewhere.local/ HTTP/1.1\r\nHost: elsewhere.local\r\nAuthorization: Bearer {WEATHER}\r\n"
        )),
        "{}",
        received[8]
    );
}
