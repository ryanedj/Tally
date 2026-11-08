// A response's chunked body (RFC 9112 §7.1): where it ends, read as its bytes arrive. One test
// per row. Each row is fed whole, one byte at a time, and split in two at every point, with the
// same outcome. A row Tally accepts must have a first size line httparse accepts, of the same size.

use tally_core::{ChunkedBody, ParseError};

/// Feeds "pieces" one after the other: where the body ends, counted from the start of the first
/// piece, or "None" while it hasn't.
fn feed(pieces: &[&[u8]]) -> Result<Option<usize>, ParseError> {
    let mut body = ChunkedBody::default();
    let mut offset = 0;
    for piece in pieces {
        if let Some(end) = body.advance(piece)? {
            return Ok(Some(offset + end));
        }
        offset += piece.len();
    }
    Ok(None)
}

fn assert_fed(bytes: &[u8], expected: Result<Option<usize>, ParseError>) {
    assert_eq!(feed(&[bytes]), expected, "whole");
    let one_by_one: Vec<&[u8]> = bytes.chunks(1).collect();
    assert_eq!(feed(&one_by_one), expected, "one byte at a time");
    for split in 0..=bytes.len() {
        let (first, second) = bytes.split_at(split);
        assert_eq!(feed(&[first, second]), expected, "split at {split}");
    }
}

/// httparse reads the first size line too, and finds the same size: its data, then "\r\n". The
/// last chunk has no data: its trailer section follows.
fn assert_httparse_agrees(body: &[u8]) {
    match httparse::parse_chunk_size(body) {
        Ok(httparse::Status::Complete((line, size))) => {
            let data_end = usize::try_from(size)
                .ok()
                .filter(|&size| size > 0)
                .and_then(|size| line.checked_add(size))
                .and_then(|start| body.get(start..start.saturating_add(2)));
            assert!(
                data_end.is_none() || data_end == Some(b"\r\n".as_slice()),
                "httparse reads a size of {size}"
            );
        }
        Ok(httparse::Status::Partial) => {}
        Err(error) => panic!("httparse refuses the first size line: {error:?}"),
    }
}

macro_rules! chunked {
    ($name:ident, $bytes:expr, then $rest:expr, Ends) => {
        #[test]
        fn $name() {
            let body: &[u8] = $bytes;
            let rest: &[u8] = $rest;
            assert_fed(&[body, rest].concat(), Ok(Some(body.len())));
            assert_httparse_agrees(body);
        }
    };
    ($name:ident, $bytes:expr, Ends) => {
        chunked! { $name, $bytes, then b"", Ends }
    };
    ($name:ident, $bytes:expr, Partial) => {
        #[test]
        fn $name() {
            let body: &[u8] = $bytes;
            assert_fed(body, Ok(None));
            assert_httparse_agrees(body);
        }
    };
    ($name:ident, $bytes:expr, Err($error:ident)) => {
        #[test]
        fn $name() {
            assert_fed($bytes, Err(ParseError::$error));
        }
    };
}

/// A body whose first size line is "len" bytes long, "\r\n" included.
fn size_line_of(len: usize) -> Vec<u8> {
    let mut body = b"3;".to_vec();
    body.resize(len - 2, b'x');
    body.extend_from_slice(b"\r\nabc\r\n0\r\n\r\n");
    body
}

/// A body whose trailer section, after "0\r\n", is "len" bytes long, the final "\r\n" included.
fn trailer_section_of(len: usize) -> Vec<u8> {
    let mut body = b"0\r\nx: ".to_vec();
    body.resize(len - 1, b'a');
    body.extend_from_slice(b"\r\n\r\n");
    body
}

/// A body with "count" trailer lines.
fn trailers(count: usize) -> Vec<u8> {
    let mut body = b"0\r\n".to_vec();
    for _ in 0..count {
        body.extend_from_slice(b"x: 1\r\n");
    }
    body.extend_from_slice(b"\r\n");
    body
}

// Where the body ends

