//! Operator construction — the single place archives get their storage handle.
//!
//! Reading one member of a remote archive is a long-lived ranged GET: hundreds
//! of megabytes, decoded as they arrive, for tens of minutes. Origins drop such
//! connections. When that happens the body fails mid-flight with a transport
//! error while only part of the range has been delivered, and because DEFLATE
//! has no mid-stream entry point the caller cannot pick up where it stopped —
//! it has to restart the whole member from byte zero. On a big member that
//! turns one dropped packet into a lost half-hour.
//!
//! opendal's `RetryLayer` makes that recovery invisible. Its reader wrapper
//! tracks how many bytes of the range it has already handed out, so a retried
//! chunk re-issues the request with `Range` advanced past them: the byte stream
//! the caller sees stays contiguous and the decoder never learns a reconnect
//! happened. Retries fire only for errors opendal marks temporary (transport
//! drops, timeouts, 5xx, 429) — a 404 or a malformed archive still fails fast.

use opendal::layers::RetryLayer;
use std::io::{Error as IoError, ErrorKind};
use std::time::Duration;

/// Attempts for a single failed chunk. The budget is per read, so it resets
/// after every chunk that lands: one long stream can survive many unrelated
/// drops, and only a genuinely dead origin exhausts it.
const MAX_RETRIES: usize = 8;
/// Delay before the first retry, doubling per attempt up to `MAX_DELAY` and
/// jittered so concurrent readers of the same origin don't resynchronize onto
/// it. Worst case is well under a minute of waiting per incident — cheap next
/// to re-reading the bytes already delivered.
const MIN_DELAY: Duration = Duration::from_millis(500);
const MAX_DELAY: Duration = Duration::from_secs(10);

/// Build an operator rooted at `uri` (a directory) whose reads resume in place
/// after a dropped connection.
pub(crate) fn build(uri: &str) -> Result<opendal::Operator, IoError> {
    let operator = opendal::Operator::from_uri(uri).map_err(|e| {
        IoError::new(
            ErrorKind::Other,
            format!("Failed to create operator: {}", e),
        )
    })?;
    Ok(operator.layer(
        RetryLayer::new()
            .with_max_times(MAX_RETRIES)
            .with_min_delay(MIN_DELAY)
            .with_max_delay(MAX_DELAY)
            .with_jitter(),
    ))
}
