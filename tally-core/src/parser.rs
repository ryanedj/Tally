mod request;
mod response;

pub use request::{Method, RequestHead, parse_host_port, parse_request_head};
pub use response::{BodyEnd, ResponseHead, parse_response_head};

pub(crate) use request::{host_port, normalize_host};

use crate::error::ParseError;

/// The longest head, up to and including its empty line.
const MAX_HEAD: usize = 8 * 1024;
/// The most header lines in a head.
const MAX_HEADERS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed<H> {
    /// The head isn't complete yet: read more, then parse again from byte 0.
    Partial,
    /// A complete head, and its length up to and including its empty line.
    Complete(H, usize),
}

/// A header line: its lowercase name, and its value without the spaces and tabs around it.
type Header = (String, Vec<u8>);

/// Pass 1, shared by requests and responses: checks each complete line as soon as it has
/// arrived, the start line with "start_line", then the header lines, up to the empty line. Empty
/// lines before the start line are skipped if "skips_empty_lines", refused otherwise; skipped,
/// they belong to the head: they count toward 8 KiB and its length.
fn parse_head<S>(
    buf: &[u8],
    skips_empty_lines: bool,
    start_line: impl Fn(&[u8]) -> Result<S, ParseError>,
) -> Result<Parsed<(S, Vec<Header>)>, ParseError> {
    let mut pos = 0;
    let line = loop {
        let Some((line, next)) = next_line(buf, pos)? else {
            return Ok(Parsed::Partial);
        };
        pos = next;
        if !line.is_empty() {
            break line;
        }
        if !skips_empty_lines {
            return Err(ParseError::LeadingEmptyLine);
        }
    };
    let start = start_line(line)?;

    let mut headers: Vec<Header> = Vec::new();
    loop {
        let Some((line, next)) = next_line(buf, pos)? else {
            return Ok(Parsed::Partial);
        };
        pos = next;
        let Some(&first) = line.first() else {
            return Ok(Parsed::Complete((start, headers), pos));
        };
        if first == b' ' || first == b'\t' {
            return Err(if headers.is_empty() {
                ParseError::SpaceBeforeFirstHeader
            } else {
                ParseError::Folding
            });
        }

        let mut parts = line.splitn(2, |&b| b == b':');
        let name = parts.next().unwrap_or_default();
        let Some(value) = parts.next() else {
            return Err(ParseError::HeaderName);
        };
        if name.is_empty() || !name.iter().all(|&b| is_token(b)) {
            return Err(ParseError::HeaderName);
        }
        let value = trim_spaces(value);
        if !value
            .iter()
            .all(|&b| b == b'\t' || (0x20..=0x7e).contains(&b) || b >= 0x80)
        {
            return Err(ParseError::HeaderValue);
        }
        if headers.len() == MAX_HEADERS {
            return Err(ParseError::TooManyHeaders);
        }

        let name: String = name
            .iter()
            .map(|&b| char::from(b.to_ascii_lowercase()))
            .collect();
        if name == "content-length" {
            let is_plain = match value {
                [b'0'] => true,
                [b'1'..=b'9', rest @ ..] => rest.iter().all(u8::is_ascii_digit),
                _ => false,
            };
            let is_repeated = headers.iter().any(|(seen, _)| seen == "content-length");
            if !is_plain || is_repeated {
                return Err(ParseError::ContentLength);
            }
        }
        headers.push((name, value.to_vec()));
    }
}

/// The complete line starting at "pos", without its "\r\n", and where the next one starts;
/// "None" while it hasn't fully arrived.
fn next_line(buf: &[u8], pos: usize) -> Result<Option<(&[u8], usize)>, ParseError> {
    let rest = buf.get(pos..).unwrap_or_default();
    let Some(newline) = rest.iter().position(|&b| b == b'\n') else {
        return if buf.len() >= MAX_HEAD {
            Err(ParseError::TooLarge)
        } else {
            Ok(None)
        };
    };
    let next = pos + newline + 1;
    // Size first, as in the branch above: a line ending past 8 KiB is TooLarge whatever it
    // holds, however the reads split it.
    if next > MAX_HEAD {
        return Err(ParseError::TooLarge);
    }
    match rest
        .get(..newline)
        .and_then(|line| line.strip_suffix(b"\r"))
    {
        Some(line) if !line.contains(&b'\r') => Ok(Some((line, next))),
        _ => Err(ParseError::LineEnding),
    }
}

fn trim_spaces(mut bytes: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = bytes {
        bytes = rest;
    }
    while let [rest @ .., b' ' | b'\t'] = bytes {
        bytes = rest;
    }
    bytes
}

/// A token byte (RFC 9110 §5.6.2).
fn is_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Plain digits as a number, "None" when they aren't all digits, or too big for u64.
fn parse_digits(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0u64, |number, &b| {
        if b.is_ascii_digit() {
            number.checked_mul(10)?.checked_add(u64::from(b - b'0'))
        } else {
            None
        }
    })
}
