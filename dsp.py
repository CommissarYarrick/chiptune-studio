"""Small CPU-only chip/wavetable synthesizer. No runtime or SoundFont dependencies."""
from __future__ import annotations

import hashlib
import math
from functools import lru_cache

import numpy as np


def poly_blep(phase, step):
    out = np.zeros_like(phase)
    left = phase < step
    x = phase[left] / step[left]
    out[left] = 2 * x - x * x - 1
    right = phase > 1 - step
    x = (phase[right] - 1) / step[right]
    out[right] = x * x + 2 * x + 1
    return out


@lru_cache(maxsize=64)
def table_wave(kind, harmonics, values=()):
    size = 2048
    if kind == "triangle":
        spectrum = np.zeros(size // 2 + 1, dtype=complex)
        for k in range(1, min(harmonics, size // 2 - 1) + 1, 2):
            spectrum[k] = -1j * (-1) ** ((k - 1) // 2) / (k * k)
        table = np.fft.irfft(spectrum, size)
    else:
        # Resample the provided periodic cycle before removing harmonics above Nyquist.
        points = np.asarray(values, dtype=float)
        table = np.interp(np.arange(size) / size, np.arange(len(points) + 1) / len(points), np.r_[points, points[0]])
        spectrum = np.fft.rfft(table)
        spectrum[0] = 0
        spectrum[harmonics + 1:] = 0
        table = np.fft.irfft(spectrum, size)
    peak = np.max(np.abs(table))
    if peak > 1e-12: table /= peak
    return np.r_[table, table[0]]


def envelope(gate, release, sample_rate, adsr):
    attack, decay, sustain, _ = adsr
    t = np.arange(gate) / sample_rate
    env = np.where(t < attack, t / max(attack, 1 / sample_rate),
                   sustain + (1 - sustain) * np.exp(-5 * np.maximum(0, t - attack) / max(decay, 1 / sample_rate)))
    env[0] = 0
    if release:
        phase = np.linspace(0, 1, release, endpoint=True)
        env = np.r_[env, env[-1] * np.exp(-5 * phase) * (1 - phase)]
    return env


def voice(event, instrument, sample_rate, bpm, seed, max_samples=None):
    adsr = instrument.get("adsr", [.003, .1, .6, .04])
    gate = max(1, round(event.duration * 60 / bpm * sample_rate))
    release = round(adsr[3] * sample_rate)
    if max_samples is not None: release = max(0, min(release, max_samples - gate))
    n = gate + release
    t = np.arange(n, dtype=float) / sample_rate
    offsets = instrument.get("arpeggio", [0])
    arp = np.asarray(offsets)[np.floor(t * instrument.get("arp_hz", 24)).astype(int) % len(offsets)]
    semitones = arp.astype(float) + instrument.get("pitch_slide", 0) * np.exp(-5 * t / (instrument.get("slide_ms", 40) / 1000))
    hz, cents, delay = instrument.get("vibrato", [0, 0, 0])
    semitones += cents / 100 * np.sin(2 * np.pi * hz * t) * np.clip((t - delay) / .05, 0, 1)
    freq = np.clip(440 * 2 ** ((event.note - 69 + semitones) / 12), 10, sample_rate * .45)
    step = freq / sample_rate
    phase = np.r_[0, np.cumsum(step[:-1])] % 1
    kind = instrument["wave"]
    if kind == "pulse":
        duty = instrument.get("duty", .5)
        signal = np.where(phase < duty, 1., -1.) + poly_blep(phase, step) - poly_blep((phase - duty) % 1, step)
        signal -= 2 * duty - 1
    elif kind == "saw":
        signal = 2 * phase - 1 - poly_blep(phase, step)
    elif kind in ("triangle", "wavetable"):
        harmonics = max(1, min(512, math.floor(sample_rate * .46 / float(np.max(freq)))))
        table = table_wave(kind, harmonics, tuple(instrument.get("table", [])))
        position = phase * (len(table) - 1)
        index = position.astype(int)
        signal = table[index] * (1 - (position - index)) + table[index + 1] * (position - index)
    elif kind == "noise":
        ident = int.from_bytes(hashlib.sha256(event.uid.encode()).digest()[:8], "little")
        rng = np.random.default_rng(np.random.SeedSequence([seed, ident & 0xffffffff, ident >> 32]))
        signal = rng.uniform(-1, 1, n)
        # Two different noise contours preserve drum articulation on a single channel.
        if "hat" in event.instrument:
            signal = (signal - .8 * np.r_[0, signal[:-1]]) / 1.8
        else:
            signal = .8 * signal + .2 * np.sin(2 * np.pi * 180 * t)
    elif kind == "kick":
        signal = np.tanh(1.7 * np.sin(2 * np.pi * phase)) / np.tanh(1.7)
    else:
        signal = np.sin(2 * np.pi * phase)
    bits = instrument.get("bits", 16)
    if bits < 16:
        levels = 2 ** bits - 1
        signal = np.round((np.clip(signal, -1, 1) + 1) * .5 * levels) / levels * 2 - 1
    return (signal * envelope(gate, release, sample_rate, adsr) * event.velocity).astype(np.float32)


def add_circular(destination, signal, start, pan=0):
    gain = np.array([math.cos((pan + 1) * math.pi / 4), math.sin((pan + 1) * math.pi / 4)], dtype=np.float32)
    # Segment copies handle tails longer than a short loop without dropping samples.
    offset = 0
    while offset < len(signal):
        index = (start + offset) % len(destination)
        count = min(len(signal) - offset, len(destination) - index)
        destination[index:index + count] += signal[offset:offset + count, None] * gain
        offset += count


def track_audio(score, track, events, sample_rate):
    samples = round(score["length"] * 60 / score["bpm"] * sample_rate)
    output = np.zeros((samples, 2), dtype=np.float32)
    notes = [e for e in events if e.track == track["id"]]
    for i, event in enumerate(notes):
        start = round(event.at * 60 / score["bpm"] * sample_rate)
        max_samples = None
        if score["profile"] == "chip4":
            # A channel's next trigger takes over its previous release, including wrap.
            next_at = notes[i + 1].at if i + 1 < len(notes) else notes[0].at + score["length"]
            max_samples = max(1, round((next_at - event.at) * 60 / score["bpm"] * sample_rate))
        signal = voice(event, score["instruments"][event.instrument], sample_rate, score["bpm"], score.get("seed", 0), max_samples)
        add_circular(output, signal, start, track.get("pan", 0))
    output *= 10 ** (track.get("gain_db", -6) / 20)
    output -= np.mean(output, axis=0, keepdims=True)
    delay = score.get("delay", {})
    wet = delay.get("mix", 0)
    if wet:
        dry = output.copy()
        step = round(delay.get("beats", .75) * 60 / score["bpm"] * sample_rate)
        feedback = delay.get("feedback", .3)
        for echo in range(1, 7):
            output += np.roll(dry, step * echo, axis=0) * wet * feedback ** (echo - 1)
    return output