chunked! { one_chunk, b"3\r\nabc\r\n0\r\n\r\n", Ends }
chunked! { two_chunks, b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n", Ends }
chunked! { only_the_last_chunk, b"0\r\n\r\n", Ends }
chunked! { data_that_looks_like_framing, b"5\r\n0\r\n\r\n\r\n0\r\n\r\n", Ends }
chunked! { size_in_either_case, b"A\r\n0123456789\r\nb\r\n0123456789a\r\n0\r\n\r\n", Ends }
// "last-chunk = 1*("0")"
chunked! { leading_zeros, b"003\r\nabc\r\n000\r\n\r\n", Ends }
chunked! { sixteen_digits, b"000000000000000a\r\n0123456789\r\n0\r\n\r\n", Ends }
chunked! { an_extension, b"3;name=value\r\nabc\r\n0\r\n\r\n", Ends }
chunked! { an_extension_with_a_quoted_value, b"3;x=\"a b\"\r\nabc\r\n0\r\n\r\n", Ends }
// "chunk-ext = *( BWS ";" BWS ..."
chunked! { spaces_before_an_extension, b"3 \t;x\r\nabc\r\n0\r\n\r\n", Ends }
chunked! { an_extension_on_the_last_chunk, b"0;x\r\n\r\n", Ends }
chunked! { a_trailer, b"0\r\nx-checksum: 1\r\n\r\n", Ends }
chunked! { a_trailer_with_an_empty_value, b"0\r\nx-checksum:\r\n\r\n", Ends }
chunked! { a_hundred_trailers, &trailers(100), Ends }
chunked! { a_size_line_of_8_kib, &size_line_of(8192), Ends }
chunked! { a_trailer_section_of_8_kib, &trailer_section_of(8192), Ends }
// The end is found whatever follows it: these bytes are never the body
chunked! { bytes_after_the_end, b"3\r\nabc\r\n0\r\n\r\n", then b"HTTP/1.1 200 OK\r\n\r\n", Ends }

// Not ended yet

chunked! { cut_inside_the_size_line, b"3", Partial }
chunked! { cut_inside_the_data, b"3\r\nab", Partial }
chunked! { cut_before_the_last_chunk, b"3\r\nabc\r\n", Partial }
chunked! { cut_before_the_final_empty_line, b"3\r\nabc\r\n0\r\n", Partial }
chunked! { cut_after_a_trailer, b"0\r\nx: 1\r\n", Partial }
chunked! { the_largest_size, b"ffffffffffffffff\r\nabc", Partial }

// Refused: the size line

chunked! { no_size, b"\r\nabc\r\n0\r\n\r\n", Err(ChunkSize) }
chunked! { size_with_0x, b"0x5\r\nabcde\r\n0\r\n\r\n", Err(ChunkSize) }
chunked! { size_with_a_plus, b"+5\r\nabcde\r\n0\r\n\r\n", Err(ChunkSize) }
chunked! { size_with_a_minus, b"-5\r\nabcde\r\n0\r\n\r\n", Err(ChunkSize) }
chunked! { seventeen_digits, b"0000000000000000a\r\n0123456789\r\n0\r\n\r\n", Err(ChunkSize) }
// The grammar has no spaces without an extension
chunked! { a_space_and_no_extension, b"5 \r\nabcde\r\n0\r\n\r\n", Err(ChunkSize) }
chunked! { a_space_inside_the_size, b"5 5\r\nabcde\r\n0\r\n\r\n", Err(ChunkSize) }
chunked! { a_bare_lf_after_the_size, b"5\nabcde\r\n0\r\n\r\n", Err(LineEnding) }
chunked! { a_lone_cr_after_the_size, b"5\rabcde\r\n0\r\n\r\n", Err(LineEnding) }
// httparse allows any byte in an extension, a lone LF could end the line for another reader
chunked! { a_bare_lf_inside_an_extension, b"5;a\nb\r\nabcde\r\n0\r\n\r\n", Err(LineEnding) }
chunked! { a_nul_inside_an_extension, b"5;a\x00b\r\nabcde\r\n0\r\n\r\n", Err(ChunkExtension) }
chunked! { a_size_line_past_8_kib, &size_line_of(8193), Err(TooLarge) }

// Refused: the data

chunked! { data_one_byte_longer, b"3\r\nabcd\r\n0\r\n\r\n", Err(ChunkDataEnd) }
chunked! { data_one_byte_shorter, b"3\r\nab\r\n0\r\n\r\n", Err(ChunkDataEnd) }
chunked! { a_bare_lf_after_the_data, b"3\r\nabc\n0\r\n\r\n", Err(ChunkDataEnd) }

// Refused: the trailer section

chunked! { a_trailer_with_no_colon, b"0\r\nx-checksum\r\n\r\n", Err(HeaderName) }
chunked! { a_trailer_starting_with_a_space, b"0\r\n x: 1\r\n\r\n", Err(HeaderName) }
chunked! { a_nul_in_a_trailer_value, b"0\r\nx: a\x00b\r\n\r\n", Err(HeaderValue) }
chunked! { a_bare_lf_after_a_trailer, b"0\r\nx: 1\n\r\n", Err(LineEnding) }
chunked! { a_bare_lf_as_the_final_line, b"0\r\n\n", Err(LineEnding) }
chunked! { a_hundred_and_one_trailers, &trailers(101), Err(TooManyHeaders) }
chunked! { a_trailer_section_past_8_kib, &trailer_section_of(8193), Err(TooLarge) }
