use crate::common::checked;
use std::path::Path;
use tally_core::{
    LoadError, StartupProblem, check_settings, parse_secrets, parse_settings, read_config_file,
    settings_sha256,
};

const SETTINGS: &str = include_str!("fixtures/one-key.toml");
const SECRETS: &str = include_str!("fixtures/one-key-secrets.toml");

/// Every problem "settings" and "secrets" break, in the order found. Empty if they pass.
fn problems(settings: &str, secrets: &str) -> Vec<StartupProblem> {
    let raw = parse_settings(settings.as_bytes(), Path::new("settings.toml"))
        .expect("the settings parse");
    let secrets =
        parse_secrets(secrets.as_bytes(), Path::new("secrets.toml")).expect("the secrets parse");
    check_settings(raw, &secrets).err().unwrap_or_default()
}

// Loading

#[test]
fn a_valid_example_loads() {
    let (settings, secrets) = checked(SETTINGS, SECRETS);
    assert_eq!(settings.client(), "weather-app");
    assert_eq!(
        settings.listen_address(),
        "127.0.0.1:8080".parse().expect("an address")
    );
    assert_eq!(settings.log_folder(), Path::new("logs"));
    assert_eq!(
        settings.hosts().get("api.weather.local:80"),
        Some(&"127.0.0.1:9001".parse().expect("an address"))
    );
    let key = settings.keys().get("weather_key").expect("the key");
    assert_eq!(key.placeholder(), "TALLY_0123456789abcdef0123456789abcdef");
    assert_eq!(key.host(), "api.weather.local:80");
    assert_eq!(key.header(), "authorization");
    assert_eq!(secrets.get("weather_key"), Some("the-real-key"));
}

#[test]
fn hosts_are_normalized_and_headers_lowercased() {
    let settings = SETTINGS
        .replace("\"api.weather.local:80\" =", "\"API.Weather.Local\" =")
        .replace(
            "host = \"api.weather.local:80\"",
            "host = \"api.weather.local:080\"",
        )
        .replace("Authorization", "X-Api-Key");
    let (settings, _) = checked(&settings, SECRETS);
    assert!(settings.hosts().contains_key("api.weather.local:80"));
    let key = settings.keys().get("weather_key").expect("the key");
    assert_eq!(key.host(), "api.weather.local:80");
    assert_eq!(key.header(), "x-api-key");
}

#[test]
fn a_repeated_key_is_refused_by_toml() {
    let settings = format!(
        "{SETTINGS}\n[keys.weather_key]\nplaceholder = \"TALLY_fedcba9876543210fedcba9876543210\"\nhost = \"api.weather.local:80\"\nheader = \"X-Api-Key\"\n"
    );
    assert!(matches!(
        parse_settings(settings.as_bytes(), Path::new("settings.toml")),
        Err(LoadError::InvalidSettings { .. })
    ));
}

#[test]
fn a_repeated_host_is_refused_by_toml() {
    let settings = SETTINGS.replace(
        "[hosts]\n",
        "[hosts]\n\"api.weather.local:80\" = \"127.0.0.1:9002\"\n",
    );
    assert!(matches!(
        parse_settings(settings.as_bytes(), Path::new("settings.toml")),
        Err(LoadError::InvalidSettings { .. })
    ));
}

#[test]
fn an_unknown_field_is_refused() {
    let settings = SETTINGS.replace("header =", "hedaer =");
    assert!(matches!(
        parse_settings(settings.as_bytes(), Path::new("settings.toml")),
        Err(LoadError::InvalidSettings { .. })
    ));
}

#[test]
fn an_unknown_top_level_field_is_refused() {
    let settings = format!("colour = \"red\"\n{SETTINGS}");
    assert!(matches!(
        parse_settings(settings.as_bytes(), Path::new("settings.toml")),
        Err(LoadError::InvalidSettings { .. })
    ));
}

#[test]
fn a_missing_field_is_refused() {
    let settings = SETTINGS.replace("header = \"Authorization\"\n", "");
    assert!(matches!(
        parse_settings(settings.as_bytes(), Path::new("settings.toml")),
        Err(LoadError::InvalidSettings { .. })
    ));
}

// An address must be an IP address.
#[test]
fn a_name_instead_of_an_ip_address_is_refused() {
    let settings = SETTINGS.replace("\"127.0.0.1:9001\"", "\"fake-api:9001\"");
    assert!(matches!(
        parse_settings(settings.as_bytes(), Path::new("settings.toml")),
        Err(LoadError::InvalidSettings { .. })
    ));
}

