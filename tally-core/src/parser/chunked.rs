use super::{MAX_HEAD, MAX_HEADERS, is_token};
use crate::error::ParseError;

/// Where a chunked body's framing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    /// The chunk size's hex digits.
    #[default]
    Size,
    /// Spaces or tabs after the size, before ";".
    SizeSpace,
    /// A chunk extension, up to its "\r".
    Extension,
    /// The size line's "\n".
    SizeEnd,
    /// The chunk's data, this many bytes still to come.
    Data(u64),
    /// The "\r" after the data.
    DataCr,
    /// The "\n" after the data.
    DataLf,
    /// After the last chunk: a trailer line, or the final empty line.
    LineStart,
    TrailerName,
    TrailerValue,
    /// A trailer line's "\n".
    TrailerEnd,
    /// The final empty line's "\n".
    LastEnd,
}

/// Follows a chunked body's framing (RFC 9112 §7.1) as its bytes arrive, without keeping them.
/// Each line is judged byte by byte, within the head's limits.
#[derive(Debug, Clone, Default)]
pub struct ChunkedBody {
    state: State,
    /// The chunk size read so far, and its number of digits.
    size: u64,
    digits: u32,
    /// Bytes of the current size line, or of the whole trailer section.
    line: usize,
    /// Trailer lines so far.
    trailers: usize,
}

impl ChunkedBody {
    /// Reads the next bytes from the Server: "None" when they all belong to the body and it
    /// hasn't ended, "Some(n)" when it ends after the first n of them.
    pub fn advance(&mut self, bytes: &[u8]) -> Result<Option<usize>, ParseError> {
        let mut pos = 0;
        while let Some(rest) = bytes.get(pos..).filter(|rest| !rest.is_empty()) {
            if let State::Data(left) = self.state {
                let take = usize::try_from(left).map_or(rest.len(), |left| left.min(rest.len()));
                pos += take;
                let left = left - take as u64;
                self.state = if left == 0 {
                    State::DataCr
                } else {
                    State::Data(left)
                };
                continue;
            }
            let Some(&b) = rest.first() else {
                break;
            };
            pos += 1;
            if !matches!(self.state, State::DataCr | State::DataLf) {
                self.line += 1;
                if self.line > MAX_HEAD {
                    return Err(ParseError::TooLarge);
                }
            }

            self.state = match (self.state, b) {
                (State::Size, _) => match char::from(b).to_digit(16) {
                    // 16 digits always fit: what httparse accepts
                    Some(digit) if self.digits < 16 => {
                        self.size = self.size * 16 + u64::from(digit);
                        self.digits += 1;
                        State::Size
                    }
                    _ if self.digits == 0 => return Err(ParseError::ChunkSize),
                    _ => match b {
                        b'\r' => State::SizeEnd,
                        b';' => State::Extension,
                        b' ' | b'\t' => State::SizeSpace,
                        b'\n' => return Err(ParseError::LineEnding),
                        _ => return Err(ParseError::ChunkSize),
                    },
                },
                // Spaces only before an extension: "chunk-size [ chunk-ext ] CRLF"
                (State::SizeSpace, b' ' | b'\t') => State::SizeSpace,
                (State::SizeSpace, b';') => State::Extension,
                (State::SizeSpace, _) => return Err(ParseError::ChunkSize),
                (State::Extension, b'\r') => State::SizeEnd,
                (State::Extension, b'\n') => return Err(ParseError::LineEnding),
                (State::Extension, b'\t' | 0x20..=0x7e | 0x80..=0xff) => State::Extension,
                (State::Extension, _) => return Err(ParseError::ChunkExtension),
                (State::SizeEnd, b'\n') => {
                    self.line = 0;
                    if self.size == 0 {
                        State::LineStart
                    } else {
                        State::Data(self.size)
                    }
                }
                (State::SizeEnd, _) => return Err(ParseError::LineEnding),
                (State::DataCr, b'\r') => State::DataLf,
                (State::DataLf, b'\n') => {
                    self.size = 0;
                    self.digits = 0;
                    State::Size
                }
                (State::DataCr | State::DataLf, _) => return Err(ParseError::ChunkDataEnd),
                // Trailer lines, checked as header lines
                (State::LineStart, b'\r') => State::LastEnd,
                (State::LineStart, b'\n') => return Err(ParseError::LineEnding),
                (State::LineStart, _) if is_token(b) => {
                    if self.trailers == MAX_HEADERS {
                        return Err(ParseError::TooManyHeaders);
                    }
                    self.trailers += 1;
                    State::TrailerName
                }
                (State::LineStart, _) => return Err(ParseError::HeaderName),
                (State::TrailerName, b':') => State::TrailerValue,
                (State::TrailerName, _) if is_token(b) => State::TrailerName,
                (State::TrailerName, _) => return Err(ParseError::HeaderName),
                (State::TrailerValue, b'\r') => State::TrailerEnd,
                (State::TrailerValue, b'\n') => return Err(ParseError::LineEnding),
                (State::TrailerValue, b'\t' | 0x20..=0x7e | 0x80..=0xff) => State::TrailerValue,
                (State::TrailerValue, _) => return Err(ParseError::HeaderValue),
                (State::TrailerEnd, b'\n') => State::LineStart,
                (State::LastEnd, b'\n') => return Ok(Some(pos)),
                (State::TrailerEnd | State::LastEnd, _) => return Err(ParseError::LineEnding),
                // Read above, a whole run at a time
                (state @ State::Data(_), _) => state,
            };
        }
        Ok(None)
    }
}
