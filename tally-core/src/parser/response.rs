use super::{Parsed, is_token, parse_digits, parse_head, trim_spaces};
use crate::error::ParseError;

/// Where a response's body ends (RFC 9112 §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyEnd {
    /// "1xx", "204" and "304": at the end of the head.
    NoBody,
    /// After exactly this many bytes ("Content-Length").
    Length(u64),
    /// Chunked, or no length: when the Server closes.
    UntilServerCloses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseHead {
    pub status: u16,
    /// May be empty.
    pub reason: Vec<u8>,
    /// In order, with lowercase names, and values trimmed of spaces and tabs.
    pub headers: Vec<(String, Vec<u8>)>,
    pub body_end: BodyEnd,
    /// The lowercase names the "Connection" header lists, for the rewrite to drop.
    pub connection_names: Vec<String>,
}

/// Reads a response head from the bytes read so far.
pub fn parse_response_head(buf: &[u8]) -> Result<Parsed<ResponseHead>, ParseError> {
    // Pass 1: the status line, then the header lines
    let parsed = parse_head(buf, |line| {
        let mut parts = line.splitn(2, |&b| b == b' ');
        match parts.next().unwrap_or_default() {
            b"HTTP/1.1" => {}
            b"HTTP/1.0" => return Err(ParseError::Http10),
            _ => return Err(ParseError::Version),
        }
        let Some([hundreds, tens, ones, b' ', reason @ ..]) = parts.next() else {
            return Err(ParseError::StatusLine);
        };
        let Some(status) = parse_digits(&[*hundreds, *tens, *ones]) else {
            return Err(ParseError::StatusLine);
        };
        let status = u16::try_from(status)
            .ok()
            .filter(|status| (100..=599).contains(status))
            .ok_or(ParseError::StatusCode)?;
        if !reason
            .iter()
            .all(|&b| b == b'\t' || (0x20..=0x7e).contains(&b) || b >= 0x80)
        {
            return Err(ParseError::StatusLine);
        }
        Ok((status, reason.to_vec()))
    })?;
    let Parsed::Complete(((status, reason), headers), length) = parsed else {
        return Ok(Parsed::Partial);
    };

    // Pass 2, in byte order: the status line, then each header
    if status == 101 {
        return Err(ParseError::SwitchingProtocols);
    }
    let ends_with_its_head = matches!(status, 100..=199 | 204);
    let mut content_length = None;
    let mut has_transfer_encoding = false;
    let mut connection_names = Vec::new();
    for (name, value) in &headers {
        match name.as_str() {
            "content-length" | "transfer-encoding" if ends_with_its_head => {
                return Err(ParseError::LengthOnNoBody);
            }
            "content-length" if has_transfer_encoding => {
                return Err(ParseError::LengthAndEncoding);
            }
            "content-length" => {
                content_length = Some(parse_digits(value).ok_or(ParseError::ContentLength)?);
            }
            "transfer-encoding" if content_length.is_some() => {
                return Err(ParseError::LengthAndEncoding);
            }
            "transfer-encoding" => has_transfer_encoding = true,
            "connection" => {
                for element in value.split(|&b| b == b',') {
                    let element = trim_spaces(element);
                    if element.is_empty() {
                        continue;
                    }
                    if !element.iter().all(|&b| is_token(b)) {
                        return Err(ParseError::ConnectionList);
                    }
                    let element: String = element
                        .iter()
                        .map(|&b| char::from(b.to_ascii_lowercase()))
                        .collect();
                    if element == "content-length" || element == "transfer-encoding" {
                        return Err(ParseError::ConnectionNamesFraming);
                    }
                    connection_names.push(element);
                }
            }
            _ => {}
        }
    }

    // RFC 9112 §6.3; Content-Length and Transfer-Encoding together were refused above
    let body_end = match (status, content_length) {
        (100..=199 | 204 | 304, _) => BodyEnd::NoBody,
        (_, Some(length)) => BodyEnd::Length(length),
        (_, None) => BodyEnd::UntilServerCloses,
    };
    let head = ResponseHead {
        status,
        reason,
        headers,
        body_end,
        connection_names,
    };
    Ok(Parsed::Complete(head, length))
}
