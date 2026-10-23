use crate::error::ParseError;
use serde::{Deserialize, Serialize};

/// The reason a "refuse" line gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefuseReason {
    IncompleteRequest,
    HeadTooLarge,
    BodyTooLarge,
    MalformedRequest,
    UnsupportedRequest,
    HostNotAllowed,
}

/// The reason an "upstream_error" line gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamReason {
    ConnectFailed,
    ConnectTimeout,
    SendFailed,
    Shutdown,
    BadResponse,
    ResponseTimeout,
    ResponseIncomplete,
}

/// The reason a request gets when the parser refuses its head. Every variant is listed, so a new
/// one needs a reason. A response's parse error is always "bad_response".
impl From<ParseError> for RefuseReason {
    fn from(error: ParseError) -> Self {
        match error {
            ParseError::Http10
            | ParseError::Method
            | ParseError::Scheme
            | ParseError::TransferEncoding
            | ParseError::Upgrade
            | ParseError::Expect
            | ParseError::Connection => RefuseReason::UnsupportedRequest,
            ParseError::TooLarge | ParseError::TooManyHeaders => RefuseReason::HeadTooLarge,
            ParseError::BodyTooLarge => RefuseReason::BodyTooLarge,
            ParseError::LineEnding
            | ParseError::LeadingEmptyLine
            | ParseError::RequestLine
            | ParseError::Version
            | ParseError::SpaceBeforeFirstHeader
            | ParseError::HeaderName
            | ParseError::HeaderValue
            | ParseError::Folding
            | ParseError::ContentLength
            | ParseError::Target
            | ParseError::Host
            | ParseError::Port
            | ParseError::PathOrQuery
            | ParseError::StatusLine
            | ParseError::StatusCode
            | ParseError::SwitchingProtocols
            | ParseError::LengthOnNoBody
            | ParseError::LengthAndEncoding
            | ParseError::ConnectionList
            | ParseError::ConnectionNamesFraming => RefuseReason::MalformedRequest,
        }
    }
}
