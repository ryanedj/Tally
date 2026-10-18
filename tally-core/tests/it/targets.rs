use crate::common::req;

// The form of the target

req! { a_short_form_target,
    b"GET /items HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Target)
}

req! { an_asterisk_target,
    b"OPTIONS * HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Target)
}

// Syntax first: CONNECT's target fails before its method is looked at.
req! { connect_with_its_authority_target,
    b"CONNECT api.local:443 HTTP/1.1\r\nHost: api.local:443\r\n\r\n",
    Err(Target)
}

req! { a_scheme_without_slashes,
    b"GET http:api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Target)
}

req! { a_scheme_starting_with_a_digit,
    b"GET 1http://api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Target)
}

// The host

req! { user_info,
    b"GET http://user@api.local/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Host)
}

req! { an_allowed_host_in_front_of_user_info,
    b"GET http://allowed.local@evil.local/ HTTP/1.1\r\nHost: evil.local\r\n\r\n",
    Err(Host)
}

req! { a_trailing_dot,
    b"GET http://api.local./ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Host)
}

req! { percent_encoding_in_the_host,
    b"GET http://api%2elocal/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Host)
}

req! { an_underscore_in_the_host,
    b"GET http://api_local/ HTTP/1.1\r\nHost: api_local\r\n\r\n",
    Err(Host)
}

req! { an_ipv6_literal,
    b"GET http://[::1]/ HTTP/1.1\r\nHost: [::1]\r\n\r\n",
    Err(Host)
}

req! { an_empty_host,
    b"GET http:///items HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Host)
}

req! { an_empty_host_with_a_port,
    b"GET http://:8080/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Host)
}

// The port

req! { port_0,
    b"GET http://api.local:0/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Port)
}

req! { port_65536,
    b"GET http://api.local:65536/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Port)
}

req! { a_letter_in_the_port,
    b"GET http://api.local:80a/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Port)
}

req! { a_port_too_long_for_u64,
    b"GET http://api.local:99999999999999999999999/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Port)
}

// "u64::from_str("+80")" is "Ok(80)": every byte of a port must be a digit.
req! { a_port_with_a_plus_sign,
    b"GET http://api.local:+80/ HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(Port)
}

// The path and query

req! { a_fragment,
    b"GET http://api.local/items#top HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(PathOrQuery)
}

req! { a_raw_brace_in_the_query,
    b"GET http://api.local/items?q={x} HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(PathOrQuery)
}

req! { a_percent_without_hex_digits,
    b"GET http://api.local/%zz HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(PathOrQuery)
}

req! { a_percent_cut_short,
    b"GET http://api.local/a%4 HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(PathOrQuery)
}

req! { a_quote_in_the_path,
    b"GET http://api.local/a\"b HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(PathOrQuery)
}

req! { a_backslash_in_the_path,
    b"GET http://api.local/a\\b HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(PathOrQuery)
}

req! { utf_8_in_the_path,
    b"GET http://api.local/caf\xc3\xa9 HTTP/1.1\r\nHost: api.local\r\n\r\n",
    Err(PathOrQuery)
}

// parse_host_port, also used by decide (L3) on each Host value

macro_rules! host {
    ($name:ident, $bytes:expr, Ok(($host:expr, $port:expr))) => {
        #[test]
        fn $name() {
            assert_eq!(
                tally_core::parse_host_port($bytes),
                Ok(($host.to_string(), $port))
            );
        }
    };
    ($name:ident, $bytes:expr, Err($error:ident)) => {
        #[test]
        fn $name() {
            assert_eq!(
                tally_core::parse_host_port($bytes),
                Err(tally_core::ParseError::$error)
            );
        }
    };
}

host! { host_alone, b"api.local", Ok(("api.local", 80)) }
host! { host_with_a_port, b"API.Local:8080", Ok(("api.local", 8080)) }
host! { host_with_an_empty_port, b"api.local:", Ok(("api.local", 80)) }
host! { host_ipv4, b"127.0.0.1:9001", Ok(("127.0.0.1", 9001)) }
host! { host_empty, b"", Err(Host) }
host! { host_with_user_info, b"user@api.local", Err(Host) }
host! { host_with_a_trailing_dot, b"api.local.", Err(Host) }
host! { host_with_a_space, b"api local", Err(Host) }
host! { host_with_port_0, b"api.local:0", Err(Port) }
host! { host_with_port_65536, b"api.local:65536", Err(Port) }
host! { host_with_a_plus_port, b"api.local:+80", Err(Port) }
