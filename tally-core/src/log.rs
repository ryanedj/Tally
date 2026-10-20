use crate::error::LogLineError;
use crate::reason::{RefuseReason, UpstreamReason};
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};

/// One line of a run's log, as written. Each event has exactly its fields:
/// a missing one or any other is refused, even "refuse"'s "host", which may be null.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum LogLine {
    Start {
        seq: u64,
        run: String,
        client: String,
        settings_sha256: String,
        time: String,
    },
    Release {
        seq: u64,
        client: String,
        // Written as an array, an empty one is refused
        keys: NonEmpty<String>,
        host: String,
        time: String,
    },
    Pass {
        seq: u64,
        client: String,
        keys: Vec<String>,
        host: String,
        time: String,
    },
    Refuse {
        seq: u64,
        client: String,
        keys: Vec<String>,
        // A plain "Option" would also accept a line without "host"
        #[serde(deserialize_with = "Option::deserialize")]
        host: Option<String>,
        reason: RefuseReason,
        time: String,
    },
    UpstreamError {
        seq: u64,
        client: String,
        keys: Vec<String>,
        host: String,
        about: u64,
        reason: UpstreamReason,
        time: String,
    },
    Stop {
        seq: u64,
        client: String,
        time: String,
    },
}

/// What a caller hands the Writer: a line without "seq", "client" and "time", which the Writer
/// adds (2. Log writer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Start {
        run: String,
        settings_sha256: String,
    },
    Release {
        keys: NonEmpty<String>,
        host: String,
    },
    Pass {
        host: String,
    },
    Refuse {
        keys: Vec<String>,
        host: Option<String>,
        reason: RefuseReason,
    },
    UpstreamError {
        keys: Vec<String>,
        host: String,
        about: u64,
        reason: UpstreamReason,
    },
    Stop,
}

/// Reads one line of a log, without its "\n" and checks each event's field
pub fn parse_log_line(text: &str) -> Result<LogLine, LogLineError> {
    let line: LogLine = serde_json::from_str(text)?;
    match &line {
        LogLine::Pass { keys, .. } if !keys.is_empty() => Err(LogLineError::PassWithKeys),
        _ => Ok(line),
    }
}
