use crate::Received;

/// How the fake API answers a request.
pub(crate) enum Response {
    /// These exact bytes, then the staged close.
    Bytes(Vec<u8>),
    /// Nothing: wait until the other side closes.
    Silent,
    /// A 200 with no length, then a body that never ends, keep sending until the other side
    /// closes, so whatever buffers lie between the fake API and the program fills up.
    Endless,
    /// The first bytes, a 500 ms pause, then the rest and the staged close.
    Paused { first: Vec<u8>, then: Vec<u8> },
}

/// What the fake API answers for a request
///
/// Every final response says "Connection: close", as a server that closes should
/// (RFC 9112 §9.6), so a test at L5 can check that the proxy doesn't pass it on to the program.
pub(crate) fn route(received: &Received) -> Response {
    let target = &received.head.target;
    let path = target
        .split_once('?')
        .map_or(target.as_str(), |(path, _)| path);
    let bytes: &[u8] = match path {
        // A normal response with a known length.
        "/ok" => b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok",
        // Chunked: the body ends with the zero-length chunk, then the connection closes.
        "/chunked" => {
            b"HTTP/1.1 200 OK\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"
        }
        // No length at all: the body ends when the connection closes.
        "/no-length" => {
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nthe body ends when the connection closes"
        }
        // For tests:protocol 4's "Program stops reading for 30 s".
        "/endless" => return Response::Endless,
        "/no-content" => b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n",
        // An interim 103, then the final response after a pause: a server sends a 103
        // "while waiting for the final response" (RFC 8297), and Tally must relay it as it arrives.
        // For tests: the 500 ms pause stands in for the server's work, so a test can see the
        // 103 arrive alone.
        "/early-hints" => {
            return Response::Paused {
                first: b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\n".to_vec(),
                then: b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".to_vec(),
            };
        }
        // The request's head sent back as the body.
        // For tests: We send the head back exactly, real key included, to show a test what the proxy forwarded.
        "/echo-headers" => {
            let head_len = received.raw.len().saturating_sub(received.body.len());
            let head = received.raw.get(..head_len).unwrap_or_default();
            let mut response =
                format!("HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", head.len()).into_bytes();
            response.extend_from_slice(head);
            return Response::Bytes(response);
        }
        // A redirect to another host.
        "/redirect" => {
            b"HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: http://elsewhere.local/\r\nContent-Length: 0\r\n\r\n"
        }
        // For tests, misbehaving on purpose: each breaks HTTP in one way the proxy must catch
        // A 101 to a request that asked for no upgrade, which RFC 9110 §7.8 forbids (bad_response)
        "/switch" => {
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: tally\r\nConnection: Upgrade\r\n\r\n"
        }
        // A 1xx with Content-Length, which RFC 9110 §8.6 forbids (bad_response)
        "/bad-interim" => {
            b"HTTP/1.1 103 Early Hints\r\nContent-Length: 0\r\n\r\nHTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok"
        }
        // a head cut off by the close (bad_response)
        "/cut-head" => b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Le",
        // nothing before the head (response_timeout)
        "/silent" => return Response::Silent,
        // a body shorter than announced (response_incomplete)
        "/cut-body" => b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 10\r\n\r\nabcd",
        _ => b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
    };
    Response::Bytes(bytes.to_vec())
}
