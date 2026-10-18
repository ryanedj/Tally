use crate::common::{req, res};

// Which fault wins: syntax first, then meaning, each in byte order

req! { a_syntax_fault_wins_over_an_earlier_transfer_encoding,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nTransfer-Encoding: chunked\r\nBad Name: x\r\n\r\n",
    Err(HeaderName)
}

req! { a_syntax_fault_wins_over_an_earlier_scheme,
    b"GET https://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\x00\r\n\r\n",
    Err(HeaderValue)
}

req! { an_unsupported_scheme_waits_for_the_whole_head,
    b"GET https://api.local/ HTTP/1.1\r\nHo",
    Partial
}

req! { a_large_body_before_transfer_encoding,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 2000000\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(BodyTooLarge)
}

req! { transfer_encoding_before_a_large_body,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nTransfer-Encoding: chunked\r\nContent-Length: 2000000\r\n\r\n",
    Err(TransferEncoding)
}

req! { http_1_0_before_its_headers,
    b"POST http://api.local/ HTTP/1.0\r\nHost: api.local\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(Http10)
}

req! { a_bad_line_wins_over_a_later_overlong_line,
    &[b"GET http://api.local/ HTTP/1.1\r\nBad Name: x\r\nX-A: ".as_slice(), &[b'a'; 9000]].concat(),
    Err(HeaderName)
}

req! { the_first_of_two_bad_lines_wins,
    b"GET http://api.local/ HTTP/1.1\r\nX(A): b\r\nX-B: a\x00\r\n\r\n",
    Err(HeaderName)
}

req! { the_first_of_two_bad_lines_wins_in_the_other_order,
    b"GET http://api.local/ HTTP/1.1\r\nX-B: a\x00\r\nX(A): b\r\n\r\n",
    Err(HeaderValue)
}

res! { response_syntax_fault_wins_over_a_101,
    b"HTTP/1.1 101 Switching Protocols\r\nBad Name: x\r\n\r\n",
    Err(HeaderName)
}

res! { response_101_comes_before_its_headers,
    b"HTTP/1.1 101 Switching Protocols\r\nContent-Length: 5\r\n\r\n",
    Err(SwitchingProtocols)
}