#[test]
fn a_secrets_error_gives_only_the_line() {
    let error = parse_secrets(
        b"weather_key = \"the-real-key\"\nother = 12345\n",
        Path::new("secrets.toml"),
    )
    .expect_err("a number isn't a secret");
    assert!(matches!(
        error,
        LoadError::InvalidSecrets { line: Some(2), .. }
    ));
    let message = error.to_string();
    assert!(message.contains("secrets.toml"), "{message}");
    assert!(!message.contains("12345"), "{message}");
    assert!(!message.contains("the-real-key"), "{message}");
}

#[test]
fn a_settings_error_names_its_file_and_line() {
    let settings = SETTINGS.replace("header =", "hedaer =");
    let error = parse_settings(settings.as_bytes(), Path::new("settings.toml"))
        .expect_err("hedaer isn't a field");
    assert!(matches!(
        error,
        LoadError::InvalidSettings { line: Some(12), .. }
    ));
    let message = error.to_string();
    assert!(message.contains("settings.toml"), "{message}");
    assert!(message.contains("at line 12"), "{message}");
    assert!(message.contains("unknown field `hedaer`"), "{message}");
}

// The secrets file passed as the settings file: the error names the field, never its value.
#[test]
fn swapped_files_never_print_a_secret() {
    let error = parse_settings(SECRETS.as_bytes(), Path::new("secrets.toml"))
        .expect_err("a secrets file isn't settings");
    let message = error.to_string();
    assert!(message.contains("unknown field `weather_key`"), "{message}");
    assert!(!message.contains("the-real-key"), "{message}");
    let debug = format!("{error:?}");
    assert!(!debug.contains("the-real-key"), "{debug}");
}

#[test]
fn a_file_that_cannot_be_read() {
    assert!(matches!(
        read_config_file(Path::new("does/not/exist.toml")),
        Err(LoadError::Read { .. })
    ));
}

#[test]
fn settings_sha256_of_abc() {
    assert_eq!(
        settings_sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

// Startup rules (1. Proxy startup), one each

#[test]
fn an_invalid_host() {
    let settings = SETTINGS.replace(
        "[hosts]\n",
        "[hosts]\n\"api_weather\" = \"127.0.0.1:9002\"\n",
    );
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::InvalidHost {
            host: "api_weather".to_string()
        }]
    );
}

#[test]
fn a_host_listed_twice_once_normalized() {
    let settings = SETTINGS.replace(
        "[hosts]\n",
        "[hosts]\n\"API.weather.local\" = \"127.0.0.1:9002\"\n",
    );
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::HostListedTwice {
            host: "api.weather.local:80".to_string()
        }]
    );
}

