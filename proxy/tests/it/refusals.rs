// Done when: every refusal case can be triggered, is logged, and never reaches the fake API,
// each malformed_request rule included, the Host ones among them. Protocol 4's rows "Head
// over 8 KiB or 100 headers", "Content-Length over 1 MiB", "Parse error" and "Refused by
// decide". The parser's own tables (L2) check each rule's exact error; these check the whole
// path, on a real connection.

use crate::common::{
    LIMITS, Program, SECRETS, arrivals, settings_for, start_fake_api, start_proxy, summary,
};

/// A request "request" gets a reply starting with "status", then the connection closes; the
/// log holds "line"; nothing reaches the fake API.
macro_rules! refused {
    ($name:ident, $request:expr, $status:expr, $line:expr) => {
        #[tokio::test]
        async fn $name() {
            let (server, seen) = start_fake_api().await;
            let proxy = start_proxy(&settings_for(server), SECRETS, LIMITS).await;
            let mut program = Program::connect(proxy.address).await;
            let request: &[u8] = $request;
            program.send(request).await;
            let reply = program.rest().await;
            let reply = String::from_utf8_lossy(&reply);
            assert!(reply.starts_with($status), "{reply}");
            assert!(reply.contains("connection: close\r\n"), "{reply}");

            let (_, lines) = proxy.stop().await;
            assert_eq!(summary(&lines), ["start", $line, "stop"]);
            assert!(arrivals(&seen).is_empty(), "nothing reaches the fake API");
        }
    };
}

const BAD_REQUEST: &str = "HTTP/1.1 400 Bad Request\r\n";
const MALFORMED: &str = "refuse [] null malformed_request";
const NOT_IMPLEMENTED: &str = "HTTP/1.1 501 Not Implemented\r\n";
const UNSUPPORTED: &str = "refuse [] null unsupported_request";

// malformed_request, from the parser
refused!(
    a_line_ending_in_a_bare_lf,
    b"GET http://api.weather.local/ HTTP/1.1\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_lone_cr_in_a_value,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nX-A: a\rb\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    two_spaces_in_the_request_line,
    b"GET  http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_version_other_than_1_1,
    b"GET http://api.weather.local/ HTTP/2.0\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_lone_lf_before_the_request_line,
    b"\nGET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    whitespace_before_the_first_header,
    b"GET http://api.weather.local/ HTTP/1.1\r\n Host: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_space_before_a_colon,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost : api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_header_name_outside_the_token_set,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nX(A): a\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_folded_header,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nX-A: a\r\n b\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_control_character_in_a_value,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nX-A: a\x01b\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_content_length_with_a_sign,
    b"POST http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nContent-Length: +1\r\n\r\na",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    two_content_lengths,
    b"POST http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\na",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_short_form_target,
    b"GET / HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    user_info_in_the_host,
    b"GET http://user@api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_host_with_a_trailing_dot,
    b"GET http://api.weather.local./ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_percent_encoded_host,
    b"GET http://api%2Eweather.local/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    an_empty_host,
    b"GET http:///ok HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_port_with_a_letter,
    b"GET http://api.weather.local:8a/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    port_0,
    b"GET http://api.weather.local:0/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_port_over_65535,
    b"GET http://api.weather.local:65536/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_path_character_rfc_3986_doesnt_allow,
    b"GET http://api.weather.local/a<b HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_percent_without_two_hex_digits,
    b"GET http://api.weather.local/%zz HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);
refused!(
    a_fragment,
    b"GET http://api.weather.local/a#b HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    MALFORMED
);

// malformed_request, from decide: the bound header of a key found, twice (RFC 9110 §5.3)
refused!(
    the_bound_header_twice,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\nauthorization: other\r\n\r\n",
    BAD_REQUEST,
    "refuse [weather_key] api.weather.local:80 malformed_request"
);

// malformed_request, from decide: no Host, several, or an invalid one (RFC 9112 §3.2). A valid
// Host naming another host is forwarded, see "connection"
refused!(
    no_host_header,
    b"GET http://api.weather.local/ HTTP/1.1\r\n\r\n",
    BAD_REQUEST,
    "refuse [] api.weather.local:80 malformed_request"
);
refused!(
    two_host_headers,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nHost: api.weather.local\r\n\r\n",
    BAD_REQUEST,
    "refuse [] api.weather.local:80 malformed_request"
);
refused!(
    a_host_header_the_parser_refuses,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api_weather.local\r\n\r\n",
    BAD_REQUEST,
    "refuse [] api.weather.local:80 malformed_request"
);

// host_not_allowed
refused!(
    a_key_toward_a_host_it_isnt_allowed_for,
    b"GET http://evil.local/ HTTP/1.1\r\nHost: evil.local\r\nAuthorization: Bearer TALLY_0123456789abcdef0123456789abcdef\r\n\r\n",
    "HTTP/1.1 403 Forbidden\r\n",
    "refuse [weather_key] evil.local:80 host_not_allowed"
);
refused!(
    no_key_toward_a_host_that_isnt_listed,
    b"GET http://evil.local/ HTTP/1.1\r\nHost: evil.local\r\n\r\n",
    "HTTP/1.1 403 Forbidden\r\n",
    "refuse [] evil.local:80 host_not_allowed"
);

// unsupported_request. Logged with no host: refused before decide looks (DESIGN, The log).
refused!(
    a_transfer_encoding,
    b"POST http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    NOT_IMPLEMENTED,
    UNSUPPORTED
);
refused!(
    an_upgrade,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nUpgrade: websocket\r\n\r\n",
    NOT_IMPLEMENTED,
    UNSUPPORTED
);
refused!(
    an_expect,
    b"POST http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nExpect: 100-continue\r\nContent-Length: 1\r\n\r\na",
    NOT_IMPLEMENTED,
    UNSUPPORTED
);
refused!(
    http_1_0,
    b"GET http://api.weather.local/ HTTP/1.0\r\nHost: api.weather.local\r\n\r\n",
    NOT_IMPLEMENTED,
    UNSUPPORTED
);
refused!(
    a_scheme_other_than_http,
    b"GET https://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    NOT_IMPLEMENTED,
    UNSUPPORTED
);
refused!(
    a_connection_naming_a_header,
    b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nConnection: Authorization\r\n\r\n",
    NOT_IMPLEMENTED,
    UNSUPPORTED
);
refused!(
    a_method_other_than_the_five,
    b"OPTIONS http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\n\r\n",
    NOT_IMPLEMENTED,
    UNSUPPORTED
);

// head_too_large
refused!(
    a_head_over_8_kib,
    &[
        b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nX-A: ".as_slice(),
        &[b'a'; 8 * 1024],
        b"\r\n\r\n"
    ]
    .concat(),
    "HTTP/1.1 431 Request Header Fields Too Large\r\n",
    "refuse [] null head_too_large"
);
refused!(
    over_100_headers,
    &[
        b"GET http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\n".as_slice(),
        &b"X-A: a\r\n".repeat(100),
        b"\r\n"
    ]
    .concat(),
    "HTTP/1.1 431 Request Header Fields Too Large\r\n",
    "refuse [] null head_too_large"
);

// body_too_large, before any body is read
refused!(
    a_content_length_over_1_mib,
    b"POST http://api.weather.local/ HTTP/1.1\r\nHost: api.weather.local\r\nContent-Length: 1048577\r\n\r\n",
    "HTTP/1.1 413 Content Too Large\r\n",
    "refuse [] null body_too_large"
);
