#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    //the request's exact bytes
    pub raw: Vec<u8>,
    pub head: Head,
    pub body: Vec<u8>,
}

pub(crate) enum Parsed {
    /// A whole request (head and body), and how many bytes it takes.
    Complete(Received, usize),
    /// The head or the body hasn't completely arrived.
    Incomplete,
    /// httparse refused it.
    Malformed,
}

pub(crate) fn parse_request(buf: &[u8]) -> Parsed {
    // The proxy forwards up to 101 headers: the program's 100 (the head_too_large limit),
    // with its own Host, plus Connection: close. Past this, httparse fails with TooManyHeaders.
    let mut headers = [httparse::EMPTY_HEADER; 128];
    let mut req = httparse::Request::new(&mut headers);
    let head_len = match req.parse(buf) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return Parsed::Incomplete,
        Err(_) => return Parsed::Malformed,
    };
    let method = req
        .method
        .expect("httparse gives a method after Complete")
        .to_string();
    let target = req
        .path
        .expect("httparse gives a target after Complete")
        .to_string();
    let headers: Vec<(String, Vec<u8>)> = req
        .headers
        .iter()
        .map(|h| (h.name.to_string(), h.value.to_vec()))
        .collect();
    // For tests: RFC 9112 §6.3 requires a 400 for an invalid Content-Length. Here it is read
    // loosely (the first one, "+5" or "05", an unreadable value as 0), and Transfer-Encoding
    // is ignored, so a chunked body is recorded as bytes after the request. Every byte is
    // recorded either way, so a proxy bug still shows.
    let body_len = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| std::str::from_utf8(value).ok()) // bytes -> text, or None
        .and_then(|text| text.trim().parse::<usize>().ok()) // text -> number, or None
        .unwrap_or(0);
    let Some(end) = head_len.checked_add(body_len) else {
        return Parsed::Malformed;
    };
    let (Some(raw), Some(body)) = (buf.get(..end), buf.get(head_len..end)) else {
        return Parsed::Incomplete;
    };
    let head = Head {
        method,
        target,
        headers,
    };
    Parsed::Complete(
        Received {
            raw: raw.to_vec(),
            head,
            body: body.to_vec(),
        },
        end,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(bytes: &[u8]) -> (Received, usize) {
        match parse_request(bytes) {
            Parsed::Complete(received, used) => (received, used),
            Parsed::Incomplete => panic!("expected a complete request, got an incomplete one"),
            Parsed::Malformed => panic!("expected a complete request, got a malformed one"),
        }
    }

    #[test]
    fn a_request_without_a_body() {
        let bytes = b"GET /ok?x=1 HTTP/1.1\r\nHost: test\r\n\r\n";
        let (received, used) = complete(bytes);
        assert_eq!(used, bytes.len());
        assert_eq!(received.head.method, "GET");
        assert_eq!(received.head.target, "/ok?x=1");
        assert_eq!(
            received.head.headers,
            vec![("Host".to_string(), b"test".to_vec())]
        );
        assert_eq!(received.raw, bytes);
        assert!(received.body.is_empty());
    }

    #[test]
    fn the_body_is_read_by_content_length_in_any_case() {
        let bytes = b"POST /ok HTTP/1.1\r\ncontent-LENGTH: 5\r\n\r\nhello";
        let (received, used) = complete(bytes);
        assert_eq!(used, bytes.len());
        assert_eq!(received.body, b"hello");
    }

    #[test]
    fn only_the_first_request_is_taken_from_glued_requests() {
        let first = b"GET /one HTTP/1.1\r\nHost: test\r\n\r\n";
        let glued = [first.as_slice(), b"GET /two HTTP/1.1\r\n\r\n"].concat();
        let (received, used) = complete(&glued);
        assert_eq!(received.head.target, "/one");
        assert_eq!(used, first.len());
    }

    #[test]
    fn a_head_that_is_not_finished_is_incomplete() {
        assert!(matches!(
            parse_request(b"GET /ok HTTP/1.1\r\nHost: te"),
            Parsed::Incomplete
        ));
    }

    #[test]
    fn a_body_that_is_not_finished_is_incomplete() {
        assert!(matches!(
            parse_request(b"POST /ok HTTP/1.1\r\nContent-Length: 5\r\n\r\nhel"),
            Parsed::Incomplete
        ));
    }

    #[test]
    fn a_request_line_with_too_many_spaces_is_malformed() {
        assert!(matches!(
            parse_request(b"GET /a b c HTTP/1.1\r\n\r\n"),
            Parsed::Malformed
        ));
    }

    #[test]
    fn a_content_length_too_big_to_add_is_malformed() {
        let request = format!(
            "POST /ok HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            usize::MAX
        );
        assert!(matches!(
            parse_request(request.as_bytes()),
            Parsed::Malformed
        ));
    }

    #[test]
    fn the_most_headers_the_proxy_forwards_fit() {
        let mut request = b"GET /ok HTTP/1.1\r\n".to_vec();
        for i in 0..101 {
            request.extend_from_slice(format!("x-header-{i}: value\r\n").as_bytes());
        }
        request.extend_from_slice(b"\r\n");
        let (received, _) = complete(&request);
        assert_eq!(received.head.headers.len(), 101);
    }
}
