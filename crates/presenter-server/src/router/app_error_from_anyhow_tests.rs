//! #633: proves the centralized `From<anyhow::Error> for AppError` mapping
//! is correct BY DEFAULT for a bare `?` — no `.map_err(...)` opt-in at the
//! call site. Deliberately calls `AppError::from` directly rather than going
//! through the HTTP router: a test that only exercises a handler which
//! still happens to have explicit wiring would prove nothing about the
//! default (the whole point of this ticket). See
//! `.claude/rules/repository-error-pattern.md`.

use super::*;
use presenter_persistence::RepositoryError;

#[test]
fn not_found_maps_to_404_with_no_call_site_wiring() {
    let err: anyhow::Error = RepositoryError::NotFound("library").into();
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::NOT_FOUND);
}

#[test]
fn target_not_found_maps_to_422_with_no_call_site_wiring() {
    let err: anyhow::Error = RepositoryError::TargetNotFound("target library").into();
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn conflict_maps_to_409_with_no_call_site_wiring() {
    let err: anyhow::Error = RepositoryError::Conflict("library still tombstoned").into();
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::CONFLICT);
}

#[test]
fn repository_error_wrapped_in_context_still_maps_correctly() {
    // downcast_ref walks the whole .context() chain, so a bare `?`
    // still resolves correctly even if something upstream added
    // .context(...) between the repository and the router.
    let err: anyhow::Error =
        anyhow::Error::from(RepositoryError::NotFound("library")).context("while renaming");
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::NOT_FOUND);
}

#[test]
fn non_refusal_repository_error_variant_still_defaults_to_500() {
    // InvalidUuid is a data-integrity fault, not a client-facing
    // refusal -- it must NOT be accidentally widened to 404 by the
    // centralized mapping.
    let err: anyhow::Error = RepositoryError::InvalidUuid("not-a-uuid".to_string()).into();
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn non_repository_error_still_defaults_to_500() {
    let err = anyhow::anyhow!("boom");
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::INTERNAL_SERVER_ERROR);
}

// --- #826: a remote Bible translation (the NLT API) that cannot serve ---

#[test]
fn unreachable_remote_bible_maps_to_503_with_its_slovak_message() {
    let err: anyhow::Error = crate::bible_remote::RemoteBibleError::Timeout {
        reference: "1Jn.1.1-3".to_string(),
        timeout: std::time::Duration::from_secs(10),
    }
    .into();
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        app_err.error.to_string().starts_with("NLT nedostupné"),
        "{}",
        app_err.error
    );
}

#[test]
fn unusable_remote_bible_answer_maps_to_502_even_under_context() {
    let err: anyhow::Error = anyhow::Error::from(crate::bible_remote::RemoteBibleError::Status {
        reference: "1Jn.1.1-3".to_string(),
        status: 500,
    })
    .context("while resolving slides");
    let app_err: AppError = err.into();
    assert_eq!(app_err.status, StatusCode::BAD_GATEWAY);
    // The operator sees the NLT message, not the outer context.
    assert!(
        app_err.error.to_string().contains("HTTP 500"),
        "{}",
        app_err.error
    );
}
