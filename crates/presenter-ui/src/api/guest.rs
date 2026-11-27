//! The guest translation page's reads (#999): the offered languages with this
//! box's status, and the QR code of the guest page.

use presenter_core::listen::guest::GuestLanguages;
use serde::Deserialize;

use super::{get_json, ApiError};

pub async fn languages() -> Result<GuestLanguages, ApiError> {
    get_json("/listen/guest/languages").await
}

/// `GET /listen/guest/qr`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GuestQr {
    pub url: String,
    /// The address is a LAN one: it does not work on mobile data.
    pub local: bool,
    pub svg: String,
}

pub async fn qr() -> Result<GuestQr, ApiError> {
    get_json("/listen/guest/qr").await
}
