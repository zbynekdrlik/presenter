const GROUP_COLORS: [&str; 8] = [
    "#fb7185", // rose
    "#fb923c", // orange
    "#fbbf24", // amber
    "#34d399", // emerald
    "#22d3ee", // cyan
    "#60a5fa", // blue
    "#a78bfa", // violet
    "#f472b6", // pink
];

pub fn group_color(name: &str) -> &'static str {
    let hash = fnv1a(name);
    GROUP_COLORS[(hash as usize) % GROUP_COLORS.len()]
}

fn fnv1a(s: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in s.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

pub fn hex_to_rgba(hex: &str, opacity: f32) -> String {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    format!("rgba({r},{g},{b},{opacity})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_color_deterministic() {
        assert_eq!(group_color("Verse 1"), group_color("Verse 1"));
        assert_eq!(group_color("Chorus"), group_color("Chorus"));
    }

    #[test]
    fn group_color_returns_valid_color() {
        let c = group_color("Bridge");
        assert!(GROUP_COLORS.contains(&c));
    }

    #[test]
    fn group_color_empty_string() {
        let c = group_color("");
        assert!(GROUP_COLORS.contains(&c));
    }

    #[test]
    fn hex_to_rgba_with_hash() {
        assert_eq!(hex_to_rgba("#fb7185", 0.25), "rgba(251,113,133,0.25)");
    }

    #[test]
    fn hex_to_rgba_without_hash() {
        assert_eq!(hex_to_rgba("60a5fa", 0.5), "rgba(96,165,250,0.5)");
    }
}
