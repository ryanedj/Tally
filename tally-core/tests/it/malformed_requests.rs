use crate::common::req;

// Line endings. A lone "\r" or "\n" is LineEnding, even inside a value.

req! { lone_lf_line_endings,
    b"GET http://api.local/ HTTP/1.1\nHost: api.local\n\n",
    Err(LineEnding)
}

req! { a_lone_lf_is_refused_before_the_head_ends,
    b"GET http://api.local/ HTTP/1.1\nHo",
    Err(LineEnding)
}

req! { a_content_length_hidden_behind_a_lone_lf,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\nContent-Length: 5\r\n\r\n",
    Err(LineEnding)
}

req! { a_lone_cr_inside_a_value,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\rb\r\n\r\n",
    Err(LineEnding)
}

req! { a_lone_lf_for_the_empty_line,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\n",
    Err(LineEnding)
}

// The request line, which must split into three parts on single spaces

req! { two_spaces_between_parts,
    b"GET  http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(RequestLine)
}

req! { a_tab_between_parts,
    b"GET\thttp://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(RequestLine)
}

req! { a_space_after_the_version,
    b"GET http://api.local/ HTTP/1.1 \r\nHost: api.local\r\n\r\n",
    Err(RequestLine)
}

req! { no_version,
    b"GET http://api.local/\r\nHost: api.local\r\n\r\n",
    Err(RequestLine)
}

req! { a_method_outside_the_token_set,
    b"GE(T http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(RequestLine)
}

req! { a_lowercase_version,
    b"GET http://api.local/ http/1.1\r\nHost: api.local\r\n\r\n",
    Err(Version)
}

req! { http_1_2,
    b"GET http://api.local/ HTTP/1.2\r\nHost: api.local\r\n\r\n",
    Err(Version)
}

req! { http_2_0,
    b"GET http://api.local/ HTTP/2.0\r\nHost: api.local\r\n\r\n",
    Err(Version)
}

// Around the header lines

req! { an_empty_line_before_the_request_line,
    b"\r\nGET http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(LeadingEmptyLine)
}

req! { whitespace_before_the_first_header,
    b"GET http://api.local/ HTTP/1.1\r\n Host: api.local\r\n\r\n",
    Err(SpaceBeforeFirstHeader)
}

req! { a_folded_header,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\r\n b\r\n\r\n",
    Err(Folding)
}

req! { a_folded_transfer_encoding,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nTransfer-Encoding: identity\r\n chunked\r\n\r\n",
    Err(Folding)
}

// Header names

req! { a_space_before_the_colon,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nTransfer-Encoding : chunked\r\n\r\n",
    Err(HeaderName)
}

req! { a_space_inside_a_name,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX Y: a\r\n\r\n",
    Err(HeaderName)
}

req! { an_empty_name,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\n: a\r\n\r\n",
    Err(HeaderName)
}

req! { a_line_without_a_colon,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A\r\n\r\n",
    Err(HeaderName)
}

req! { a_name_outside_the_token_set,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX(A): b\r\n\r\n",
    Err(HeaderName)
}

// Header values

req! { a_nul_in_a_value,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\x00b\r\n\r\n",
    Err(HeaderValue)
}

req! { a_del_in_a_value,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\x7fb\r\n\r\n",
    Err(HeaderValue)
}

req! { another_control_byte_in_a_value,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\x01b\r\n\r\n",
    Err(HeaderValue)
}

// Content-Length

req! { a_content_length_with_a_plus_sign,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: +5\r\n\r\n",
    Err(ContentLength)
}

req! { a_content_length_with_a_leading_zero,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 05\r\n\r\n",
    Err(ContentLength)
}

req! { a_content_length_of_two_zeros,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 00\r\n\r\n",
    Err(ContentLength)
}

req! { a_content_length_list,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 5, 5\r\n\r\n",
    Err(ContentLength)
}

req! { an_empty_content_length,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length:\r\n\r\n",
    Err(ContentLength)
}

req! { a_hexadecimal_content_length,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 0x5\r\n\r\n",
    Err(ContentLength)
}

req! { two_content_lengths_with_the_same_value,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n",
    Err(ContentLength)
}

req! { two_content_lengths_with_different_values,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 5\r\ncontent-length: 6\r\n\r\n",
    Err(ContentLength)
}
