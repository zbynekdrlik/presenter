//! The guest translation page's reads (#999): the offered languages with this
//! box's status, and the QR code of the guest page.

use presenter_core::listen::guest::GuestLanguages;
use serde::Deserialize;

use super::{get_json, ApiError};

pub async fn languages() -> Result<GuestLanguages, ApiError> {
    get_json("/listen/guest/languages").await
}

/// `GET /listen/guest/qr`: `https://<guest host>/` and its QR code; both
/// `None` on a box without a guest hostname.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GuestQr {
    pub url: Option<String>,
    pub svg: Option<String>,
}

pub async fn qr() -> Result<GuestQr, ApiError> {
    get_json("/listen/guest/qr").await
}
