use tally_core::{ParseError, RefuseReason};

// One test per variant, against DESIGN's reasons table.
macro_rules! reason {
    ($name:ident, $error:ident, $reason:ident) => {
        #[test]
        fn $name() {
            assert_eq!(
                RefuseReason::from(ParseError::$error),
                RefuseReason::$reason
            );
        }
    };
}

// malformed_request
reason! { line_ending, LineEnding, MalformedRequest }
reason! { leading_empty_line, LeadingEmptyLine, MalformedRequest }
reason! { request_line, RequestLine, MalformedRequest }
reason! { version, Version, MalformedRequest }
reason! { space_before_first_header, SpaceBeforeFirstHeader, MalformedRequest }
reason! { header_name, HeaderName, MalformedRequest }
reason! { header_value, HeaderValue, MalformedRequest }
reason! { folding, Folding, MalformedRequest }
reason! { content_length, ContentLength, MalformedRequest }
reason! { target, Target, MalformedRequest }
reason! { host, Host, MalformedRequest }
reason! { port, Port, MalformedRequest }
reason! { path_or_query, PathOrQuery, MalformedRequest }

// head_too_large
reason! { too_large, TooLarge, HeadTooLarge }
reason! { too_many_headers, TooManyHeaders, HeadTooLarge }

// unsupported_request
reason! { http_10, Http10, UnsupportedRequest }
reason! { method, Method, UnsupportedRequest }
reason! { scheme, Scheme, UnsupportedRequest }
reason! { transfer_encoding, TransferEncoding, UnsupportedRequest }
reason! { upgrade, Upgrade, UnsupportedRequest }
reason! { expect, Expect, UnsupportedRequest }
reason! { connection, Connection, UnsupportedRequest }

// body_too_large
reason! { body_too_large, BodyTooLarge, BodyTooLarge }

// Response-only variants: a request never gets them, and would be malformed if it did.
reason! { status_line, StatusLine, MalformedRequest }
reason! { status_code, StatusCode, MalformedRequest }
reason! { switching_protocols, SwitchingProtocols, MalformedRequest }
reason! { length_on_no_body, LengthOnNoBody, MalformedRequest }
reason! { length_and_encoding, LengthAndEncoding, MalformedRequest }
reason! { connection_list, ConnectionList, MalformedRequest }
reason! { connection_names_framing, ConnectionNamesFraming, MalformedRequest }
