use crate::parser::{RequestHead, host_port, normalize_host};
use crate::reason::RefuseReason;
use crate::settings::Settings;
use nonempty::NonEmpty;

/// What "decide" says about a request (3. Decide request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// At least one key found in its bound header, each allowed for this host.
    Release {
        keys: NonEmpty<String>,
        host: String,
    },
    /// No key found, and the host is listed.
    Pass {
        host: String,
    },
    Refuse(Refusal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: RefuseReason,
    /// The names of the keys found, in name order; empty when not known yet.
    pub keys: Vec<String>,
    /// Lowercase "host:port"; "None" when not known yet.
    pub host: Option<String>,
}

/// 3. Decide request: the first case that applies.
pub fn decide(request: &RequestHead, settings: &Settings) -> Decision {
    let url_host = host_port(&request.host, request.port);
    // A key counts only when its placeholder is in its own bound header
    let keys_found: Vec<_> = settings
        .keys()
        .iter()
        .filter(|(_, key)| {
            let placeholder = key.placeholder().as_bytes();
            values_of(request, key.header()).any(|value| {
                value
                    .windows(placeholder.len())
                    .any(|part| part == placeholder)
            })
        })
        .collect();
    let names: Vec<String> = keys_found.iter().map(|(name, _)| (*name).clone()).collect();
    let refuse = |reason| {
        Decision::Refuse(Refusal {
            reason,
            keys: names.clone(),
            host: Some(url_host.clone()),
        })
    };

    // Case 1: host_mismatch
    let host_headers: Vec<Option<String>> =
        values_of(request, "host").map(normalize_host).collect();
    if host_headers != [Some(url_host.clone())] {
        return refuse(RefuseReason::HostMismatch);
    }

    // Case 2: malformed_request (a bound header appears more than once)
    if keys_found
        .iter()
        .any(|(_, key)| values_of(request, key.header()).count() > 1)
    {
        return refuse(RefuseReason::MalformedRequest);
    }

    // Case 3: host_not_allowed (a key not allowed for this host)
    if keys_found.iter().any(|(_, key)| key.host() != url_host) {
        return refuse(RefuseReason::HostNotAllowed);
    }

    // Case 4: host_not_allowed (no key, host not listed)
    if keys_found.is_empty() && !settings.hosts().contains_key(&url_host) {
        return refuse(RefuseReason::HostNotAllowed);
    }

    match NonEmpty::from_vec(names) {
        // Case 5: pass
        None => Decision::Pass { host: url_host },
        // Case 6: release
        Some(keys) => Decision::Release {
            keys,
            host: url_host,
        },
    }
}

/// The values of the header lines named "name" (lowercase), in order.
fn values_of<'a>(request: &'a RequestHead, name: &'a str) -> impl Iterator<Item = &'a [u8]> {
    request
        .headers
        .iter()
        .filter(move |(header, _)| header == name)
        .map(|(_, value)| value.as_slice())
}
