use std::io;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// The placeholders the program holds instead of its API keys.
pub struct Placeholders {
    pub weather: String,
    pub maps: String,
}

/// How a request is sent.
enum Sending {
    /// A GET built from its URL and header lines. A redirect is followed once, headers kept, as a
    /// compromised program would.
    Get { url: String, headers: String },
    /// These exact bytes.
    Raw(Vec<u8>),
    /// These exact bytes, then the program stops sending, part-way through the body.
    CutShort(Vec<u8>),
}

/// Sends each request of the demo through the proxy at "proxy", each on its own connection, and
/// prints what it sent and what came back.
pub async fn run(proxy: SocketAddr, keys: &Placeholders) -> io::Result<()> {
    let weather = format!("Authorization: Bearer {}\r\n", keys.weather);
    let maps = format!("X-Api-Key: {}\r\n", keys.maps);
    let requests = [
        (
            "the weather call, with its placeholder in Authorization",
            Sending::Get {
                url: "http://api.weather.local/ok".to_string(),
                headers: weather.clone(),
            },
        ),
        (
            "no key, to a listed host",
            Sending::Get {
                url: "http://api.weather.local/ok".to_string(),
                headers: String::new(),
            },
        ),
        (
            "the placeholder in the URL, outside its bound header",
            Sending::Get {
                url: format!("http://api.weather.local/echo-headers?key={}", keys.weather),
                headers: String::new(),
            },
        ),
        (
            "a Host naming another host than the URL",
            Sending::Raw(
                b"GET http://api.weather.local/echo-headers HTTP/1.1\r\nHost: maps.local:8080\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            ),
        ),
        (
            "no key, to a host that isn't listed",
            Sending::Get {
                url: "http://elsewhere.local/".to_string(),
                headers: String::new(),
            },
        ),
        (
            "the weather key, toward the maps host",
            Sending::Get {
                url: "http://maps.local:8080/ok".to_string(),
                headers: weather.clone(),
            },
        ),
        (
            "two keys, the maps key not allowed for the weather host",
            Sending::Get {
                url: "http://api.weather.local/ok".to_string(),
                headers: format!("{weather}{maps}"),
            },
        ),
        (
            "the weather call to a redirect toward another host, followed with the key",
            Sending::Get {
                url: "http://api.weather.local/redirect".to_string(),
                headers: weather.clone(),
            },
        ),
        (
            "Connection naming Authorization, so the key's header would be dropped after the decision",
            Sending::Raw(
                format!("GET http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\n{weather}Connection: authorization\r\n\r\n")
                    .into_bytes(),
            ),
        ),
        (
            "no Host",
            Sending::Raw(
                b"GET http://api.weather.local/ok HTTP/1.1\r\nConnection: close\r\n\r\n".to_vec(),
            ),
        ),
        (
            "a head over 8 KiB",
            Sending::Raw(
                format!(
                    "GET http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nX-Padding: {}\r\nConnection: close\r\n\r\n",
                    "x".repeat(9000)
                )
                .into_bytes(),
            ),
        ),
        (
            "a Content-Length over 1 MiB",
            Sending::Raw(
                b"POST http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\nContent-Length: 2097152\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            ),
        ),
        (
            "the weather call with a body, closed part-way through it",
            Sending::CutShort(
                format!("POST http://api.weather.local/ok HTTP/1.1\r\nHost: api.weather.local\r\n{weather}Content-Length: 10\r\nConnection: close\r\n\r\nabcd")
                    .into_bytes(),
            ),
        ),
    ];

    for (number, (label, sending)) in (1..).zip(requests) {
        println!("--- {number}. {label} ---");
        match sending {
            Sending::Get { url, headers } => {
                let reply = exchange(proxy, &get(&url, &headers), false).await?;
                let mut fields = [httparse::EMPTY_HEADER; 64];
                let mut response = httparse::Response::new(&mut fields);
                let is_redirect =
                    matches!(response.parse(&reply), Ok(httparse::Status::Complete(_)))
                        && response.code.is_some_and(|code| (300..400).contains(&code));
                let location = response
                    .headers
                    .iter()
                    .find(|field| field.name.eq_ignore_ascii_case("location"))
                    .map(|field| String::from_utf8_lossy(field.value).into_owned());
                if is_redirect && let Some(location) = location {
                    if location.starts_with("http://") {
                        println!("following the redirect to {location}, with the same headers");
                        exchange(proxy, &get(&location, &headers), false).await?;
                    } else {
                        println!("not following the redirect to {location}: not an http:// URL");
                    }
                }
            }
            Sending::Raw(bytes) => {
                exchange(proxy, &bytes, false).await?;
            }
            Sending::CutShort(bytes) => {
                exchange(proxy, &bytes, true).await?;
            }
        }
    }
    Ok(())
}

/// A GET for "url" (`http://host[:port]/…`) with its host as Host, then "headers" (each line
/// ending in "\r\n") and "Connection: close", so the proxy closes once it has replied.
fn get(url: &str, headers: &str) -> Vec<u8> {
    let authority = url.strip_prefix("http://").unwrap_or(url);
    let host = authority
        .split_once('/')
        .map_or(authority, |(host, _)| host);
    format!("GET {url} HTTP/1.1\r\nHost: {host}\r\n{headers}Connection: close\r\n\r\n").into_bytes()
}

/// Sends "request" on a new connection to the proxy, then closes the sending side if
/// "stop_sending". Prints the request and what came back until the proxy closes, or for 10 s at
/// most, and returns what came back. An error only when the proxy can't be reached.
async fn exchange(proxy: SocketAddr, request: &[u8], stop_sending: bool) -> io::Result<Vec<u8>> {
    print_lines("> ", request);
    let mut stream = TcpStream::connect(proxy).await?;
    if let Err(error) = stream.write_all(request).await {
        println!("the proxy stopped taking the request: {error}");
    }
    if stop_sending {
        println!("(the program stops sending)");
        if let Err(error) = stream.shutdown().await {
            println!("the sending side can't be closed: {error}");
        }
    }

    let mut reply = Vec::new();
    let ended = timeout(Duration::from_secs(10), async {
        loop {
            match stream.read_buf(&mut reply).await {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error) => return Err(error),
            }
        }
    })
    .await;
    if reply.is_empty() {
        println!("< (no reply)");
    } else {
        print_lines("< ", &reply);
    }
    match ended {
        Ok(Ok(())) => println!("(closed by the proxy)"),
        Ok(Err(error)) => println!("(the connection broke: {error})"),
        Err(_) => println!("(still open after 10 s)"),
    }
    Ok(reply)
}

/// Prints each line of "bytes" after "prefix", the bytes that aren't printable escaped and long
/// lines cut.
fn print_lines(prefix: &str, bytes: &[u8]) {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    for line in bytes.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let shown = line.get(..120).unwrap_or(line);
        if shown.len() < line.len() {
            println!("{prefix}{}… ({} bytes)", shown.escape_ascii(), line.len());
        } else {
            println!("{prefix}{}", line.escape_ascii());
        }
    }
}
