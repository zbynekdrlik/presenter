"""Tests for the VBAN test sender (#852 C1a). Run:
python3 -m unittest discover -s scripts/dev/listen -p 'test_vban_send.py' -v
"""

import contextlib
import io
import math
import os
import socket
import struct
import tempfile
import unittest
import wave

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


class FakeClock:
    """A virtual clock: `sleep(d)` advances it by d plus the overshoot configured for
    that call (1-based), so a test asserts the exact schedule with no real waiting."""

    def __init__(self, start=1000.0, overshoot=None):
        self.t = start
        self.sleeps = []
        self.overshoot = overshoot or {}

    def now(self):
        return self.t

    def sleep(self, seconds):
        self.sleeps.append(seconds)
        self.t += seconds + self.overshoot.get(len(self.sleeps), 0.0)


def decode(packet):
    """(SR index, nbs, nbc, format byte, name, counter, samples) of a golden-layout packet."""
    samples = struct.unpack(f"<{(len(packet) - 28) // 2}h", packet[28:])
    return (packet[4], packet[5], packet[6], packet[7], packet[8:24].rstrip(b"\0"),
            int.from_bytes(packet[24:28], "little"), samples)


def write_wav(path, channels, rate, samples):
    """A 16-bit WAV of interleaved `samples`."""
    with wave.open(path, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))


def assert_same_samples(case, got, want):
    """Compare long sample lists without assertEqual's diff (quadratic on a mismatch)."""
    got, want = list(got), list(want)
    if got != want:
        first = next((i for i, (g, w) in enumerate(zip(got, want)) if g != w),
                     min(len(got), len(want)))
        case.fail(f"samples differ: {len(got)} vs {len(want)}, first difference at {first}")


def sine_16k(frames, hz=1000.0, rate=48000):
    return [int(16000 * math.sin(2.0 * math.pi * hz * i / rate)) for i in range(frames)]


class PacerTest(unittest.TestCase):
    def test_group_send_times_are_the_anchor_plus_the_first_frame_over_the_rate(self):
        # rate 1024, 4 x 256 frames = 1.0 s per group: every number is exact in binary.
        clock = FakeClock()
        groups = [(g * 1024, g) for g in range(6)]
        times = [clock.now() for _ in vban_send.paced(groups, 1024, clock.now, clock.sleep)]
        self.assertEqual(times, [1000.0 + k for k in range(6)])
        self.assertEqual(clock.sleeps, [1.0] * 5)  # the first group needs no sleep

    def test_the_schedule_does_not_drift_over_ten_thousand_groups_at_48k(self):
        clock = FakeClock()
        groups = [(g * 1024, g) for g in range(10000)]
        times = [clock.now() for _ in vban_send.paced(groups, 48000, clock.now, clock.sleep)]
        for k in (0, 1, 2, 4999, 9999):
            self.assertAlmostEqual(times[k], 1000.0 + k * 1024 / 48000, delta=1e-9)

    def test_the_anchor_is_taken_when_the_first_group_is_in_hand(self):
        # A slow first read (decoder start-up) moves the anchor, it is no lateness.
        clock = FakeClock()

        def slow_start():
            clock.t += 0.75
            yield 0, "a"
            yield 1024, "b"

        out = list(vban_send.paced(slow_start(), 1024, clock.now, clock.sleep))
        self.assertEqual([late for _, late in out], [0.0, 0.0])
        self.assertEqual(clock.now(), 1000.75 + 1.0)

    def test_a_sleep_that_overshoots_is_caught_up_without_re_anchoring(self):
        # The 2nd sleep (group 2, due +2) overshoots by 2.5 s: groups 3 and 4 (due +3,
        # +4) are late and go out at once, group 5 is on its ORIGINAL moment again.
        clock = FakeClock(overshoot={2: 2.5})
        groups = [(g * 1024, g) for g in range(7)]
        sent = [(clock.now(), g, late) for g, late in vban_send.paced(groups, 1024, clock.now, clock.sleep)]
        self.assertEqual([t for t, _, _ in sent],
                         [1000.0, 1001.0, 1004.5, 1004.5, 1004.5, 1005.0, 1006.0])
        self.assertEqual([late for _, _, late in sent], [0.0, 0.0, 2.5, 1.5, 0.5, 0.0, 0.0])
        self.assertTrue(all(s > 0 for s in clock.sleeps))  # behind schedule = never a sleep


