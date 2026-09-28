use axum::{extract::State, http::StatusCode, Json};
use presenter_core::StageTextMode;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::AppError;
use crate::state::{ApiStageState, AppState};

#[instrument(skip_all)]
pub(super) async fn update_api_stage(
    State(state): State<AppState>,
    Json(payload): Json<ApiStageState>,
) -> Result<StatusCode, AppError> {
    state
        .update_api_stage(payload)
        .await
        .map_err(AppError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Body of `GET`/`PUT /stage/text-mode` (#799): `{"mode":"original"}` |
/// `"translation"` | `"both"`. An unknown mode is rejected by the typed
/// `Json` extractor (422) before the handler runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StageTextModeBody {
    pub(super) mode: StageTextMode,
}

pub(super) async fn get_stage_text_mode(State(state): State<AppState>) -> Json<StageTextModeBody> {
    Json(StageTextModeBody {
        mode: state.stage_text_mode(),
    })
}

#[instrument(skip_all)]
pub(super) async fn set_stage_text_mode(
    State(state): State<AppState>,
    Json(payload): Json<StageTextModeBody>,
) -> Json<StageTextModeBody> {
    state.set_stage_text_mode(payload.mode).await;
    Json(StageTextModeBody {
        mode: state.stage_text_mode(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::LiveEvent;
    use axum::body::Body;
    use axum::http::Request;
    use std::time::Duration;
    use tokio::time::timeout;
    use tower::ServiceExt;

    #[test]
    fn api_stage_state_accepts_the_old_payload_without_translations() {
        // Back-compat (#799): songplayer's current payload has no translation.
        let old = r#"{"currentText":"Haleluja","nextText":"Amen","currentGroup":"V1",
            "nextGroup":"C","currentSong":"Song","nextSong":"Next"}"#;
        let state: ApiStageState = serde_json::from_str(old).unwrap();
        assert_eq!(state.current_text, "Haleluja");
        assert_eq!(state.current_translation, "");
        assert_eq!(state.next_translation, "");

        let empty: ApiStageState = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.current_text, "");
    }

    #[test]
    fn api_stage_state_reads_the_translation_fields() {
        let new = r#"{"currentText":"Hallelujah","currentTranslation":"Haleluja",
            "nextText":"Amen","nextTranslation":"Nech sa stane"}"#;
        let state: ApiStageState = serde_json::from_str(new).unwrap();
        assert_eq!(state.current_translation, "Haleluja");
        assert_eq!(state.next_translation, "Nech sa stane");
    }

    async fn put_api_stage(app: &axum::Router, body: serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/stage")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn translation_reaches_the_api_ambient_snapshot_with_the_text_mode() {
        let state = AppState::in_memory().await.unwrap();
        state
            .set_stage_layout_code("api-ambient")
            .await
            .expect("api-ambient selectable");
        let mut rx = state.live_hub().subscribe();
        let app = crate::router::build_router(state.clone());

        put_api_stage(
            &app,
            serde_json::json!({
                "currentText": "Hallelujah",
                "currentTranslation": "Haleluja",
                "nextText": "Amen",
                "nextTranslation": "Nech sa stane",
            }),
        )
        .await;

        // The PUT publishes the api snapshot stamped with the SELECTED
        // api-ambient layout (displays adopt it) and the current text mode.
        let published = async {
            loop {
                if let Ok(LiveEvent::Stage { snapshot }) = rx.recv().await {
                    return snapshot;
                }
            }
        };
        let snapshot = timeout(Duration::from_millis(500), published)
            .await
            .expect("api snapshot published");
        assert_eq!(snapshot.layout.code, "api-ambient");
        assert_eq!(snapshot.text_mode, Some(StageTextMode::Both));
        let current = snapshot.current.expect("current slide");
        assert_eq!(current.main, "Hallelujah");
        assert_eq!(current.translation, "Haleluja");
        assert_eq!(
            snapshot.next.expect("next slide").translation,
            "Nech sa stane"
        );

        // GET /stage/snapshot (the reconnect resync source) serves the same.
        let served = state
            .selected_stage_display_snapshot()
            .await
            .unwrap()
            .expect("selected snapshot");
        assert_eq!(served.layout.code, "api-ambient");
        assert_eq!(served.text_mode, Some(StageTextMode::Both));
    }

    #[tokio::test]
    async fn switching_api_to_api_ambient_publishes_the_stored_snapshot_for_it() {
        // api-ambient behaves as an API layout (#799): the switch publishes
        // the STORED api snapshot stamped with the new layout, and the
        // regular resolution broadcast never overwrites it.
        let state = AppState::in_memory().await.unwrap();
        state.set_stage_layout_code("api").await.expect("api");
        state
            .update_api_stage(ApiStageState {
                current_text: "Stored line".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut rx = state.live_hub().subscribe();

        state
            .set_stage_layout_code("api-ambient")
            .await
            .expect("api-ambient");

        let published = async {
            loop {
                if let Ok(LiveEvent::Stage { snapshot }) = rx.recv().await {
                    if snapshot.layout.code != "camera-crew" {
                        return snapshot;
                    }
                }
            }
        };
        let snapshot = timeout(Duration::from_millis(500), published)
            .await
            .expect("api-ambient snapshot published on switch");
        assert_eq!(snapshot.layout.code, "api-ambient");
        assert_eq!(
            snapshot.current.map(|slide| slide.main).as_deref(),
            Some("Stored line")
        );
        assert_eq!(
            state
                .stage_display_snapshot("api-ambient")
                .await
                .unwrap()
                .map(|s| s.layout.code)
                .as_deref(),
            Some("api-ambient"),
            "explicit ?layout=api-ambient serves the api snapshot"
        );
    }

    #[tokio::test]
    async fn text_mode_endpoints_round_trip_and_reject_unknown() {
        let state = AppState::in_memory().await.unwrap();
        let app = crate::router::build_router(state.clone());

        let get = |app: axum::Router| async move {
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/stage/text-mode")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        };
        assert_eq!(get(app.clone()).await, serde_json::json!({"mode": "both"}));

        let put = |app: axum::Router, body: &'static str| async move {
            app.oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/stage/text-mode")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
        };
        assert_eq!(
            put(app.clone(), r#"{"mode":"translation"}"#).await,
            StatusCode::OK
        );
        assert_eq!(state.stage_text_mode(), StageTextMode::Translation);
        assert_eq!(
            get(app.clone()).await,
            serde_json::json!({"mode": "translation"})
        );

        assert_eq!(
            put(app.clone(), r#"{"mode":"klingon"}"#).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            state.stage_text_mode(),
            StageTextMode::Translation,
            "a rejected mode must not change the setting"
        );
    }
}
