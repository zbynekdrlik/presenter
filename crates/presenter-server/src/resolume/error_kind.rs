//! #563c: classify a Resolume host error into timeout / connect-refused /
//! connect-other / reset / other, so host-error logs and the status surface
//! can say WHICH of those happened instead of the opaque top-level context
//! message (`with_context(...).to_string()` drops the reqwest/io source
//! chain — the incident's "failed to fetch composition", never "timed out"
//! or "connection refused").
//!
//! Also the trigger classification for #564's port-drift probe: ONLY a
//! `ConnectRefused` (Resolume's process is up but nothing is bound on the
//! dialed port) should ever kick off a port scan — a `Timeout` or DNS
//! failure means the host itself is unreachable, and probing a range of
//! ports there would just be six more ways to time out.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolumeErrorKind {
    /// The request timed out (`COMPOSITION_TIMEOUT` / `ACTION_TIMEOUT`).
    Timeout,
    /// TCP connect failed with ECONNREFUSED — the host answers, but nothing
    /// is listening on the dialed port (Resolume's own restart raced ours,
    /// or the wrong port is configured).
    ConnectRefused,
    /// Any other connect-phase failure (DNS, network unreachable, TCP
    /// connect timeout not surfaced as [`Self::Timeout`]).
    ConnectOther,
    /// The peer reset the connection mid-request (ECONNRESET).
    Reset,
    /// Anything else (a non-2xx status, a JSON parse failure, ...).
    Other,
}

/// Walk the full error chain (context layers + the underlying reqwest/io
/// error) looking for the most specific classification. Pure + deterministic
/// — unit-tested against both real reqwest errors (connect-refused, timeout)
/// and synthetic `io::Error`s (reset), never guessed.
///
/// A REAL connect-refused reqwest error is itself a `reqwest::Error` whose
/// `is_connect()` is true — that alone can't distinguish "refused" from any
/// other connect-phase failure. The specific `io::ErrorKind` only shows up
/// FURTHER down the SAME chain (`reqwest::Error` → hyper's connect error →
/// `std::io::Error`). So this walks the ENTIRE chain before deciding: an
/// `io::Error` match wins immediately (most specific, returned right away);
/// a `reqwest::Error` predicate match is remembered as a FALLBACK and only
/// used if the rest of the chain never yields a more specific `io::Error`.
pub(super) fn classify_error(err: &anyhow::Error) -> ResolumeErrorKind {
    let mut fallback: Option<ResolumeErrorKind> = None;
    for cause in err.chain() {
        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
            match io_err.kind() {
                std::io::ErrorKind::ConnectionRefused => return ResolumeErrorKind::ConnectRefused,
                std::io::ErrorKind::ConnectionReset => return ResolumeErrorKind::Reset,
                std::io::ErrorKind::TimedOut => return ResolumeErrorKind::Timeout,
                _ => {}
            }
        }
        if fallback.is_none() {
            if let Some(reqwest_err) = cause.downcast_ref::<reqwest::Error>() {
                if reqwest_err.is_timeout() {
                    fallback = Some(ResolumeErrorKind::Timeout);
                } else if reqwest_err.is_connect() {
                    // A connect-PHASE failure (DNS, TLS, host unreachable, …)
                    // without a more specific io::Error kind above — the
                    // host itself could not be reached at all, so a
                    // port-drift probe would be pointless (nothing in the
                    // window would answer either).
                    fallback = Some(ResolumeErrorKind::ConnectOther);
                } else if reqwest_err.is_request() {
                    // A request-DISPATCH failure that is NOT a connect-phase
                    // failure — empirically, this is what a REUSED pooled
                    // keep-alive connection produces when the peer has gone
                    // away/moved ports ("connection closed before message
                    // completed"), rather than a fresh io::Error::
                    // ConnectionRefused (which only happens when there is no
                    // pooled connection to reuse). Operationally identical
                    // for our purposes — the endpoint no longer behaves like
                    // a live Resolume instance — so this ALSO must trigger
                    // the #564 port-drift probe.
                    fallback = Some(ResolumeErrorKind::ConnectRefused);
                }
            }
        }
    }
    fallback.unwrap_or(ResolumeErrorKind::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::TcpListener as TokioTcpListener;

    /// Bind a port then immediately free it — nothing can be listening there
    /// (short of a concurrent process racing us, negligible on loopback in a
    /// test's lifetime), so connecting to it deterministically ECONNREFUSEs.
    fn free_local_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        listener.local_addr().expect("local addr").port()
    }

    #[tokio::test]
    async fn classifies_a_real_connection_refused_error() {
        let port = free_local_port();
        let client = reqwest::Client::new();
        let err = client
            .get(format!("http://127.0.0.1:{port}/api/v1/product"))
            .send()
            .await
            .expect_err("nothing listens on a just-freed port");
        let wrapped: anyhow::Error = anyhow::Error::new(err).context("failed to fetch composition");
        assert_eq!(classify_error(&wrapped), ResolumeErrorKind::ConnectRefused);
    }

    #[tokio::test]
    async fn classifies_a_real_timeout_error() {
        // Accept the connection but never write a response — the client's
        // own request timeout must fire before anything arrives.
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                // Hold the connection open (no response) until the test's
                // client-side timeout fires and drops it.
                tokio::time::sleep(Duration::from_secs(5)).await;
                drop(stream);
            }
        });

        let client = reqwest::Client::new();
        let err = client
            .get(format!("http://{addr}/api/v1/composition"))
            .timeout(Duration::from_millis(150))
            .send()
            .await
            .expect_err("the server never responds, so the client times out");
        let wrapped: anyhow::Error = anyhow::Error::new(err).context("failed to fetch composition");
        assert_eq!(classify_error(&wrapped), ResolumeErrorKind::Timeout);
    }

    /// The Reset case is exercised against a synthetic `io::Error` — proving
    /// the CLASSIFIER'S own chain-walk logic, not that reqwest necessarily
    /// surfaces this exact io::ErrorKind for every RST on every platform.
    #[test]
    fn classifies_a_connection_reset_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer");
        let wrapped: anyhow::Error = anyhow::Error::new(io_err).context("failed to trigger clip");
        assert_eq!(classify_error(&wrapped), ResolumeErrorKind::Reset);
    }

    #[test]
    fn classifies_an_unrecognised_error_as_other() {
        let err = anyhow::anyhow!("composition request failed with status 500");
        assert_eq!(classify_error(&err), ResolumeErrorKind::Other);
    }

    #[test]
    fn the_full_chain_is_rendered_on_one_line_via_alternate_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let wrapped = anyhow::Error::new(io_err)
            .context("failed to fetch composition from http://host/api/v1/composition");
        let rendered = format!("{wrapped:#}");
        assert!(
            rendered.contains("failed to fetch composition"),
            "must keep the context: {rendered}"
        );
        assert!(
            rendered.contains("refused"),
            "must include the underlying cause, unlike `.to_string()`: {rendered}"
        );
        assert!(
            !rendered.contains('\n'),
            "the alternate rendering must stay on ONE line for log/grep friendliness: {rendered}"
        );
    }
}
