//! Adaptive streaming tier ladder for `/ndi/mjpeg`.
//!
//! Four tiers chosen to keep text readable (floor at 720p) while degrading
//! framerate first (Resolume composed graphics don't move much).
//!
//! L0 (native): 1080p @ 30 fps  ~24 Mbps
//! L1:          1080p @ 15 fps  ~12 Mbps
//! L2:          720p  @ 15 fps  ~6 Mbps
//! L3 (floor):  720p  @ 10 fps  ~4 Mbps

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    L0,
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Copy)]
pub struct TierSpec {
    pub target_height: u32,
    pub target_fps: u32,
    pub frame_skip_modulus: u32,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::L0, Tier::L1, Tier::L2, Tier::L3];

    pub fn spec(self) -> TierSpec {
        match self {
            Tier::L0 => TierSpec {
                target_height: 1080,
                target_fps: 30,
                frame_skip_modulus: 1,
            },
            Tier::L1 => TierSpec {
                target_height: 1080,
                target_fps: 15,
                frame_skip_modulus: 2,
            },
            Tier::L2 => TierSpec {
                target_height: 720,
                target_fps: 15,
                frame_skip_modulus: 2,
            },
            Tier::L3 => TierSpec {
                target_height: 720,
                target_fps: 10,
                frame_skip_modulus: 3,
            },
        }
    }

    /// One step worse. Returns `None` at the floor.
    pub fn demote(self) -> Option<Tier> {
        match self {
            Tier::L0 => Some(Tier::L1),
            Tier::L1 => Some(Tier::L2),
            Tier::L2 => Some(Tier::L3),
            Tier::L3 => None,
        }
    }

    /// One step better. Returns `None` at native.
    pub fn promote(self) -> Option<Tier> {
        match self {
            Tier::L0 => None,
            Tier::L1 => Some(Tier::L0),
            Tier::L2 => Some(Tier::L1),
            Tier::L3 => Some(Tier::L2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l0_is_native_1080p_30fps() {
        let s = Tier::L0.spec();
        assert_eq!(s.target_height, 1080);
        assert_eq!(s.target_fps, 30);
        assert_eq!(s.frame_skip_modulus, 1);
    }

    #[test]
    fn l3_is_floor_720p_10fps() {
        let s = Tier::L3.spec();
        assert_eq!(s.target_height, 720);
        assert_eq!(s.target_fps, 10);
        assert_eq!(s.frame_skip_modulus, 3);
    }

    #[test]
    fn demote_walks_l0_to_l3_then_none() {
        assert_eq!(Tier::L0.demote(), Some(Tier::L1));
        assert_eq!(Tier::L1.demote(), Some(Tier::L2));
        assert_eq!(Tier::L2.demote(), Some(Tier::L3));
        assert_eq!(Tier::L3.demote(), None);
    }

    #[test]
    fn promote_walks_l3_to_l0_then_none() {
        assert_eq!(Tier::L3.promote(), Some(Tier::L2));
        assert_eq!(Tier::L2.promote(), Some(Tier::L1));
        assert_eq!(Tier::L1.promote(), Some(Tier::L0));
        assert_eq!(Tier::L0.promote(), None);
    }

    #[test]
    fn all_lists_every_tier() {
        assert_eq!(Tier::ALL.len(), 4);
        for t in Tier::ALL {
            // every tier round-trips through spec()
            let _ = t.spec();
        }
    }
}
