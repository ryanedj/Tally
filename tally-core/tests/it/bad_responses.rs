use crate::common::{head_of_length, head_with_headers, res};

// The status line: the version, one space, three digits, one space, then the reason.
// A fault in that shape is StatusLine; a code outside 100 to 599 is StatusCode.

res! { response_http_1_0,
    b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n",
    Err(Http10)
}

res! { response_http_1_0_is_refused_with_its_status_line,
    b"HTTP/1.0 200 OK\r\nContent-Le",
    Err(Http10)
}

res! { response_http_1_2,
    b"HTTP/1.2 200 OK\r\nContent-Length: 0\r\n\r\n",
    Err(Version)
}

res! { response_lowercase_version,
    b"http/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
    Err(Version)
}

res! { response_without_a_space_after_the_code,
    b"HTTP/1.1 200\r\nContent-Length: 0\r\n\r\n",
    Err(StatusLine)
}

res! { response_two_spaces_before_the_code,
    b"HTTP/1.1  200 OK\r\nContent-Length: 0\r\n\r\n",
    Err(StatusLine)
}

res! { response_two_digit_code,
    b"HTTP/1.1 20 OK\r\nContent-Length: 0\r\n\r\n",
    Err(StatusLine)
}

res! { response_four_digit_code,
    b"HTTP/1.1 2000 OK\r\nContent-Length: 0\r\n\r\n",
    Err(StatusLine)
}

res! { response_letter_in_the_code,
    b"HTTP/1.1 2x0 OK\r\nContent-Length: 0\r\n\r\n",
    Err(StatusLine)
}

res! { response_nul_in_the_reason,
    b"HTTP/1.1 200 O\x00K\r\nContent-Length: 0\r\n\r\n",
    Err(StatusLine)
}

res! { response_code_099,
    b"HTTP/1.1 099 Low\r\nContent-Length: 0\r\n\r\n",
    Err(StatusCode)
}

res! { response_code_600,
    b"HTTP/1.1 600 High\r\nContent-Length: 0\r\n\r\n",
    Err(StatusCode)
}

// The checks of step 2 of 5. Relay response

res! { response_101,
    b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: upgrade\r\n\r\n",
    Err(SwitchingProtocols)
}

res! { response_103_with_a_length,
    b"HTTP/1.1 103 Early Hints\r\nContent-Length: 0\r\n\r\n",
    Err(LengthOnNoBody)
}

res! { response_100_with_transfer_encoding,
    b"HTTP/1.1 100 Continue\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(LengthOnNoBody)
}

res! { response_204_with_a_length,
    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n",
    Err(LengthOnNoBody)
}

res! { response_204_with_transfer_encoding,
    b"HTTP/1.1 204 No Content\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(LengthOnNoBody)
}

res! { response_with_a_length_and_transfer_encoding,
    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(LengthAndEncoding)
}

res! { response_with_transfer_encoding_then_a_length,
    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n",
    Err(LengthAndEncoding)
}

res! { response_length_too_big_for_u64,
    b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999999\r\n\r\n",
    Err(ContentLength)
}

res! { response_connection_naming_content_length,
    b"HTTP/1.1 200 OK\r\nConnection: Content-Length\r\nContent-Length: 0\r\n\r\n",
    Err(ConnectionNamesFraming)
}

res! { response_connection_naming_transfer_encoding,
    b"HTTP/1.1 200 OK\r\nConnection: close, transfer-encoding\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(ConnectionNamesFraming)
}

res! { response_connection_element_with_a_space,
    b"HTTP/1.1 200 OK\r\nConnection: close, x y\r\nContent-Length: 0\r\n\r\n",
    Err(ConnectionList)
}

res! { response_quoted_connection,
    b"HTTP/1.1 200 OK\r\nConnection: \"close\"\r\nContent-Length: 0\r\n\r\n",
    Err(ConnectionList)
}

// The rules shared with requests (pass 1)

res! { response_lone_lf_line_endings,
    b"HTTP/1.1 200 OK\nContent-Length: 0\n\n",
    Err(LineEnding)
}

res! { response_empty_line_before_the_status_line,
    b"\r\nHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
    Err(LeadingEmptyLine)
}

res! { response_whitespace_before_the_first_header,
    b"HTTP/1.1 200 OK\r\n Content-Length: 0\r\n\r\n",
    Err(SpaceBeforeFirstHeader)
}

res! { response_folded_header,
    b"HTTP/1.1 200 OK\r\nX-A: a\r\n b\r\nContent-Length: 0\r\n\r\n",
    Err(Folding)
}

res! { response_space_before_the_colon,
    b"HTTP/1.1 200 OK\r\nContent-Length : 5\r\n\r\n",
    Err(HeaderName)
}

res! { response_nul_in_a_value,
    b"HTTP/1.1 200 OK\r\nX-A: a\x00b\r\nContent-Length: 0\r\n\r\n",
    Err(HeaderValue)
}

res! { response_length_with_a_plus_sign,
    b"HTTP/1.1 200 OK\r\nContent-Length: +5\r\n\r\n",
    Err(ContentLength)
}

res! { response_two_lengths,
    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n",
    Err(ContentLength)
}

res! { response_head_of_8_kib_and_1_byte,
    &head_of_length(b"HTTP/1.1 200 OK", 8193),
    Err(TooLarge)
}

res! { response_with_101_headers,
    &head_with_headers(b"HTTP/1.1 200 OK", 101),
    Err(TooManyHeaders)
}
