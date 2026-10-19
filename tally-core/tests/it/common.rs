use std::path::Path;
use tally_core::{Secrets, Settings, check_settings, parse_secrets, parse_settings};

// One test per row, like httparse's "req!" and "res!". A good row checks that the head ends
// where "$bytes" ends (any "then" bytes come after it), that httparse agrees on its length,
// and that every shorter piece of it is partial. A refused row checks for its exact error.
macro_rules! row {
    ($parse:path, $httparse:ident, $name:ident, $bytes:expr, Partial) => {
        #[test]
        fn $name() {
            assert_eq!($parse($bytes), Ok(tally_core::Parsed::Partial));
        }
    };
    ($parse:path, $httparse:ident, $name:ident, $bytes:expr, Err($error:ident)) => {
        #[test]
        fn $name() {
            assert_eq!($parse($bytes), Err(tally_core::ParseError::$error));
        }
    };
    ($parse:path, $httparse:ident, $name:ident, $bytes:expr, then $rest:expr,
        |$head:pat_param| $checks:block) => {
        #[test]
        fn $name() {
            let head_bytes: &[u8] = $bytes;
            let rest: &[u8] = $rest;
            let bytes = [head_bytes, rest].concat();
            let $head = match $parse(&bytes) {
                Ok(tally_core::Parsed::Complete(head, length)) => {
                    assert_eq!(length, head_bytes.len(), "head length");
                    head
                }
                other => panic!("expected a complete head, got {other:?}"),
            };
            let mut headers = [httparse::EMPTY_HEADER; 128];
            assert_eq!(
                httparse::$httparse::new(&mut headers).parse(&bytes),
                Ok(httparse::Status::Complete(head_bytes.len())),
                "httparse must accept the head too, with the same length"
            );
            for end in 0..head_bytes.len() {
                assert_eq!(
                    $parse(&bytes[..end]),
                    Ok(tally_core::Parsed::Partial),
                    "the first {end} bytes"
                );
            }
            $checks
        }
    };
    ($parse:path, $httparse:ident, $name:ident, $bytes:expr,
        |$head:pat_param| $checks:block) => {
        $crate::common::row! { $parse, $httparse, $name, $bytes, then b"", |$head| $checks }
    };
}

/// A row for a request head: see "row!".
macro_rules! req {
    ($($row:tt)*) => {
        $crate::common::row! { tally_core::parse_request_head, Request, $($row)* }
    };
}

/// A row for a response head: see "row!".
macro_rules! res {
    ($($row:tt)*) => {
        $crate::common::row! { tally_core::parse_response_head, Response, $($row)* }
    };
}

pub(crate) use req;
pub(crate) use res;
pub(crate) use row;

/// Settings and secrets from TOML text, through every startup rule.
pub fn checked(settings: &str, secrets: &str) -> (Settings, Secrets) {
    let raw = parse_settings(settings.as_bytes(), Path::new("settings.toml"))
        .expect("the settings parse");
    let secrets =
        parse_secrets(secrets.as_bytes(), Path::new("secrets.toml")).expect("the secrets parse");
    let settings = check_settings(raw, &secrets).expect("the settings pass every startup rule");
    (settings, secrets)
}

/// A head with "start_line" and "count" headers.
pub fn head_with_headers(start_line: &[u8], count: usize) -> Vec<u8> {
    let mut head = [start_line, b"\r\n".as_slice()].concat();
    for i in 0..count {
        head.extend_from_slice(format!("x-{i}: v\r\n").as_bytes());
    }
    head.extend_from_slice(b"\r\n");
    head
}

/// A complete head with "start_line", of exactly "length" bytes, padded by one header.
pub fn head_of_length(start_line: &[u8], length: usize) -> Vec<u8> {
    let mut head = [start_line, b"\r\nx-pad: ".as_slice()].concat();
    head.resize(length - 4, b'a');
    head.extend_from_slice(b"\r\n\r\n");
    head
}