class Int16PackingTest(unittest.TestCase):
    def test_int16_pcm_packs_into_the_golden_packet_unchanged(self):
        pcm = struct.pack("<4h", 0, 16384, -8192, 32767)
        packet = vban_send.build_packet_int16("GUEST_EN", 0x01020304, 48000, 1, pcm)
        self.assertEqual(packet.hex(), GOLDEN)

    def test_a_sample_already_int16_is_not_requantised(self):
        pcm = struct.pack("<3h", -32768, 1, 32767)  # -32768 has no symmetric float twin
        packet = vban_send.build_packet_int16("x", 0, 48000, 1, pcm)
        self.assertEqual(decode(packet)[6], (-32768, 1, 32767))

    def test_the_int16_path_refuses_what_build_packet_refuses(self):
        for rate, channels, pcm in ((44000, 1, b"\0\0"), (48000, 2, b"\0\0\0\0\0\0"),
                                    (48000, 1, b"\0" * 514), (48000, 1, b"\0")):
            with self.assertRaises(ValueError):
                vban_send.build_packet_int16("x", 0, rate, channels, pcm)

    def test_replay_counts_up_by_one_and_stamps_the_header_of_every_packet(self):
        # 1000 mono frames, 256 per packet: 256 + 256 + 256 + 232, two per group.
        pcm = struct.pack("<1000h", *range(1000))
        out, clock = [], FakeClock()
        result = vban_send.replay(io.BytesIO(pcm), "GUEST_EN", 48000, 1, 256, 2,
                                  out.append, clock=clock.now, sleep=clock.sleep)
        decoded = [decode(p) for p in out]
        self.assertEqual([d[5] for d in decoded], [0, 1, 2, 3])
        self.assertEqual([d[1] for d in decoded], [255, 255, 255, 231])  # nbs = frames - 1
        self.assertTrue(all(d[0] == 3 and d[2] == 0 and d[3] == 1 and d[4] == b"GUEST_EN"
                            for d in decoded))
        assert_same_samples(self, [s for d in decoded for s in d[6]], range(1000))
        self.assertEqual((result.packets, result.frames), (4, 1000))

    def test_stereo_sets_nbc_and_keeps_the_interleaving(self):
        pcm = struct.pack("<8h", 1, -1, 2, -2, 3, -3, 4, -4)
        out, clock = [], FakeClock()
        vban_send.replay(io.BytesIO(pcm), "x", 48000, 2, 256, 4, out.append,
                         clock=clock.now, sleep=clock.sleep)
        self.assertEqual(len(out), 1)
        self.assertEqual(decode(out[0])[:3], (3, 3, 1))  # 4 frames, 2 channels
        self.assertEqual(decode(out[0])[6], (1, -1, 2, -2, 3, -3, 4, -4))

    def test_skip_leaves_a_counter_out_but_keeps_its_time_slot(self):
        pcm = struct.pack("<1000h", *range(1000))
        out, clock = [], FakeClock()
        result = vban_send.replay(io.BytesIO(pcm), "x", 48000, 1, 256, 2, out.append,
                                  skip={1}, clock=clock.now, sleep=clock.sleep)
        self.assertEqual([decode(p)[5] for p in out], [0, 2, 3])
        self.assertEqual((result.packets, result.frames), (3, 1000))

    def test_groups_leave_at_the_anchor_plus_their_first_frame(self):
        # 3 groups of 2 x 256 frames at 48 kHz: first frames 0, 512, 1024.
        pcm = struct.pack("<1536h", *([0] * 1536))
        clock, stamps = FakeClock(), []
        vban_send.replay(io.BytesIO(pcm), "x", 48000, 1, 256, 2,
                         lambda p: stamps.append((decode(p)[5], clock.now())),
                         clock=clock.now, sleep=clock.sleep)
        for counter, at in stamps:
            self.assertAlmostEqual(at, 1000.0 + (counter // 2) * 512 / 48000, delta=1e-9)
        self.assertEqual([c for c, _ in stamps], [0, 1, 2, 3, 4, 5])

    def test_late_max_is_the_worst_lateness_in_seconds(self):
        pcm = struct.pack("<2048h", *([0] * 2048))  # 4 groups of 2 x 256 at 48 kHz
        clock = FakeClock(overshoot={1: 0.25})  # the first sleep (group 1) runs 0.25 s long
        result = vban_send.replay(io.BytesIO(pcm), "x", 48000, 1, 256, 2, lambda p: None,
                                  clock=clock.now, sleep=clock.sleep)
        self.assertAlmostEqual(result.late_max, 0.25, delta=1e-9)

    def test_a_stream_cut_inside_a_frame_is_refused(self):
        with self.assertRaises(vban_send.PcmTruncated):
            vban_send.replay(io.BytesIO(b"\0\0\0"), "x", 48000, 2, 256, 4, lambda p: None,
                             clock=FakeClock().now, sleep=FakeClock().sleep)


class FileReplayTest(unittest.TestCase):
    """The real ffmpeg (/usr/bin/ffmpeg here, installed by the quality job in CI): a
    missing ffmpeg FAILS these tests, it never skips them."""

    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.TemporaryDirectory()
        cls.mono = os.path.join(cls.dir.name, "tone.wav")
        cls.samples = sine_16k(24000)  # 0.5 s of 1 kHz at 48 kHz
        write_wav(cls.mono, 1, 48000, cls.samples)

    @classmethod
    def tearDownClass(cls):
        cls.dir.cleanup()

    def replay_mono(self, **kw):
        out, clock = [], FakeClock()
        result = vban_send.replay_file(self.mono, "GUEST_EN", 48000, 1, 256, 4, out.append,
                                       clock=clock.now, sleep=clock.sleep, **kw)
        return out, result, clock

    def test_a_wav_is_replayed_sample_for_sample_through_ffmpeg(self):
        out, result, clock = self.replay_mono()
        decoded = [decode(p) for p in out]
        self.assertEqual(len(out), 94)  # 93 x 256 frames + one of 192
        self.assertEqual([d[5] for d in decoded], list(range(94)))
        self.assertEqual(sum(len(d[6]) for d in decoded), 24000)
        self.assertEqual(decoded[0][6], tuple(self.samples[:256]))
        assert_same_samples(self, [s for d in decoded for s in d[6]], self.samples)
        self.assertEqual(decoded[-1][1], 191)
        self.assertEqual((decoded[0][0], decoded[0][2], decoded[0][3]), (3, 0, 1))
        self.assertEqual((result.packets, result.frames), (94, 24000))
        # 24 groups (23 x 4 packets + 2): the last leaves 23 x 1024 frames after the first.
        self.assertAlmostEqual(clock.now(), 1000.0 + 23 * 1024 / 48000, delta=1e-9)

    def test_start_and_duration_cut_the_recording(self):
        out, result, _ = self.replay_mono(start=0.25, duration=0.1)
        got = [s for p in out for s in decode(p)[6]]
        assert_same_samples(self, got, self.samples[12000:12000 + 4800])
        self.assertEqual(result.frames, 4800)

    def test_stereo_is_kept_with_channels_2_and_downmixed_with_channels_1(self):
        left, right = sine_16k(4800), sine_16k(4800, hz=440.0)
        stereo = os.path.join(self.dir.name, "stereo.wav")
        write_wav(stereo, 2, 48000, [s for pair in zip(left, right) for s in pair])
        out, clock = [], FakeClock()
        vban_send.replay_file(stereo, "x", 48000, 2, 256, 4, out.append,
                              clock=clock.now, sleep=clock.sleep)
        self.assertEqual(decode(out[0])[2], 1)  # nbc = channels - 1
        got = [s for p in out for s in decode(p)[6]]
        assert_same_samples(self, got, [s for pair in zip(left, right) for s in pair])
        mono_out = []
        vban_send.replay_file(stereo, "x", 48000, 1, 256, 4, mono_out.append,
                              clock=clock.now, sleep=clock.sleep)
        mixed = [s for p in mono_out for s in decode(p)[6]]
        self.assertEqual(len(mixed), 4800)
        for m, l, r in zip(mixed, left, right):
            self.assertLessEqual(abs(m - (l + r) / 2), 1.5)

    def test_a_rate_conversion_gives_the_audio_length_at_the_new_rate(self):
        slow = os.path.join(self.dir.name, "slow.wav")
        write_wav(slow, 1, 16000, sine_16k(8000, rate=16000))  # 0.5 s at 16 kHz
        out, clock = [], FakeClock()
        result = vban_send.replay_file(slow, "x", 16000, 1, 256, 4, out.append,
                                       clock=clock.now, sleep=clock.sleep)
        self.assertEqual(decode(out[0])[0], 8)  # SR index 8 = 16 kHz
        self.assertEqual(result.frames, 8000)

    def test_a_missing_file_is_refused_with_its_name(self):
        gone = os.path.join(self.dir.name, "gone.wav")
        with self.assertRaisesRegex(vban_send.ReplayError, "cannot read .*gone.wav"):
            vban_send.replay_file(gone, "x", 48000, 1, 256, 4, lambda p: None)

    def test_a_missing_ffmpeg_is_refused_not_skipped(self):
        with self.assertRaisesRegex(vban_send.ReplayError, "cannot run .*install ffmpeg"):
            vban_send.replay_file(self.mono, "x", 48000, 1, 256, 4, lambda p: None,
                                  ffmpeg="no-such-ffmpeg-binary")

    def test_a_file_ffmpeg_cannot_decode_exits_non_zero_with_its_message(self):
        junk = os.path.join(self.dir.name, "junk.wav")
        with open(junk, "wb") as f:
            f.write(b"this is not audio\n" * 50)
        with self.assertRaisesRegex(vban_send.ReplayError, "ffmpeg exited [1-9]"):
            vban_send.replay_file(junk, "x", 48000, 1, 256, 4, lambda p: None)

    def test_a_start_past_the_end_decodes_nothing_and_fails(self):
        with self.assertRaisesRegex(vban_send.ReplayError, "decoded no audio"):
            vban_send.replay_file(self.mono, "x", 48000, 1, 256, 4, lambda p: None, start=60.0)

    def test_the_decoder_command_line(self):
        self.assertEqual(
            vban_send.ffmpeg_command("ffmpeg", "a.wav", 120.0, 120.0, 48000, 1),
            ["ffmpeg", "-nostdin", "-v", "error", "-ss", "120.0", "-t", "120.0", "-i", "a.wav",
             "-f", "s16le", "-ac", "1", "-ar", "48000", "-"])
        self.assertEqual(
            vban_send.ffmpeg_command("ffmpeg", "a.wav", None, None, 16000, 2),
            ["ffmpeg", "-nostdin", "-v", "error", "-i", "a.wav",
             "-f", "s16le", "-ac", "2", "-ar", "16000", "-"])


class CommandLineTest(unittest.TestCase):
    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = vban_send.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def test_the_tone_mode_still_sends_with_its_defaults_resolved(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as rx:
            rx.bind(("127.0.0.1", 0))
            rx.settimeout(5.0)
            host, port = rx.getsockname()
            code, out, _ = self.run_main("--to", f"{host}:{port}", "--stream", "T",
                                         "--seconds", "0.05", "--skip", "2")
            counters = [int.from_bytes(rx.recv(2048)[24:28], "little") for _ in range(8)]
        self.assertEqual(code, 0)
        self.assertEqual(counters, [0, 1, 3, 4, 5, 6, 7, 8])
        self.assertRegex(out, r"^sent 8 VBAN packets \(T, 48000 Hz, 1 ch, 256 frames, skipped \[2\]\)")

    def test_a_wav_replays_over_udp_in_real_time_and_prints_the_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            wav = os.path.join(tmp, "tone.wav")
            samples = sine_16k(12000)  # 0.25 s
            write_wav(wav, 1, 48000, samples)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as rx:
                rx.bind(("127.0.0.1", 0))
                rx.settimeout(5.0)
                host, port = rx.getsockname()
                code, out, err = self.run_main("--to", f"{host}:{port}", "--stream",
                                               "GUEST_EN", "--file", wav, "--skip", "5")
                got = [rx.recv(2048) for _ in range(46)]  # 47 packets, counter 5 left out
        self.assertEqual((code, err), (0, ""))
        self.assertEqual([decode(p)[5] for p in got], [c for c in range(47) if c != 5])
        self.assertRegex(out, r"^replayed 46 VBAN packets \(0\.250 s of audio, GUEST_EN, "
                              r"48000 Hz, 1 ch, 256 frames, skipped \[5\], "
                              r"late_max_ms \d+\.\d\) to 127\.0\.0\.1:\d+\n$")

    def test_a_replay_failure_exits_1_with_the_reason_on_stderr(self):
        code, out, err = self.run_main("--to", "127.0.0.1:9", "--stream", "x",
                                       "--file", "/nonexistent/sermon.wav")
        self.assertEqual((code, out), (1, ""))
        self.assertRegex(err, r"vban_send: error: cannot read /nonexistent/sermon\.wav")

    def test_tone_options_and_a_file_do_not_mix(self):
        for extra in (["--tone", "440"], ["--seconds", "1"], ["--amplitude", "0.1"]):
            with self.assertRaises(SystemExit) as raised, contextlib.redirect_stderr(io.StringIO()):
                vban_send.main(["--to", "h:1", "--stream", "x", "--file", "a.wav"] + extra)
            self.assertEqual(raised.exception.code, 2)

    def test_start_and_duration_need_a_file_and_replay_geometry_is_checked(self):
        for argv in (["--start", "5"], ["--duration", "5"],
                     ["--file", "a.wav", "--channels", "3"],
                     ["--file", "a.wav", "--rate", "44000"],
                     ["--file", "a.wav", "--frames", "257"],
                     ["--file", "a.wav", "--burst", "0"],
                     ["--file", "a.wav", "--start", "-1"],
                     ["--file", "a.wav", "--duration", "0"]):
            with self.assertRaises(SystemExit) as raised, contextlib.redirect_stderr(io.StringIO()):
                vban_send.main(["--to", "h:1", "--stream", "x"] + argv)
            self.assertEqual(raised.exception.code, 2, argv)


if __name__ == "__main__":
    unittest.main()
