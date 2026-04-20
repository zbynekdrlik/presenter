use crate::entities::group_color;
use anyhow::Context;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;

use super::Repository;

const AUTO_PALETTE: [&str; 20] = [
    "#FF6B6B", "#4ECDC4", "#45B7D1", "#96CEB4", "#FFEAA7", "#DDA0DD", "#98D8C8", "#F7DC6F",
    "#BB8FCE", "#85C1E9", "#F8C471", "#82E0AA", "#F1948A", "#AED6F1", "#D7BDE2", "#A3E4D7",
    "#FAD7A0", "#A9CCE3", "#D5F5E3", "#FADBD8",
];

/// FNV-1a 32-bit hash.
fn fnv1a(s: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in s.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

/// Map a group name to a deterministic palette entry.
fn generate_color(name: &str) -> &'static str {
    let idx = (fnv1a(name) as usize) % AUTO_PALETTE.len();
    AUTO_PALETTE[idx]
}

impl Repository {
    /// Load all group color rows as a `name → color` map.
    pub async fn load_all_group_colors(&self) -> anyhow::Result<HashMap<String, String>> {
        let rows = group_color::Entity::find()
            .all(&self.db)
            .await
            .context("failed to load group colors")?;
        Ok(rows.into_iter().map(|r| (r.name, r.color)).collect())
    }

    /// Return the color for `name`.  Looks up the database first; if absent,
    /// generates a deterministic color from the palette and persists it so
    /// future calls are consistent.
    pub async fn resolve_group_color(&self, name: &str) -> anyhow::Result<String> {
        if let Some(row) = group_color::Entity::find()
            .filter(group_color::Column::Name.eq(name))
            .one(&self.db)
            .await
            .context("failed to query group color")?
        {
            return Ok(row.color);
        }

        let color = generate_color(name).to_string();
        let model = group_color::ActiveModel {
            name: Set(name.to_string()),
            color: Set(color.clone()),
        };
        model
            .insert(&self.db)
            .await
            .context("failed to insert generated group color")?;

        Ok(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::Repository;

    #[test]
    fn generate_color_is_deterministic() {
        let a = generate_color("TestGroup");
        let b = generate_color("TestGroup");
        assert_eq!(a, b);
    }

    #[test]
    fn generate_color_returns_palette_entry() {
        let color = generate_color("SomeRandomName");
        assert!(
            AUTO_PALETTE.contains(&color),
            "expected a palette entry, got {color}"
        );
    }

    #[tokio::test]
    async fn load_all_group_colors_includes_seeded_data() {
        let repo = Repository::connect_in_memory().await.expect("in-memory db");
        let colors = repo
            .load_all_group_colors()
            .await
            .expect("load_all_group_colors");
        assert_eq!(
            colors.get("Vsetci").map(String::as_str),
            Some("#E08A3C"),
            "expected Vsetci = #E08A3C"
        );
        assert!(
            colors.len() >= 63,
            "expected at least 63 seeded rows, got {}",
            colors.len()
        );
    }

    #[tokio::test]
    async fn resolve_group_color_returns_legacy_for_known() {
        let repo = Repository::connect_in_memory().await.expect("in-memory db");
        let color = repo
            .resolve_group_color("Stevo")
            .await
            .expect("resolve Stevo");
        assert_eq!(color, "#D62828");
    }

    #[tokio::test]
    async fn resolve_group_color_generates_for_unknown() {
        let repo = Repository::connect_in_memory().await.expect("in-memory db");

        let unique_name = "ZZZ_UnknownGroupXYZ";
        let color1 = repo
            .resolve_group_color(unique_name)
            .await
            .expect("first resolve");
        assert!(
            AUTO_PALETTE.contains(&color1.as_str()),
            "generated color should be a palette entry"
        );

        // Second call must return the same color (persisted).
        let color2 = repo
            .resolve_group_color(unique_name)
            .await
            .expect("second resolve");
        assert_eq!(color1, color2, "second call should return same color");
    }
}
