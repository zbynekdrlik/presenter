//! Cross-instance song identity (#555). One canonical implementation used by the
//! importer (from the `.pro` UUID), the create path, the LWW apply path, and the
//! backfill migration — so two instances derive identical `sync_id`s with zero
//! coordination. NEVER change the namespace or the name format without a data
//! migration: it would re-pair every existing song.
use uuid::Uuid;

/// Fixed project namespace for song sync ids. A stable random UUID — do not change.
pub const SYNC_ID_NAMESPACE: Uuid = Uuid::from_u128(0x9f1c7a2e_5b34_4d8e_9a11_6c2f0b7d4e15);

/// Deterministic `sync_id` for a song that has no `.pro` UUID: UUIDv5 over
/// `"<library_name>/<name>"`. Both sites compute this identically for the same
/// repertoire, so existing identical songs pair up automatically.
pub fn sync_id_for_name(library_name: &str, name: &str) -> String {
    let key = format!("{library_name}/{name}");
    Uuid::new_v5(&SYNC_ID_NAMESPACE, key.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_id() {
        assert_eq!(
            sync_id_for_name("Songs", "Amazing Grace"),
            sync_id_for_name("Songs", "Amazing Grace"),
        );
    }

    #[test]
    fn different_library_or_name_differs() {
        let base = sync_id_for_name("Songs", "Amazing Grace");
        assert_ne!(base, sync_id_for_name("Hymns", "Amazing Grace"));
        assert_ne!(base, sync_id_for_name("Songs", "Amazing Grace 2"));
    }
}
