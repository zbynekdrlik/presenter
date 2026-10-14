//! The frame-counter rule of one VBAN stream (#852 C1a, spec §5.3). Pure.
//!
//! - the expected counter → use the packet;
//! - ahead by packets totalling ≤ 1 s → that many were lost: insert their
//!   length of silence, then use the packet;
//! - behind by ≤ 1 s (late or a duplicate) → drop it;
//! - a jump of more than 1 s either way (a sender restart) → use it and start
//!   counting from it.
//!
//! "1 s" is counted in packets of the CURRENT packet's length (`rate / frames`:
//! 187 packets of 256 frames at 48 kHz). The counter wraps at `u32::MAX`
//! (songplayer's `VbanEncoder`), so all arithmetic is wrapping. No jitter
//! buffer: the ASR has no playout clock, and Voicemeeter's bursts of 2–4
//! packets are normal (songplayer `vban-out.md`).

/// What to do with one packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceVerdict {
    /// The stream's first packet, or the expected one.
    InOrder,
    /// `missing` packets were lost; `silence_frames` of silence go before this one.
    Gap { missing: u32, silence_frames: usize },
    /// Late or a duplicate: dropped.
    Late,
    /// A jump of more than a second (a sender restart): used, counting starts over.
    Resync,
}

/// One stream's expected counter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamSequencer {
    expected: Option<u32>,
}

impl StreamSequencer {
    /// The verdict for a packet with `counter` carrying `frames` frames at `rate_hz`.
    pub fn observe(&mut self, counter: u32, frames: usize, rate_hz: u32) -> SequenceVerdict {
        let Some(expected) = self.expected else {
            self.expected = Some(counter.wrapping_add(1));
            return SequenceVerdict::InOrder;
        };
        let second = packets_per_second(frames, rate_hz);
        let ahead = counter.wrapping_sub(expected);
        if ahead == 0 {
            self.expected = Some(counter.wrapping_add(1));
            return SequenceVerdict::InOrder;
        }
        if ahead <= second {
            self.expected = Some(counter.wrapping_add(1));
            return SequenceVerdict::Gap {
                missing: ahead,
                silence_frames: ahead as usize * frames,
            };
        }
        if expected.wrapping_sub(counter) <= second {
            return SequenceVerdict::Late;
        }
        self.expected = Some(counter.wrapping_add(1));
        SequenceVerdict::Resync
    }

    /// Forget the stream (a new sender): the next packet is a first packet.
    pub fn reset(&mut self) {
        self.expected = None;
    }
}

/// Whole packets of `frames` in one second at `rate_hz` (at least 1).
fn packets_per_second(frames: usize, rate_hz: u32) -> u32 {
    let frames = u32::try_from(frames.max(1)).unwrap_or(u32::MAX);
    (rate_hz / frames).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use SequenceVerdict::{Gap, InOrder, Late, Resync};

    const RATE: u32 = 48_000;
    /// 187 packets per second (the Python mirror: 48000 // 256).
    const FRAMES: usize = 256;

    fn run(counters: &[u32]) -> Vec<SequenceVerdict> {
        let mut sequencer = StreamSequencer::default();
        counters
            .iter()
            .map(|&c| sequencer.observe(c, FRAMES, RATE))
            .collect()
    }

    #[test]
    fn consecutive_counters_are_in_order() {
        assert_eq!(run(&[7, 8, 9]), vec![InOrder; 3]);
    }

    #[test]
    fn a_gap_inserts_the_missing_packets_as_silence() {
        assert_eq!(
            run(&[0, 1, 4]),
            vec![
                InOrder,
                InOrder,
                Gap {
                    missing: 2,
                    silence_frames: 512
                }
            ]
        );
    }

    #[test]
    fn exactly_one_second_ahead_is_still_a_gap_and_one_more_is_a_resync() {
        assert_eq!(
            run(&[0, 188]),
            vec![
                InOrder,
                Gap {
                    missing: 187,
                    silence_frames: 187 * 256
                }
            ]
        );
        assert_eq!(run(&[0, 189]), vec![InOrder, Resync]);
        assert_eq!(
            run(&[0, 189, 190]),
            vec![InOrder, Resync, InOrder],
            "counting goes on"
        );
    }

    #[test]
    fn late_and_duplicate_packets_are_dropped_without_moving_the_expectation() {
        assert_eq!(
            run(&[0, 1, 2, 2, 1, 3]),
            vec![InOrder, InOrder, InOrder, Late, Late, InOrder]
        );
    }

    #[test]
    fn a_second_behind_is_late_and_more_is_a_resync() {
        // expected 300 after 299: 300 − 187 = 113 is late, 112 is a restart.
        assert_eq!(run(&[299, 113]), vec![InOrder, Late]);
        assert_eq!(run(&[299, 112, 113]), vec![InOrder, Resync, InOrder]);
    }

    #[test]
    fn the_counter_wraps_at_u32_max() {
        assert_eq!(run(&[u32::MAX - 1, u32::MAX, 0, 1]), vec![InOrder; 4]);
        // the Python mirror: (1 − (2**32 − 2)) % 2**32 == 3
        assert_eq!(
            run(&[u32::MAX - 2, 1]),
            vec![
                InOrder,
                Gap {
                    missing: 3,
                    silence_frames: 768
                }
            ]
        );
        assert_eq!(
            run(&[1, u32::MAX]),
            vec![InOrder, Late],
            "behind across the wrap"
        );
    }

    #[test]
    fn one_second_is_counted_in_packets_of_the_current_length() {
        let mut sequencer = StreamSequencer::default();
        assert_eq!(sequencer.observe(0, 64, 48_000), InOrder);
        assert_eq!(
            sequencer.observe(751, 64, 48_000),
            Gap {
                missing: 750,
                silence_frames: 48_000
            },
            "48000 / 64 = 750 packets"
        );
        assert_eq!(packets_per_second(256, 44_100), 172);
        assert_eq!(
            packets_per_second(0, 48_000),
            48_000,
            "0 frames counts as 1"
        );
    }

    #[test]
    fn reset_makes_the_next_packet_a_first_packet() {
        let mut sequencer = StreamSequencer::default();
        sequencer.observe(5, FRAMES, RATE);
        sequencer.reset();
        assert_eq!(sequencer.observe(9_000, FRAMES, RATE), InOrder);
        assert_eq!(sequencer.observe(9_001, FRAMES, RATE), InOrder);
    }
}
