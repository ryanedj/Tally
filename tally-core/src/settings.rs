use crate::error::{LoadError, StartupProblem};
use crate::parser::normalize_host;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// The settings file as written, before "check_settings".
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSettings {
    client: String,
    listen_address: SocketAddr,
    log_folder: PathBuf,
    hosts: BTreeMap<String, SocketAddr>,
    keys: BTreeMap<String, RawKey>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKey {
    placeholder: String,
    host: String,
    header: String,
}

/// Settings that passed every startup rule: only "check_settings" builds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    client: String,
    listen_address: SocketAddr,
    log_folder: PathBuf,
    hosts: BTreeMap<String, SocketAddr>,
    keys: BTreeMap<String, Key>,
}

impl Settings {
    pub fn client(&self) -> &str {
        &self.client
    }

    pub fn listen_address(&self) -> SocketAddr {
        self.listen_address
    }

    pub fn log_folder(&self) -> &Path {
        &self.log_folder
    }

    /// Each host, as lowercase "host:port", with the Server's address.
    pub fn hosts(&self) -> &BTreeMap<String, SocketAddr> {
        &self.hosts
    }

    /// Each key, by name.
    pub fn keys(&self) -> &BTreeMap<String, Key> {
        &self.keys
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    placeholder: String,
    host: String,
    header: String,
}

impl Key {
    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    /// Lowercase "host:port".
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Lowercase.
    pub fn header(&self) -> &str {
        &self.header
    }
}

/// The real keys, by key name. A value only leaves through "get": "Debug" prints the names.
#[derive(Deserialize)]
#[serde(transparent)]
pub struct Secrets(BTreeMap<String, String>);

impl Secrets {
    /// The real key named "name".
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// Reads a settings or secrets file.
pub fn read_config_file(path: &Path) -> Result<Vec<u8>, LoadError> {
    std::fs::read(path).map_err(|source| LoadError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// The SHA-256 of the settings file's bytes, in lowercase hex.
pub fn settings_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Reads the settings from the file's bytes, "path" names the file in an error.
pub fn parse_settings(bytes: &[u8], path: &Path) -> Result<RawSettings, LoadError> {
    toml::from_slice(bytes).map_err(|error: toml::de::Error| LoadError::InvalidSettings {
        path: path.to_path_buf(),
        line: line_of(&error, bytes),
        message: error.message().to_string(),
    })
}

/// Reads the secrets from the file's bytes. An error gives only the line, since the TOML error
/// could quote a secret.
pub fn parse_secrets(bytes: &[u8], path: &Path) -> Result<Secrets, LoadError> {
    toml::from_slice(bytes).map_err(|error: toml::de::Error| LoadError::InvalidSecrets {
        path: path.to_path_buf(),
        line: line_of(&error, bytes),
    })
}

/// The line a toml error is at, counted from 1, "None" when toml gives no position.
fn line_of(error: &toml::de::Error, bytes: &[u8]) -> Option<usize> {
    error.span().map(|span| {
        1 + bytes
            .iter()
            .take(span.start)
            .filter(|&&b| b == b'\n')
            .count()
    })
}

/// Checks every startup rule (1. Proxy startup): the settings with hosts normalized and headers
/// lowercased, or every problem found.
pub fn check_settings(
    raw: RawSettings,
    secrets: &Secrets,
) -> Result<Settings, Vec<StartupProblem>> {
    let mut problems = Vec::new();

    let mut hosts = BTreeMap::new();
    for (written, address) in raw.hosts {
        match normalize_host(written.as_bytes()) {
            None => problems.push(StartupProblem::InvalidHost { host: written }),
            Some(host) => {
                if hosts.insert(host.clone(), address).is_some() {
                    problems.push(StartupProblem::HostListedTwice { host });
                }
            }
        }
    }

    let mut keys = BTreeMap::new();
    let mut placeholders: BTreeMap<String, String> = BTreeMap::new();
    for (name, raw_key) in raw.keys {
        let host = normalize_host(raw_key.host.as_bytes()).unwrap_or(raw_key.host);
        if !hosts.contains_key(&host) {
            problems.push(StartupProblem::KeyHostNotListed { key: name.clone() });
        }
        let is_placeholder = raw_key
            .placeholder
            .strip_prefix("TALLY_")
            .is_some_and(|hex| {
                hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            });
        if !is_placeholder {
            problems.push(StartupProblem::MalformedPlaceholder { key: name.clone() });
        }
        if let Some(other) = placeholders.insert(raw_key.placeholder.clone(), name.clone()) {
            problems.push(StartupProblem::PlaceholderReused {
                key: name.clone(),
                other,
            });
        }
        let header = raw_key.header.to_ascii_lowercase();
        let is_forbidden = matches!(
            header.as_str(),
            "host" | "connection" | "keep-alive" | "te" | "content-length" | "transfer-encoding"
        ) || header.starts_with("proxy-");
        if is_forbidden {
            problems.push(StartupProblem::ForbiddenHeader { key: name.clone() });
        }
        if secrets.get(&name).is_none() {
            problems.push(StartupProblem::NoSecret { key: name.clone() });
        }
        let key = Key {
            placeholder: raw_key.placeholder,
            host,
            header,
        };
        keys.insert(name, key);
    }

    for (name, value) in &secrets.0 {
        if !keys.contains_key(name) {
            problems.push(StartupProblem::SecretWithoutKey {
                secret: name.clone(),
            });
        }
        let is_valid =
            (1..=4096).contains(&value.len()) && value.bytes().all(|b| (0x21..=0x7e).contains(&b));
        if !is_valid {
            problems.push(StartupProblem::InvalidSecret {
                secret: name.clone(),
            });
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Settings {
        client: raw.client,
        listen_address: raw.listen_address,
        log_folder: raw.log_folder,
        hosts,
        keys,
    })
}
