use crate::common::{head_of_length, head_with_headers, req};

// head_too_large

req! { a_head_of_8_kib_and_1_byte,
    &head_of_length(b"GET http://api.local/ HTTP/1.1", 8193),
    Err(TooLarge)
}

req! { a_head_still_unfinished_at_8_kib,
    &head_of_length(b"GET http://api.local/ HTTP/1.1", 9000)[..8192],
    Err(TooLarge)
}

// Empty lines before the request line count toward the 8 KiB
req! { eight_kib_of_empty_lines,
    &b"\r\n".repeat(4096),
    Err(TooLarge)
}

req! { a_head_with_101_headers,
    &head_with_headers(b"GET http://api.local/ HTTP/1.1", 101),
    Err(TooManyHeaders)
}

// Each line is judged once it has fully arrived, as Go's net/textproto reads heads

req! { a_bad_line_cut_short_waits_for_its_end,
    b"GET http://api.local/ HTTP/1.1\r\nBad Na",
    Partial
}

req! { an_overlong_line_is_too_large_whatever_it_holds,
    &[b"GET http://api.local/ HTTP/1.1\r\nX(A): ".as_slice(), &[b'a'; 9000]].concat(),
    Err(TooLarge)
}

// Read in one piece or cut at 8 KiB, the same bytes give the same answer.
req! { an_overlong_line_ending_in_a_lone_lf_is_too_large,
    &[b"GET http://api.local/ HTTP/1.1\r\nX-A: ".as_slice(), &[b'a'; 9000], b"\n\r\n"].concat(),
    Err(TooLarge)
}

// unsupported_request

req! { http_1_0,
    b"GET http://api.local/ HTTP/1.0\r\nHost: api.local\r\n\r\n",
    Err(Http10)
}

req! { a_head_request,
    b"HEAD http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Method)
}

req! { an_options_request,
    b"OPTIONS http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Method)
}

req! { a_lowercase_method,
    b"get http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Method)
}

req! { https,
    b"GET https://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Scheme)
}

req! { an_uppercase_https,
    b"GET HTTPS://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Scheme)
}

req! { ftp,
    b"GET ftp://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Scheme)
}

req! { transfer_encoding_chunked,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(TransferEncoding)
}

req! { transfer_encoding_in_lowercase,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\ntransfer-encoding: identity\r\n\r\n",
    Err(TransferEncoding)
}

req! { content_length_and_transfer_encoding,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n",
    Err(TransferEncoding)
}

req! { an_upgrade_header,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nUpgrade: websocket\r\n\r\n",
    Err(Upgrade)
}

req! { an_expect_header,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 5\r\nExpect: 100-continue\r\n\r\n",
    Err(Expect)
}

req! { a_connection_list,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nConnection: keep-alive, close\r\n\r\n",
    Err(Connection)
}

req! { two_connection_lines,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nConnection: close\r\nConnection: close\r\n\r\n",
    Err(Connection)
}

req! { a_connection_naming_a_header,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nConnection: authorization\r\n\r\n",
    Err(Connection)
}

req! { an_empty_connection,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nConnection:\r\n\r\n",
    Err(Connection)
}

// body_too_large

req! { a_content_length_of_1_mib_and_1_byte,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 1048577\r\n\r\n",
    Err(BodyTooLarge)
}

req! { a_content_length_too_big_for_u64,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 99999999999999999999999\r\n\r\n",
    Err(BodyTooLarge)
}
