pub mod passive;
pub mod stage;
pub mod stage_diag;
pub mod stream;

/// Build the WebSocket URL from the current page location, tagging the
/// connecting surface so the server can log which UI (operator/tablet) opened
/// the socket (see #471). `/live/ws` is shared by every UI and the handshake
/// carries no other surface signal, so the surface travels as a query param.
fn ws_url(surface: &str) -> String {
    let window = web_sys::window().expect("no global window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
    let host = location.host().unwrap_or_else(|_| "localhost".to_string());
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    format!("{ws_protocol}//{host}/live/ws?surface={surface}")
}
