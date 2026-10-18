use crate::common::{head_of_length, head_with_headers, res};
use tally_core::{BodyEnd, ResponseHead};

res! { response_200_with_a_length,
    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n",
    then b"ok",
    |head| {
        assert_eq!(head, ResponseHead {
            status: 200,
            reason: b"OK".to_vec(),
            headers: vec![("content-length".to_string(), b"2".to_vec())],
            body_end: BodyEnd::Length(2),
            connection_names: vec![],
        });
    }
}

res! { response_204,
    b"HTTP/1.1 204 No Content\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::NoBody); }
}

// A 304's Content-Length gives the size of the resource, not of a body (RFC 9110 §8.6).
res! { response_304_with_a_length_has_no_body,
    b"HTTP/1.1 304 Not Modified\r\nContent-Length: 1234\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::NoBody); }
}

res! { response_103_is_interim,
    b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\n",
    |head| {
        assert_eq!(head.status, 103);
        assert_eq!(head.body_end, BodyEnd::NoBody);
    }
}

res! { response_100_continue,
    b"HTTP/1.1 100 Continue\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::NoBody); }
}

res! { response_chunked,
    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::UntilServerCloses); }
}

res! { response_with_any_transfer_encoding,
    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::UntilServerCloses); }
}

res! { response_without_a_length,
    b"HTTP/1.1 200 OK\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::UntilServerCloses); }
}

res! { response_with_a_length_of_0,
    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::Length(0)); }
}

// The 1 MiB limit is for request bodies only.
res! { response_longer_than_a_request_body,
    b"HTTP/1.1 200 OK\r\nContent-Length: 10000000000\r\n\r\n",
    |head| { assert_eq!(head.body_end, BodyEnd::Length(10000000000)); }
}

res! { response_reason_with_spaces,
    b"HTTP/1.1 404 Not Found Here\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.reason, b"Not Found Here"); }
}

res! { response_with_an_empty_reason,
    b"HTTP/1.1 200 \r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.reason, b""); }
}

res! { response_reason_with_a_tab_and_bytes_above_0x7f,
    b"HTTP/1.1 200 O\tK caf\xe9\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.reason, b"O\tK caf\xe9"); }
}

res! { response_with_an_unknown_code,
    b"HTTP/1.1 599 Whatever\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.status, 599); }
}

res! { response_connection_names,
    b"HTTP/1.1 200 OK\r\nConnection: Keep-Alive, X-Trace\r\nX-Trace: 1\r\nContent-Length: 0\r\n\r\n",
    |head| {
        assert_eq!(head.connection_names, vec!["keep-alive", "x-trace"]);
        assert_eq!(head.headers.len(), 3);
    }
}

res! { response_connection_skips_empty_elements,
    b"HTTP/1.1 200 OK\r\nConnection: close, , x-foo\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.connection_names, vec!["close", "x-foo"]); }
}

res! { response_connection_on_two_lines,
    b"HTTP/1.1 200 OK\r\nConnection: close\r\nConnection: x-a\r\nContent-Length: 0\r\n\r\n",
    |head| { assert_eq!(head.connection_names, vec!["close", "x-a"]); }
}

res! { response_with_exactly_100_headers,
    &head_with_headers(b"HTTP/1.1 200 OK", 100),
    |head| { assert_eq!(head.headers.len(), 100); }
}

res! { response_head_of_exactly_8_kib,
    &head_of_length(b"HTTP/1.1 200 OK", 8192),
    |_| {}
}
