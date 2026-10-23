use std::future::Future;
use std::io;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

/// Why a wait ended without its result.
pub(crate) enum Interrupted {
    TimedOut,
    CutOff,
}

/// Why "write_with_progress" didn't write every byte.
pub(crate) enum WriteFailed {
    Broken,
    TimedOut,
    CutOff,
}

/// Waits for "operation" until "deadline". Stops at once once "cut_off" is set, which is checked
/// first.
pub(crate) async fn until<T>(
    operation: impl Future<Output = T>,
    deadline: Instant,
    cut_off: &CancellationToken,
) -> Result<T, Interrupted> {
    tokio::select! {
        biased;
        () = cut_off.cancelled() => Err(Interrupted::CutOff),
        () = sleep_until(deadline) => Err(Interrupted::TimedOut),
        output = operation => Ok(output),
    }
}

/// Reads more of "stream" into "buffer", with room for at least 64 KiB: how many bytes, 0 when
/// the other side closed, or the error when the connection broke.
pub(crate) async fn read_more(
    stream: &mut (impl AsyncRead + Unpin),
    buffer: &mut Vec<u8>,
    deadline: Instant,
    cut_off: &CancellationToken,
) -> Result<io::Result<usize>, Interrupted> {
    buffer.reserve(64 * 1024);
    until(stream.read_buf(buffer), deadline, cut_off).await
}

/// Writes every byte of "bytes" to "stream". Each write must make progress within "limit".
pub(crate) async fn write_with_progress(
    stream: &mut (impl AsyncWrite + Unpin),
    mut bytes: &[u8],
    limit: Duration,
    cut_off: &CancellationToken,
) -> Result<(), WriteFailed> {
    while !bytes.is_empty() {
        match until(stream.write(bytes), Instant::now() + limit, cut_off).await {
            Ok(Ok(0) | Err(_)) => return Err(WriteFailed::Broken),
            Ok(Ok(written)) => bytes = bytes.get(written..).unwrap_or_default(),
            Err(Interrupted::TimedOut) => return Err(WriteFailed::TimedOut),
            Err(Interrupted::CutOff) => return Err(WriteFailed::CutOff),
        }
    }
    Ok(())
}
