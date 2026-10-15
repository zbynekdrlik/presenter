#!/usr/bin/env python3
"""Send a test tone, or replay a recording, as VBAN audio (#852).

The tone is the box check of `/listen/status` (C1a); the replay is the input of the
live-interpreter engine evaluation.

The packet is songplayer's header layout (crates/sp-server/src/playback/vban_packet.rs
@ dd551b7d, VB-Audio "VBAN Protocol Specifications" rev. 13), INT16 PCM:

    'VBAN' | format_SR (SR index, sub-protocol AUDIO 0x00) | nbs = frames - 1 |
    nbc = channels - 1 | format_bit 0x01 (INT16, PCM) | stream name, 16 bytes,
    ASCII, zero padded | nuFrame, u32 LE, +1 per packet | interleaved INT16 LE

Tone mode (the default): samples use songplayer's symmetric scale (+-1.0 -> +-32767,
rounded half away from zero). Packets go out in real time, `--burst` at a time
(Voicemeeter senders burst 2-4 packets, songplayer `vban-out.md`). `--skip N` leaves
the packet with counter N out: the receiver must count it as lost.

    python3 scripts/dev/listen/vban_send.py --to 10.77.9.205:6980 \
        --stream PRESENTER_TEST --seconds 5 --skip 100

Replay mode (`--file`): a recording (a sermon downloaded from YouTube, any container
ffmpeg reads) goes through Presenter's own VBAN input in real time, so the engine
evaluation never waits for a live service. `ffmpeg` decodes it to raw s16le at `--rate`
/ `--channels` (`--channels 1` downmixes) and the PCM is read incrementally, never loaded
whole; the samples are already INT16, so they are packed as they are (a -32768 stays).
ONE wall-clock anchor paces the send: packet group g (`--burst` packets) leaves at
`anchor + (first frame of the group) / rate`, computed from the anchor and never from a
summed sleep; a sender that falls behind catches up without re-anchoring. The summary line
ends with `late_max_ms`, the worst lateness of a send against its schedule. A missing
ffmpeg, an unreadable file, an ffmpeg exit != 0 or a recording that decodes to nothing
exits 1 with the reason.

    python3 scripts/dev/listen/vban_send.py --to 10.77.8.134:6980 --stream GUEST_EN \
        --file sermon.wav --start 120 --duration 120

Downloaded sermon audio is copyrighted: never commit it (keep it under
`~/.claude/work-products/presenter/eval-audio/`).
"""

import argparse
import math
import socket
import struct
import subprocess
import sys
import tempfile
import time
from typing import NamedTuple

SAMPLE_RATES = (
    6000, 12000, 24000, 48000, 96000, 192000, 384000, 8000, 16000, 32000, 64000,
    128000, 256000, 512000, 11025, 22050, 44100, 88200, 176400, 352800, 705600,
)
INT16_PCM = 0x01
NAME_LEN = 16
SAMPLE_BYTES = 2


class ReplayError(Exception):
    """A replay that cannot run or did not finish: `main` prints it and exits 1."""


class PcmTruncated(ReplayError):
    """The decoded PCM ended inside a frame."""


def stream_name_bytes(name):
    """The 16-byte name field: the first 16 characters, a character that is
    not printable ASCII as `_` (songplayer `stream_name_bytes`), zero padded."""
    chars = (ord(c) if 0x20 <= ord(c) < 0x7F else ord("_") for c in name[:NAME_LEN])
    return bytes(chars).ljust(NAME_LEN, b"\0")


def int16(x):
    """One sample as INT16: NaN is silence, clamped, x 32767, half away from 0."""
    if math.isnan(x):
        return 0
    v = max(-1.0, min(1.0, x)) * 32767.0
    return int(math.floor(v + 0.5)) if v >= 0 else -int(math.floor(-v + 0.5))