#[test]
fn a_key_host_not_listed() {
    let settings = SETTINGS.replace(
        "host = \"api.weather.local:80\"",
        "host = \"maps.local:80\"",
    );
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::KeyHostNotListed {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_placeholder_with_uppercase_hex() {
    let settings = SETTINGS.replace(
        "0123456789abcdef0123456789abcdef",
        "0123456789ABCDEF0123456789ABCDEF",
    );
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::MalformedPlaceholder {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_placeholder_too_short() {
    let settings = SETTINGS.replace("0123456789abcdef0123456789abcdef", "0123456789abcdef");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::MalformedPlaceholder {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_placeholder_with_another_prefix() {
    let settings = SETTINGS.replace("TALLY_", "SECRET");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::MalformedPlaceholder {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_placeholder_used_twice() {
    let settings = format!(
        "{SETTINGS}\n[keys.maps_key]\nplaceholder = \"TALLY_0123456789abcdef0123456789abcdef\"\nhost = \"api.weather.local:80\"\nheader = \"X-Api-Key\"\n"
    );
    let secrets = format!("{SECRETS}maps_key = \"maps-secret\"\n");
    assert_eq!(
        problems(&settings, &secrets),
        vec![StartupProblem::PlaceholderReused {
            key: "weather_key".to_string(),
            other: "maps_key".to_string()
        }]
    );
}

#[test]
fn a_key_bound_to_proxy_authorization() {
    let settings = SETTINGS.replace("\"Authorization\"", "\"Proxy-Authorization\"");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_key_bound_to_host() {
    let settings = SETTINGS.replace("\"Authorization\"", "\"Host\"");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_key_bound_to_content_length() {
    let settings = SETTINGS.replace("\"Authorization\"", "\"Content-Length\"");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_key_bound_to_connection() {
    let settings = SETTINGS.replace("\"Authorization\"", "\"Connection\"");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string()
        }]
    );
}

// The parser lets Keep-Alive and TE through with any value, and the rewrite drops them: the log
// would say a key left when it never did.
#[test]
fn a_key_bound_to_keep_alive() {
    let settings = SETTINGS.replace("\"Authorization\"", "\"Keep-Alive\"");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_key_bound_to_te() {
    let settings = SETTINGS.replace("\"Authorization\"", "\"TE\"");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_key_bound_to_transfer_encoding() {
    let settings = SETTINGS.replace("\"Authorization\"", "\"Transfer-Encoding\"");
    assert_eq!(
        problems(&settings, SECRETS),
        vec![StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_key_without_a_secret() {
    assert_eq!(
        problems(SETTINGS, ""),
        vec![StartupProblem::NoSecret {
            key: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_secret_without_a_key() {
    let secrets = format!("{SECRETS}other = \"x\"\n");
    assert_eq!(
        problems(SETTINGS, &secrets),
        vec![StartupProblem::SecretWithoutKey {
            secret: "other".to_string()
        }]
    );
}

// A secret ending in a newline would split the header it's swapped into.
#[test]
fn a_secret_ending_in_a_newline() {
    assert_eq!(
        problems(SETTINGS, "weather_key = \"the-real-key\\n\"\n"),
        vec![StartupProblem::InvalidSecret {
            secret: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_secret_with_a_space() {
    assert_eq!(
        problems(SETTINGS, "weather_key = \"the real key\"\n"),
        vec![StartupProblem::InvalidSecret {
            secret: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_secret_with_a_nul() {
    assert_eq!(
        problems(SETTINGS, "weather_key = \"the-real\\u0000key\"\n"),
        vec![StartupProblem::InvalidSecret {
            secret: "weather_key".to_string()
        }]
    );
}

// A DEL swapped into a header would give the Server a line Tally's parser refuses (HeaderValue).
#[test]
fn a_secret_with_a_del() {
    assert_eq!(
        problems(SETTINGS, "weather_key = \"the-real\\u007Fkey\"\n"),
        vec![StartupProblem::InvalidSecret {
            secret: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_secret_with_a_non_ascii_character() {
    assert_eq!(
        problems(SETTINGS, "weather_key = \"café-key\"\n"),
        vec![StartupProblem::InvalidSecret {
            secret: "weather_key".to_string()
        }]
    );
}

#[test]
fn an_empty_secret() {
    assert_eq!(
        problems(SETTINGS, "weather_key = \"\"\n"),
        vec![StartupProblem::InvalidSecret {
            secret: "weather_key".to_string()
        }]
    );
}

#[test]
fn a_secret_of_exactly_4096_characters() {
    let secrets = format!("weather_key = \"{}\"\n", "k".repeat(4096));
    assert_eq!(problems(SETTINGS, &secrets), vec![]);
}

#[test]
fn a_secret_of_4097_characters() {
    let secrets = format!("weather_key = \"{}\"\n", "k".repeat(4097));
    assert_eq!(
        problems(SETTINGS, &secrets),
        vec![StartupProblem::InvalidSecret {
            secret: "weather_key".to_string()
        }]
    );
}

// All of them, and never a secret's value

#[test]
fn every_problem_is_reported_at_once() {
    let settings = SETTINGS
        .replace("TALLY_", "SECRET")
        .replace("\"Authorization\"", "\"Host\"");
    let found = problems(&settings, "other = \"x\"\n");
    for problem in [
        StartupProblem::MalformedPlaceholder {
            key: "weather_key".to_string(),
        },
        StartupProblem::ForbiddenHeader {
            key: "weather_key".to_string(),
        },
        StartupProblem::NoSecret {
            key: "weather_key".to_string(),
        },
        StartupProblem::SecretWithoutKey {
            secret: "other".to_string(),
        },
    ] {
        assert!(
            found.contains(&problem),
            "{problem:?} missing from {found:?}"
        );
    }
    assert_eq!(found.len(), 4, "{found:?}");
}

#[test]
fn no_message_holds_a_secret() {
    let found = problems(SETTINGS, "weather_key = \"leaky secret\"\n");
    let message = found
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(message, "secret weather_key: invalid value");
}

#[test]
fn secrets_print_their_names_only() {
    let (_, secrets) = checked(SETTINGS, SECRETS);
    assert_eq!(format!("{secrets:?}"), "{\"weather_key\"}");
}
