#!/usr/bin/env python3
"""Send a test tone as VBAN audio (#852 C1a): the box check of `/listen/status`.

The packet is songplayer's header layout (crates/sp-server/src/playback/vban_packet.rs
@ dd551b7d, VB-Audio "VBAN Protocol Specifications" rev. 13), INT16 PCM:

    'VBAN' | format_SR (SR index, sub-protocol AUDIO 0x00) | nbs = frames - 1 |
    nbc = channels - 1 | format_bit 0x01 (INT16, PCM) | stream name, 16 bytes,
    ASCII, zero padded | nuFrame, u32 LE, +1 per packet | interleaved INT16 LE

Samples use songplayer's symmetric scale (+-1.0 -> +-32767, rounded half away
from zero). Packets go out in real time, `--burst` at a time (Voicemeeter
senders burst 2-4 packets, songplayer `vban-out.md`). `--skip N` leaves the
packet with counter N out: the receiver must count it as lost.

    python3 scripts/dev/listen/vban_send.py --to 10.77.9.205:6980 \
        --stream PRESENTER_TEST --seconds 5 --skip 100
"""

import argparse
import math
import socket
import struct
import sys
import time

SAMPLE_RATES = (
    6000, 12000, 24000, 48000, 96000, 192000, 384000, 8000, 16000, 32000, 64000,
    128000, 256000, 512000, 11025, 22050, 44100, 88200, 176400, 352800, 705600,
)
INT16_PCM = 0x01
NAME_LEN = 16


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


def build_packet(name, counter, rate, channels, samples):
    """One INT16 VBAN packet of interleaved `samples` (whole frames)."""
    if rate not in SAMPLE_RATES:
        raise ValueError(f"VBAN carries no {rate} Hz")
    if not 1 <= channels <= 256 or len(samples) % channels:
        raise ValueError("samples must be whole frames of 1-256 channels")
    frames = len(samples) // channels
    if not 1 <= frames <= 256:
        raise ValueError("a packet carries 1-256 frames")
    header = (
        b"VBAN"
        + bytes([SAMPLE_RATES.index(rate), frames - 1, channels - 1, INT16_PCM])
        + stream_name_bytes(name)
        + struct.pack("<I", counter & 0xFFFFFFFF)
    )
    return header + struct.pack(f"<{len(samples)}h", *(int16(x) for x in samples))


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


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--to", required=True, help="HOST:PORT of the receiver")
    parser.add_argument("--stream", required=True, help="VBAN stream name (1-16 ASCII)")
    parser.add_argument("--seconds", type=float, default=3.0)
    parser.add_argument("--tone", type=float, default=1000.0, help="Hz")
    parser.add_argument("--amplitude", type=float, default=0.5)
    parser.add_argument("--rate", type=int, default=48000)
    parser.add_argument("--channels", type=int, default=1)
    parser.add_argument("--frames", type=int, default=256, help="frames per packet")
    parser.add_argument("--burst", type=int, default=4, help="packets per send burst")
    parser.add_argument("--skip", type=int, action="append", default=[],
                        help="leave this counter out")
    args = parser.parse_args(argv)
    host, _, port = args.to.rpartition(":")
    generated = packets(args.stream, args.rate, args.channels, args.frames, args.seconds,
                        args.tone, args.amplitude, set(args.skip))
    sent = send((host, int(port)), generated, args.rate, args.frames, max(1, args.burst))
    print(f"sent {sent} VBAN packets ({args.stream}, {args.rate} Hz, {args.channels} ch, "
          f"{args.frames} frames, skipped {sorted(args.skip)}) to {args.to}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
