use crate::common::{head_of_length, head_with_headers, req};
use tally_core::{Method, RequestHead};

req! { a_plain_get,
    b"GET http://api.local/items?id=7 HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| {
        assert_eq!(head, RequestHead {
            method: Method::Get,
            host: "api.local".to_string(),
            port: 80,
            path_and_query: "/items?id=7".to_string(),
            headers: vec![("host".to_string(), b"api.local".to_vec())],
            content_length: 0,
            wants_close: false,
        });
    }
}

req! { a_post_body_is_not_part_of_the_head,
    b"POST http://api.local/items HTTP/1.1\r\nHost: api.local\r\nContent-Length: 5\r\n\r\n",
    then b"hello",
    |head| {
        assert_eq!(head.method, Method::Post);
        assert_eq!(head.content_length, 5);
    }
}

req! { a_put,
    b"PUT http://api.local/items/7 HTTP/1.1\r\nHost: api.local\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.method, Method::Put); }
}

req! { a_patch,
    b"PATCH http://api.local/items/7 HTTP/1.1\r\nHost: api.local\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.method, Method::Patch); }
}

req! { a_delete,
    b"DELETE http://api.local/items/7 HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.method, Method::Delete); }
}

// Empty lines before the request line are skipped, and belong to its head (RFC 9112 §2.2)

req! { an_empty_line_before_the_request_line,
    b"\r\nGET http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.path_and_query, "/"); }
}

req! { several_empty_lines_before_the_request_line,
    b"\r\n\r\n\r\nGET http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.path_and_query, "/"); }
}

req! { a_glued_request_is_left_for_the_next_call,
    b"GET http://api.local/one HTTP/1.1\r\nHost: api.local\r\n\r\n",
    then b"GET http://api.local/two HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.path_and_query, "/one"); }
}

req! { a_body_past_8_kib_is_not_part_of_the_head,
    b"POST http://api.local/items HTTP/1.1\r\nHost: api.local\r\nContent-Length: 9000\r\n\r\n",
    then &[b'x'; 9000],
    |head| { assert_eq!(head.content_length, 9000); }
}

req! { no_headers_at_all,
    b"GET http://api.local/ HTTP/1.1\r\n\r\n",
    |head| { assert!(head.headers.is_empty()); }
}

req! { names_are_lowercased,
    b"GET http://api.local/ HTTP/1.1\r\nHoSt: api.local\r\nX-Custom-Header: v\r\n\r\n",
    |head| {
        assert_eq!(head.headers, vec![
            ("host".to_string(), b"api.local".to_vec()),
            ("x-custom-header".to_string(), b"v".to_vec()),
        ]);
    }
}

// L3 counts two "authorization" headers, so a key found there is refused.
req! { both_spellings_of_a_header_are_kept,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nauthorization: Bearer TALLY_fedcba9876543210fedcba9876543210\r\n\r\n",
    |head| {
        assert_eq!(head.headers, vec![
            ("host".to_string(), b"api.local".to_vec()),
            ("authorization".to_string(), b"Bearer TALLY_0123456789abcdef0123456789abcdef".to_vec()),
            ("authorization".to_string(), b"Bearer TALLY_fedcba9876543210fedcba9876543210".to_vec()),
        ]);
    }
}

req! { spaces_and_tabs_around_a_value_are_dropped,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: \t value with spaces \t\r\n\r\n",
    |head| { assert_eq!(head.headers[1].1, b"value with spaces"); }
}

req! { a_tab_inside_a_value_is_kept,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: a\tb\r\n\r\n",
    |head| { assert_eq!(head.headers[1].1, b"a\tb"); }
}

req! { bytes_above_0x7f_in_a_value_are_kept,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A: caf\xe9\r\n\r\n",
    |head| { assert_eq!(head.headers[1].1, b"caf\xe9"); }
}

req! { an_empty_value_is_kept,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A:\r\n\r\n",
    |head| { assert_eq!(head.headers[1].1, b""); }
}

req! { a_value_needs_no_space_after_the_colon,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nX-A:value\r\n\r\n",
    |head| { assert_eq!(head.headers[1].1, b"value"); }
}

req! { an_uppercase_scheme_is_http,
    b"GET HTTP://API.Local/Path HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| {
        assert_eq!(head.host, "api.local");
        assert_eq!(head.path_and_query, "/Path");
    }
}

req! { an_empty_path_is_a_slash,
    b"GET http://api.local HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.path_and_query, "/"); }
}

req! { a_query_without_a_path_gets_a_slash,
    b"GET http://api.local?x=1 HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.path_and_query, "/?x=1"); }
}

req! { a_port,
    b"GET http://api.local:8080/ HTTP/1.1\r\nHost: api.local:8080\r\n\r\n",
    |head| { assert_eq!(head.port, 8080); }
}

req! { an_empty_port_is_80,
    b"GET http://api.local:/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.port, 80); }
}

req! { leading_zeros_in_a_port_are_read,
    b"GET http://api.local:080/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.port, 80); }
}

req! { the_highest_port,
    b"GET http://api.local:65535/ HTTP/1.1\r\nHost: api.local:65535\r\n\r\n",
    |head| { assert_eq!(head.port, 65535); }
}

req! { an_ipv4_host,
    b"GET http://127.0.0.1:9001/ HTTP/1.1\r\nHost: 127.0.0.1:9001\r\n\r\n",
    |head| {
        assert_eq!(head.host, "127.0.0.1");
        assert_eq!(head.port, 9001);
    }
}

req! { every_character_rfc_3986_allows_in_a_path_and_query,
    b"GET http://api.local/a-b._~!$&'()*+,;=:@/%41%4a?x=/?:@ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    |head| { assert_eq!(head.path_and_query, "/a-b._~!$&'()*+,;=:@/%41%4a?x=/?:@"); }
}

req! { exactly_100_headers,
    &head_with_headers(b"GET http://api.local/ HTTP/1.1", 100),
    |head| { assert_eq!(head.headers.len(), 100); }
}

req! { a_head_of_exactly_8_kib,
    &head_of_length(b"GET http://api.local/ HTTP/1.1", 8192),
    |_| {}
}

req! { a_content_length_of_0,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.content_length, 0); }
}

req! { a_content_length_of_exactly_1_mib,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length: 1048576\r\n\r\n",
    |head| { assert_eq!(head.content_length, 1048576); }
}

req! { a_content_length_with_spaces_around_it,
    b"POST http://api.local/ HTTP/1.1\r\nHost: api.local\r\nContent-Length:  5 \r\n\r\n",
    |head| { assert_eq!(head.content_length, 5); }
}

req! { connection_close_in_any_case,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nConnection: Close\r\n\r\n",
    |head| { assert!(head.wants_close); }
}

req! { connection_keep_alive,
    b"GET http://api.local/ HTTP/1.1\r\nHost: api.local\r\nConnection: keep-alive\r\n\r\n",
    |head| { assert!(!head.wants_close); }
}
