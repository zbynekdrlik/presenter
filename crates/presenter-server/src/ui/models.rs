use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRow {
    pub id: String,
    pub name: String,
    pub presentation_count: usize,
    pub presentations: Vec<PresentationRow>,
    pub is_favorite: bool,
}

#[derive(Clone, Serialize)]
pub struct PresentationRow {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistRow {
    pub id: String,
    pub name: String,
    pub entries: Vec<PlaylistEntryRow>,
    #[serde(default)]
    pub show_in_dashboard: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntryRow {
    pub entry_id: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_id: Option<String>,
}
