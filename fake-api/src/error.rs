use std::io;

#[derive(Debug, thiserror::Error)]
pub(crate) enum HandleError {
    #[error("socket error: {0}")]
    Io(#[from] io::Error),
    #[error("malformed request")]
    BadRequest,
    #[error("connection closed mid-request")]
    ClosedEarly,
    #[error("read limit reached before the request was complete")]
    TooLarge,
}