def _header(name, counter, rate, channels, sample_count):
    """The 28-byte header of a packet of `sample_count` interleaved samples."""
    if rate not in SAMPLE_RATES:
        raise ValueError(f"VBAN carries no {rate} Hz")
    if not 1 <= channels <= 256 or sample_count % channels:
        raise ValueError("samples must be whole frames of 1-256 channels")
    frames = sample_count // channels
    if not 1 <= frames <= 256:
        raise ValueError("a packet carries 1-256 frames")
    return (
        b"VBAN"
        + bytes([SAMPLE_RATES.index(rate), frames - 1, channels - 1, INT16_PCM])
        + stream_name_bytes(name)
        + struct.pack("<I", counter & 0xFFFFFFFF)
    )


def build_packet(name, counter, rate, channels, samples):
    """One INT16 VBAN packet of interleaved `samples` (whole frames)."""
    header = _header(name, counter, rate, channels, len(samples))
    return header + struct.pack(f"<{len(samples)}h", *(int16(x) for x in samples))


def build_packet_int16(name, counter, rate, channels, pcm):
    """One VBAN packet of `pcm` = interleaved s16le bytes (whole frames), packed as
    they are: the replay path never re-quantises a sample that is already INT16."""
    if len(pcm) % SAMPLE_BYTES:
        raise ValueError("INT16 PCM is whole 2-byte samples")
    return _header(name, counter, rate, channels, len(pcm) // SAMPLE_BYTES) + bytes(pcm)


def tone(rate, hz, amplitude, start_frame, frames, channels):
    """`frames` frames of a sine starting at `start_frame`, the same in every channel."""
    return [
        amplitude * math.sin(2.0 * math.pi * hz * (start_frame + i) / rate)
        for i in range(frames)
        for _ in range(channels)
    ]


def packets(name, rate, channels, frames, seconds, hz, amplitude, skip=()):
    """Yield (counter, packet) for `seconds` of tone; counters in `skip` are left out."""
    for counter in range(int(rate * seconds) // frames):
        if counter in skip:
            continue
        samples = tone(rate, hz, amplitude, counter * frames, frames, channels)
        yield counter, build_packet(name, counter, rate, channels, samples)


def send(addr, generated, rate, frames, burst):
    """Send in real time, `burst` packets at a time; returns the count sent."""
    sent = 0
    start = time.monotonic()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        for counter, packet in generated:
            sock.sendto(packet, addr)
            sent += 1
            if sent % burst == 0:
                due = start + (counter + 1) * frames / rate
                time.sleep(max(0.0, due - time.monotonic()))
    return sent


# ── file replay ───────────────────────────────────────────────────────────────


class Replay(NamedTuple):
    """What a replay did: packets sent (a skipped counter is not one), frames read
    (so audio seconds = frames / rate) and the worst lateness of a send, in seconds."""

    packets: int
    frames: int
    late_max: float


def pcm_groups(stream, frames, burst, channels):
    """Yield (first frame index, [one s16le chunk per packet]) per group of `burst`
    packets read from `stream`; the last packet of a recording may be shorter.
    `stream.read(n)` returns n bytes until EOF (a BufferedReader / BytesIO)."""
    frame_bytes = SAMPLE_BYTES * channels
    packet_bytes = frames * frame_bytes
    first = 0
    while True:
        chunk = stream.read(packet_bytes * burst)
        if not chunk:
            return
        if len(chunk) % frame_bytes:
            raise PcmTruncated(f"the decoded PCM ended inside a frame ({len(chunk)} bytes)")
        yield first, [chunk[i:i + packet_bytes] for i in range(0, len(chunk), packet_bytes)]
        first += len(chunk) // frame_bytes


def paced(groups, rate, clock=time.monotonic, sleep=time.sleep):
    """Yield (payload, lateness s) of each (first frame, payload) group at its moment.

    ONE anchor, taken when the first group is in hand (the decoder's start-up never
    counts as lateness): group g is due at `anchor + first_frame / rate` — computed from
    the anchor, never from a summed sleep, so an hour of audio does not drift. A caller
    behind schedule gets the next groups at once (no sleep) and the schedule is never
    re-anchored: it is back on time as soon as it has caught up."""
    anchor = None
    for first_frame, payload in groups:
        if anchor is None:
            anchor = clock()
        due = anchor + first_frame / rate
        delay = due - clock()
        if delay > 0:
            sleep(delay)
        yield payload, max(0.0, clock() - due)


def replay(stream, name, rate, channels, frames, burst, send_packet, skip=(),
           clock=time.monotonic, sleep=time.sleep):
    """Send the s16le PCM of `stream` as VBAN packets paced in real time; the counter
    starts at 0 and a counter in `skip` is left out (it still takes its time slot)."""
    counter = sent = total_frames = 0
    late_max = 0.0
    groups = pcm_groups(stream, frames, burst, channels)
    for payload, late in paced(groups, rate, clock, sleep):
        late_max = max(late_max, late)
        for pcm in payload:
            if counter not in skip:
                send_packet(build_packet_int16(name, counter, rate, channels, pcm))
                sent += 1
            counter += 1
            total_frames += len(pcm) // (SAMPLE_BYTES * channels)
    return Replay(sent, total_frames, late_max)


def ffmpeg_command(ffmpeg, path, start, duration, rate, channels):
    """The decoder command line: the recording as raw s16le on stdout."""
    cmd = [ffmpeg, "-nostdin", "-v", "error"]
    if start is not None:
        cmd += ["-ss", str(start)]
    if duration is not None:
        cmd += ["-t", str(duration)]
    return cmd + ["-i", path, "-f", "s16le", "-ac", str(channels), "-ar", str(rate), "-"]


class FfmpegPcm:
    """ffmpeg decoding `path` to s16le on a pipe: `with FfmpegPcm(...) as pcm:` gives the
    stream, `finish()` after its EOF turns an ffmpeg exit != 0 into a ReplayError, and
    leaving the block kills a decoder that is still running (a failed send)."""

    def __init__(self, path, start, duration, rate, channels, ffmpeg="ffmpeg"):
        self.command = ffmpeg_command(ffmpeg, path, start, duration, rate, channels)
        self.path = path
        self.proc = None
        self.errors = None

    def __enter__(self):
        try:
            open(self.path, "rb").close()
        except OSError as e:
            raise ReplayError(f"cannot read {self.path}: {e.strerror or e}") from e
        # ffmpeg's stderr goes to a file, not a pipe: nobody reads a pipe while the
        # PCM is paced, so a chatty decoder could otherwise fill it and stall.
        self.errors = tempfile.TemporaryFile()
        try:
            self.proc = subprocess.Popen(
                self.command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                stderr=self.errors)
        except OSError as e:
            self.errors.close()
            raise ReplayError(
                f"cannot run {self.command[0]}: {e.strerror or e} (install ffmpeg)") from e
        return self.proc.stdout

    def finish(self):
        """After the PCM stream's EOF: ffmpeg must have exited 0."""
        code = self.proc.wait()
        if code != 0:
            self.errors.seek(0)
            text = self.errors.read().decode("utf-8", "replace").strip()
            raise ReplayError(f"ffmpeg exited {code} decoding {self.path}: {text or 'no message'}")

    def __exit__(self, *_):
        if self.proc is not None:
            self.proc.stdout.close()
            if self.proc.poll() is None:
                self.proc.kill()
            self.proc.wait()
            self.errors.close()
        return False


def replay_file(path, name, rate, channels, frames, burst, send_packet, skip=(),
                start=None, duration=None, ffmpeg="ffmpeg",
                clock=time.monotonic, sleep=time.sleep):
    """Decode `path` with ffmpeg and replay it (see `replay`); returns a Replay."""
    decoder = FfmpegPcm(path, start, duration, rate, channels, ffmpeg)
    with decoder as pcm:
        try:
            result = replay(pcm, name, rate, channels, frames, burst, send_packet, skip,
                            clock, sleep)
        except PcmTruncated:
            decoder.finish()  # a dead ffmpeg explains a cut stream better than the cut does
            raise
        decoder.finish()
    if result.frames == 0:
        raise ReplayError(f"ffmpeg decoded no audio from {path} (--start past its end?)")
    return result


# ── command line ──────────────────────────────────────────────────────────────


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--to", required=True, help="HOST:PORT of the receiver")
    parser.add_argument("--stream", required=True, help="VBAN stream name (1-16 ASCII)")
    tone_group = parser.add_argument_group("tone mode (default, no --file)")
    tone_group.add_argument("--seconds", type=float, help="length (default 3)")
    tone_group.add_argument("--tone", type=float, help="Hz (default 1000)")
    tone_group.add_argument("--amplitude", type=float, help="0-1 (default 0.5)")
    file_group = parser.add_argument_group("replay mode")
    file_group.add_argument("--file", help="recording to replay in real time (needs ffmpeg)")
    file_group.add_argument("--start", type=float, help="seconds into the file to start at")
    file_group.add_argument("--duration", type=float, help="seconds of audio to replay")
    parser.add_argument("--rate", type=int, default=48000)
    parser.add_argument("--channels", type=int, default=1,
                        help="1 or 2 with --file (1 = downmix); default 1")
    parser.add_argument("--frames", type=int, default=256, help="frames per packet")
    parser.add_argument("--burst", type=int, default=4, help="packets per send burst")
    parser.add_argument("--skip", type=int, action="append", default=[],
                        help="leave this counter out")
    return parser


def check_modes(parser, args):
    """Refuse a mix of tone and replay options, and a replay VBAN cannot carry."""
    if args.file is None:
        if args.start is not None or args.duration is not None:
            parser.error("--start / --duration need --file")
        return
    if args.seconds is not None or args.tone is not None or args.amplitude is not None:
        parser.error("--file replays a recording: --seconds / --tone / --amplitude are "
                     "for the tone mode")
    if args.rate not in SAMPLE_RATES:
        parser.error(f"VBAN carries no {args.rate} Hz")
    if args.channels not in (1, 2):
        parser.error("--channels must be 1 or 2 with --file")
    if not 1 <= args.frames <= 256:
        parser.error("--frames must be 1-256")
    if args.burst < 1:
        parser.error("--burst must be at least 1")
    if args.start is not None and args.start < 0:
        parser.error("--start must not be negative")
    if args.duration is not None and args.duration <= 0:
        parser.error("--duration must be positive")


def run_tone(args, addr):
    seconds = 3.0 if args.seconds is None else args.seconds
    hz = 1000.0 if args.tone is None else args.tone
    amplitude = 0.5 if args.amplitude is None else args.amplitude
    generated = packets(args.stream, args.rate, args.channels, args.frames, seconds,
                        hz, amplitude, set(args.skip))
    sent = send(addr, generated, args.rate, args.frames, max(1, args.burst))
    print(f"sent {sent} VBAN packets ({args.stream}, {args.rate} Hz, {args.channels} ch, "
          f"{args.frames} frames, skipped {sorted(args.skip)}) to {args.to}")


def run_file(args, addr):
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        result = replay_file(
            args.file, args.stream, args.rate, args.channels, args.frames, args.burst,
            lambda packet: sock.sendto(packet, addr), set(args.skip),
            start=args.start, duration=args.duration)
    print(f"replayed {result.packets} VBAN packets ({result.frames / args.rate:.3f} s of audio, "
          f"{args.stream}, {args.rate} Hz, {args.channels} ch, {args.frames} frames, "
          f"skipped {sorted(args.skip)}, late_max_ms {result.late_max * 1000.0:.1f}) "
          f"to {args.to}")


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    check_modes(parser, args)
    host, _, port = args.to.rpartition(":")
    addr = (host, int(port))
    if args.file is None:
        run_tone(args, addr)
        return 0
    try:
        run_file(args, addr)
    except ReplayError as e:
        print(f"vban_send: error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
