#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    // malformed_request (pass 1)
    /// A line ending that isn't "\r\n", or a lone "\r" or "\n".
    LineEnding,
    /// An empty line before the start line.
    LeadingEmptyLine,
    /// A request line that doesn't split into three parts on single spaces, or a method
    /// outside the token set.
    RequestLine,
    /// A version other than "HTTP/1.1" and "HTTP/1.0".
    Version,
    /// A status line that isn't "VERSION code reason", one space apart.
    StatusLine,
    /// A status code outside 100 to 599.
    StatusCode,
    /// Whitespace between the start line and the first header.
    SpaceBeforeFirstHeader,
    /// A header name outside the token set, or a space before the colon.
    HeaderName,
    /// A control character other than tab in a header value (a lone "\r" or "\n" is "LineEnding").
    HeaderValue,
    /// A header continued on the next line.
    Folding,
    /// A "Content-Length" that isn't plain digits, or appears twice.
    ContentLength,
    /// A target that isn't "scheme://host[:port]" with an optional path and query.
    Target,
    /// User info, a trailing dot, percent-encoding, another character, or an empty host.
    Host,
    /// A port with characters other than digits, or 0, or over 65535.
    Port,
    /// A character RFC 3986 doesn't allow in a path or query, a bad "%", or a "#".
    PathOrQuery,
    // head_too_large (pass 1)
    /// Over 8 KiB.
    TooLarge,
    /// Over 100 headers.
    TooManyHeaders,
    // unsupported_request (pass 2)
    /// "HTTP/1.0": checked with the meaning in a request, with the status line in a response.
    Http10,
    /// A method other than "GET", "POST", "PUT", "PATCH" and "DELETE".
    Method,
    /// A scheme other than "http", in any case.
    Scheme,
    /// Any "Transfer-Encoding".
    TransferEncoding,
    /// Any "Upgrade".
    Upgrade,
    /// Any "Expect".
    Expect,
    /// A "Connection" other than one line with a single "close" or "keep-alive".
    Connection,
    // body_too_large (pass 2)
    /// A "Content-Length" over 1 MiB.
    BodyTooLarge,
    // responses only (pass 2)
    /// A "101".
    SwitchingProtocols,
    /// A "1xx" or "204" with "Content-Length" or "Transfer-Encoding".
    LengthOnNoBody,
    /// Both "Content-Length" and "Transfer-Encoding".
    LengthAndEncoding,
    /// A "Connection" element that isn't a token.
    ConnectionList,
    /// A "Connection" naming "content-length" or "transfer-encoding".
    ConnectionNamesFraming,
}
