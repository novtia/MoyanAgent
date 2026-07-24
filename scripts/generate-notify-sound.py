"""Generate the "response finished" notification sound.

A quiet two-stage mechanical click — like a pen click or a camera
shutter: a short filtered-noise "tick" with a low thock body, then a
smaller release tick ~32 ms later. Deterministic (fixed RNG seed).
Pure stdlib: writes a 16-bit mono WAV.

Output: public/sounds/notify.wav
"""

import math
import os
import random
import struct
import wave

SAMPLE_RATE = 44100
DURATION = 0.16  # seconds — mechanical clicks are short
PEAK = 0.55  # normalized peak amplitude; kept low, it should stay quiet

rng = random.Random(42)


def highpass(samples, cutoff):
    """First-order high-pass filter."""
    rc = 1.0 / (2.0 * math.pi * cutoff)
    dt = 1.0 / SAMPLE_RATE
    alpha = rc / (rc + dt)
    out = []
    prev_in = 0.0
    prev_out = 0.0
    for x in samples:
        y = alpha * (prev_out + x - prev_in)
        out.append(y)
        prev_in = x
        prev_out = y
    return out


def tick(duration, cutoff, tau, weight):
    """Short burst of high-passed noise with exponential decay."""
    n = int(SAMPLE_RATE * duration)
    noise = highpass([rng.uniform(-1.0, 1.0) for _ in range(n)], cutoff)
    return [
        weight * s * math.exp(-(i / SAMPLE_RATE) / tau)
        for i, s in enumerate(noise)
    ]


def thock(freq, duration, tau, weight):
    """Fast-decaying low sine — the body of the click."""
    n = int(SAMPLE_RATE * duration)
    out = []
    for i in range(n):
        t = i / SAMPLE_RATE
        attack = min(1.0, t / 0.001)
        out.append(
            weight * attack * math.exp(-t / tau) * math.sin(2.0 * math.pi * freq * t)
        )
    return out


def mix_into(track, offset, part):
    start = int(SAMPLE_RATE * offset)
    for i, s in enumerate(part):
        if start + i < len(track):
            track[start + i] += s


def main() -> None:
    total = int(SAMPLE_RATE * DURATION)
    track = [0.0] * total

    # Stage 1 — press: bright tick + low thock body.
    mix_into(track, 0.000, tick(0.020, 2500, 0.0018, 0.55))
    mix_into(track, 0.000, thock(190, 0.030, 0.008, 1.0))
    mix_into(track, 0.000, thock(390, 0.020, 0.005, 0.25))
    # Stage 2 — release: a smaller, duller tick.
    mix_into(track, 0.032, tick(0.015, 1800, 0.0015, 0.30))
    mix_into(track, 0.032, thock(150, 0.020, 0.006, 0.35))

    peak = max(abs(s) for s in track) or 1.0
    scale = PEAK / peak
    pcm = b"".join(
        struct.pack("<h", int(max(-1.0, min(1.0, s * scale)) * 32767))
        for s in track
    )

    out_path = os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "public",
        "sounds",
        "notify.wav",
    )
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with wave.open(out_path, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(SAMPLE_RATE)
        wav.writeframes(pcm)
    print(f"wrote {out_path} ({len(track)} frames)")


if __name__ == "__main__":
    main()
