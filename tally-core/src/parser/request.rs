use super::{Parsed, is_token, parse_digits, parse_head};
use crate::error::ParseError;

/// The largest request body.
const MAX_BODY: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    pub method: Method,
    /// Lowercase.
    pub host: String,
    /// 80 when the URL has no port, or an empty one.
    pub port: u16,
    /// "/" when the URL's path is empty.
    pub path_and_query: String,
    /// In order, with lowercase names, and values trimmed of spaces and tabs.
    pub headers: Vec<(String, Vec<u8>)>,
    /// 0 when there is no "Content-Length".
    pub content_length: u64,
    /// The program sent "Connection: close".
    pub wants_close: bool,
}

/// Reads a request head from the bytes read so far.
pub fn parse_request_head(buf: &[u8]) -> Result<Parsed<RequestHead>, ParseError> {
    // Pass 1: the request line, after any empty lines (RFC 9112 §2.2), then the header lines
    let parsed = parse_head(buf, true, |line| {
        let mut parts = line.split(|&b| b == b' ');
        let (Some(method), Some(target), Some(version), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ParseError::RequestLine);
        };
        if method.is_empty() || target.is_empty() || version.is_empty() {
            return Err(ParseError::RequestLine);
        }
        if !method.iter().all(|&b| is_token(b)) {
            return Err(ParseError::RequestLine);
        }
        let method = match method {
            b"GET" => Some(Method::Get),
            b"POST" => Some(Method::Post),
            b"PUT" => Some(Method::Put),
            b"PATCH" => Some(Method::Patch),
            b"DELETE" => Some(Method::Delete),
            _ => None,
        };

        // scheme://host[:port], then an optional path and query
        let Some(scheme_end) = target.windows(3).position(|w| w == b"://") else {
            return Err(ParseError::Target);
        };
        let (scheme, after) = target.split_at_checked(scheme_end).unwrap_or((target, &[]));
        let scheme_is_valid = matches!(scheme, [first, rest @ ..]
            if first.is_ascii_alphabetic()
                && rest.iter().all(|&b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)));
        if !scheme_is_valid {
            return Err(ParseError::Target);
        }
        let after = after.get(3..).unwrap_or_default();
        let authority_end = after
            .iter()
            .position(|&b| b == b'/' || b == b'?')
            .unwrap_or(after.len());
        let (authority, path) = after
            .split_at_checked(authority_end)
            .unwrap_or((after, &[]));
        let (host, port) = parse_host_port(authority)?;

        // RFC 3986 §3.3 and §3.4: pchar, "/" and "?", with "%" followed by two hex digits
        let mut rest = path;
        while let [b, tail @ ..] = rest {
            rest = match (b, tail) {
                (b'%', [high, low, tail @ ..])
                    if high.is_ascii_hexdigit() && low.is_ascii_hexdigit() =>
                {
                    tail
                }
                (b'%', _) => return Err(ParseError::PathOrQuery),
                (b, _) if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/?".contains(b) => tail,
                _ => return Err(ParseError::PathOrQuery),
            };
        }
        let mut path_and_query: String = path.iter().map(|&b| char::from(b)).collect();
        if !path_and_query.starts_with('/') {
            path_and_query.insert(0, '/');
        }

        let is_http_1_0 = match version {
            b"HTTP/1.1" => false,
            b"HTTP/1.0" => true,
            _ => return Err(ParseError::Version),
        };
        let is_http = scheme.eq_ignore_ascii_case(b"http");
        Ok((method, is_http, host, port, path_and_query, is_http_1_0))
    })?;
    let Parsed::Complete(
        ((method, is_http, host, port, path_and_query, is_http_1_0), headers),
        length,
    ) = parsed
    else {
        return Ok(Parsed::Partial);
    };

    // Pass 2, in byte order: the request line, then each header
    let Some(method) = method else {
        return Err(ParseError::Method);
    };
    if !is_http {
        return Err(ParseError::Scheme);
    }
    if is_http_1_0 {
        return Err(ParseError::Http10);
    }
    let mut content_length = 0;
    let mut wants_close = None;
    for (name, value) in &headers {
        match name.as_str() {
            "content-length" => {
                content_length = parse_digits(value)
                    .filter(|&length| length <= MAX_BODY)
                    .ok_or(ParseError::BodyTooLarge)?;
            }
            "transfer-encoding" => return Err(ParseError::TransferEncoding),
            "upgrade" => return Err(ParseError::Upgrade),
            "expect" => return Err(ParseError::Expect),
            "connection" if wants_close.is_none() => {
                wants_close = Some(match value.to_ascii_lowercase().as_slice() {
                    b"close" => true,
                    b"keep-alive" => false,
                    _ => return Err(ParseError::Connection),
                });
            }
            "connection" => return Err(ParseError::Connection),
            _ => {}
        }
    }

    let head = RequestHead {
        method,
        host,
        port,
        path_and_query,
        headers,
        content_length,
        wants_close: wants_close == Some(true),
    };
    Ok(Parsed::Complete(head, length))
}

/// Reads "host[:port]", from the URL or a "Host" value: the host in lowercase,
/// the port 80 when there is none or an empty one.
pub fn parse_host_port(bytes: &[u8]) -> Result<(String, u16), ParseError> {
    let mut parts = bytes.splitn(2, |&b| b == b':');
    let host = parts.next().unwrap_or_default();
    let host_is_valid = !host.is_empty()
        && !host.ends_with(b".")
        && host
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    if !host_is_valid {
        return Err(ParseError::Host);
    }
    let port = match parts.next() {
        None | Some([]) => 80,
        Some(port) => parse_digits(port)
            .and_then(|port| u16::try_from(port).ok())
            .filter(|&port| port != 0)
            .ok_or(ParseError::Port)?,
    };
    let host = host
        .iter()
        .map(|&b| char::from(b.to_ascii_lowercase()))
        .collect();
    Ok((host, port))
}

/// A host and its port as "host:port", the form the settings, the decisions and the log use.
pub(crate) fn host_port(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// A host as lowercase "host:port", by "parse_host_port", "None" if it refuses it.
pub(crate) fn normalize_host(bytes: &[u8]) -> Option<String> {
    parse_host_port(bytes)
        .ok()
        .map(|(host, port)| host_port(&host, port))
}
