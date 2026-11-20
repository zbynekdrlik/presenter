//! The production TCP connector over a real loopback socket (plain `ws://`,
//! no TLS): host and port from the URL, the shared handshake, the headers.

use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

use super::super::conn::{request, Connector, TlsConnector};
use super::*;

#[tokio::test]
async fn the_tcp_connector_upgrades_a_plain_ws_url_with_the_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut seen = None;
        // tungstenite's `Callback` fixes the error type (an http response).
        #[allow(clippy::result_large_err)]
        let callback = |req: &Request, resp: Response| {
            seen = Some((req.uri().path().to_string(), req.headers().clone()));
            Ok(resp)
        };
        let _ws = tokio_tungstenite::accept_hdr_async(tcp, callback)
            .await
            .unwrap();
        seen.unwrap()
    });
    let mut local = env(Some(KEY));
    local.url = format!("ws://127.0.0.1:{port}/transcribe-websocket");

    let connected = TlsConnector.connect(request(&local).unwrap()).await;

    assert!(connected.is_ok(), "{:?}", connected.err());
    let (path, headers) = server.await.unwrap();
    assert_eq!(path, "/transcribe-websocket");
    assert_eq!(
        headers["authorization"].to_str().unwrap(),
        "Bearer test-key-not-real"
    );
    let agent = headers["user-agent"].to_str().unwrap();
    assert!(agent.starts_with("presenter-listen/"), "{agent}");
}
