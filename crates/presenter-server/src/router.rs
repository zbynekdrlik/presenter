use crate::state::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/libraries", get(list_libraries))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn list_libraries(State(state): State<AppState>) -> impl IntoResponse {
    let libraries = state.libraries().await;
    Json(libraries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use presenter_core::Library;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_endpoint_returns_ok() {
        let app = build_router(AppState::seeded());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn libraries_endpoint_returns_seed() {
        let app = build_router(AppState::seeded());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Vec<Library> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].presentations.len(), 1);
    }
}
