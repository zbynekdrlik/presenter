"""Tests for the VBAN test sender (#852 C1a). Run:
python3 -m unittest discover -s scripts/dev/listen -p 'test_vban_send.py' -v
"""

import math
import socket
import unittest

import vban_send

# The same packet `golden_packet_decodes_and_the_encoder_rebuilds_it` pins in
# crates/presenter-core/src/audio/vban/tests.rs: INT16 mono 48 kHz, 4 frames
# [0.0, 0.5, -0.25, 1.5], stream GUEST_EN, counter 0x01020304.
GOLDEN = (
    "5642414e"  # VBAN
    "03030001"  # SR index 3 (48 kHz), 4 frames, 1 channel, INT16 PCM
    "47554553545f454e"  # "GUEST_EN"
    "0000000000000000"  # name padding
    "04030201"  # counter 0x01020304
    "0000004000e0ff7f"  # 0, 16384, -8192, 32767
)


class VbanSendTest(unittest.TestCase):
    def test_golden_packet_matches_the_rust_decoder_test(self):
        packet = vban_send.build_packet("GUEST_EN", 0x01020304, 48000, 1, [0.0, 0.5, -0.25, 1.5])
        self.assertEqual(packet.hex(), GOLDEN)

    def test_header_fields_of_a_256_frame_mono_packet(self):
        packet = vban_send.build_packet("PRESENTER_TEST", 7, 48000, 1, [0.0] * 256)
        self.assertEqual(len(packet), 28 + 512)
        self.assertEqual(packet[:4], b"VBAN")
        self.assertEqual((packet[4], packet[5], packet[6], packet[7]), (3, 255, 0, 0x01))
        self.assertEqual(packet[8:24], b"PRESENTER_TEST\0\0")
        self.assertEqual(packet[24:28], (7).to_bytes(4, "little"))

    def test_the_sr_index_follows_the_spec_list(self):
        for rate, index in ((48000, 3), (96000, 4), (192000, 5), (44100, 16), (88200, 17), (16000, 8)):
            self.assertEqual(vban_send.build_packet("x", 0, rate, 1, [0.0])[4], index)

    def test_the_stream_name_is_16_ascii_bytes_zero_padded(self):
        self.assertEqual(vban_send.stream_name_bytes("sp-program"), b"sp-program\0\0\0\0\0\0")
        self.assertEqual(vban_send.stream_name_bytes("a-very-long-stream-name"), b"a-very-long-stre")
        self.assertEqual(vban_send.stream_name_bytes("é\tx"), b"__x" + b"\0" * 13)

    def test_int16_conversion_matches_songplayer(self):
        self.assertEqual(vban_send.int16(0.5), 16384)
        self.assertEqual(vban_send.int16(-0.25), -8192)
        self.assertEqual(vban_send.int16(1.5), 32767)
        self.assertEqual(vban_send.int16(-1.0), -32767)
        self.assertEqual(vban_send.int16(math.nan), 0)

    def test_skip_leaves_its_counter_out(self):
        counters = [c for c, _ in vban_send.packets("x", 48000, 1, 256, 0.1, 1000.0, 0.5, {3})]
        self.assertEqual(counters, [0, 1, 2] + list(range(4, 18)))

    def test_the_counter_wraps_at_u32_max(self):
        packet = vban_send.build_packet("x", 2**32 + 5, 48000, 1, [0.0])
        self.assertEqual(packet[24:28], (5).to_bytes(4, "little"))

    def test_bad_geometry_is_refused(self):
        with self.assertRaises(ValueError):
            vban_send.build_packet("x", 0, 44000, 1, [0.0])
        with self.assertRaises(ValueError):
            vban_send.build_packet("x", 0, 48000, 1, [0.0] * 257)
        with self.assertRaises(ValueError):
            vban_send.build_packet("x", 0, 48000, 2, [0.0] * 3)

    def test_packets_go_out_over_real_udp_and_skip_their_counter(self):
        # A loopback receiver on an ephemeral port (never 6980): 0.1 s at 48 kHz
        # in 256-frame packets is 18 counters, counter 10 left out = 17 datagrams.
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as rx:
            rx.bind(("127.0.0.1", 0))
            rx.settimeout(5.0)
            generated = vban_send.packets("PRESENTER_TEST", 48000, 1, 256, 0.1, 1000.0, 0.5, {10})
            sent = vban_send.send(rx.getsockname(), generated, 48000, 256, 4)
            counters = [int.from_bytes(rx.recv(2048)[24:28], "little") for _ in range(sent)]
        self.assertEqual(sent, 17)
        self.assertEqual(counters, [c for c in range(18) if c != 10])


if __name__ == "__main__":
    unittest.main()
