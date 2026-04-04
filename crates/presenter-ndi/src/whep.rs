use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};

use crate::manager::NdiManager;

/// WHEP endpoint handler: accepts an SDP offer and returns an SDP answer.
///
/// Conforms to the WHEP protocol (draft-ietf-wish-whep) for WebRTC playback.
pub async fn whep_handler(
    State(manager): State<Arc<NdiManager>>,
    body: String,
) -> Result<(StatusCode, HeaderMap, String), (StatusCode, String)> {
    let sdp_answer = manager
        .create_whep_session(body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        "application/sdp"
            .parse()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "header error".into()))?,
    );

    Ok((StatusCode::CREATED, headers, sdp_answer))
}
