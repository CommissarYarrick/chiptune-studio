"""Local monophonic humming/whistling -> editable notes and optional accompaniment."""
from __future__ import annotations

import copy
import math
import shutil
import uuid
import wave
from pathlib import Path

import numpy as np

import engine
import score as S


def read_audio(path):
    try:
        with wave.open(str(path), "rb") as stream:
            channels, width, rate, count = stream.getnchannels(), stream.getsampwidth(), stream.getframerate(), stream.getnframes()
            if channels not in (1, 2) or width not in (1, 2, 3, 4) or not 8000 <= rate <= 192000:
                raise S.ScoreError("Use mono/stereo PCM WAV, 8..192kHz")
            if not .15 <= count / rate <= 60: raise S.ScoreError("Take duration must be0.15..60seconds")
            raw = stream.readframes(count)
    except wave.Error as error: raise S.ScoreError("Use a PCM WAV recording (the studio records this format)") from error
    if len(raw) != count * channels * width: raise S.ScoreError("Truncated WAV")
    if width == 1: samples = (np.frombuffer(raw, dtype=np.uint8).astype(float) - 128) / 128
    elif width == 2: samples = np.frombuffer(raw, dtype="<i2").astype(float) / 32768
    elif width == 4: samples = np.frombuffer(raw, dtype="<i4").astype(float) / 2147483648
    else:
        b = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        v = b[:, 0] | b[:, 1] << 8 | b[:, 2] << 16
        samples = ((v ^ 0x800000) - 0x800000) / 8388608
    samples = samples.reshape(-1, channels).mean(axis=1)
    samples -= np.mean(samples)
    if rate != 16000:
        # Fourier low-pass/resampling; no external converter or model download.
        original = len(samples); size = max(1, round(original * 16000 / rate))
        spectrum = np.fft.rfft(samples)
        output_spectrum = np.zeros(size // 2 + 1, dtype=complex)
        limit = min(len(spectrum), len(output_spectrum))
        output_spectrum[:limit] = spectrum[:limit]
        samples = np.fft.irfft(output_spectrum, n=size) * size / original
    return samples.astype(np.float32), {"source_rate": rate, "channels": channels, "seconds": count / rate}


def yin(frame, rate=16000, low=65, high=2000):
    """YIN cumulative-mean normalized difference, parabolic lag refinement."""
    x = frame.astype(float) - float(np.mean(frame))
    size = len(x)
    spectrum = np.fft.rfft(x, n=2 * size)
    correlation = np.fft.irfft(spectrum * np.conj(spectrum), n=2 * size)[:size]
    squares = np.r_[0, np.cumsum(x * x)]
    lags = np.arange(1, min(size - 2, int(rate / low)) + 1)
    diff = np.maximum(0, squares[size - lags] + squares[size] - squares[lags] - 2 * correlation[lags])
    cumulative = np.cumsum(diff)
    normalized = np.ones(len(lags) + 1)
    normalized[1:] = diff * lags / np.maximum(cumulative, 1e-20)
    first, last = max(2, int(rate / high)), len(lags) - 1
    if first >= last or np.max(np.abs(x)) < 1e-7: return 0., 0.
    candidate = None
    for tau in range(first, last):
        if normalized[tau] < .15:
            while tau + 1 < last and normalized[tau + 1] < normalized[tau]: tau += 1
            candidate = tau; break
    if candidate is None: candidate = first + int(np.argmin(normalized[first:last]))
    confidence = max(0., min(1., 1 - normalized[candidate]))
    left, mid, right = normalized[candidate - 1:candidate + 2]
    denominator = left - 2 * mid + right
    correction = .5 * (left - right) / denominator if abs(denominator) > 1e-12 else 0.
    lag = candidate + float(np.clip(correction, -.5, .5))
    return float(rate / lag), confidence


def detect(samples, bpm=120, quantize=.25):
    S.number(bpm, 30, 300, "bpm"); S.number(quantize, .0625, 2, "quantize")
    frame_size, hop, rate = 1024, 160, 16000
    if len(samples) < frame_size: raise S.ScoreError("Recording is too short")
    starts = range(0, len(samples) - frame_size + 1, hop)
    energies = [float(np.sqrt(np.mean(np.square(samples[s:s + frame_size], dtype=np.float64)))) for s in starts]
    gate = max(1e-5, min(.002, float(np.percentile(energies, 20)) * 2), max(energies) * .015)
    pitches, confidences, curve = [], [], []
    for i, s in enumerate(starts):
        hz, confidence = yin(samples[s:s + frame_size]) if energies[i] >= gate else (0., 0.)
        midi = 69 + 12 * math.log2(hz / 440) if hz > 0 and confidence >= .75 else None
        pitch = round(midi) if midi is not None and 0 <= midi <= 127 else None
        pitches.append(pitch); confidences.append(confidence)
        curve.append({"seconds": (s + frame_size / 2) / rate, "hz": round(hz, 3), "confidence": round(confidence, 4)})
    # Remove isolated quantized-pitch glitches without filling voiced/unvoiced rests.
    smooth = pitches.copy()
    for i in range(1, len(pitches) - 1):
        if all(p is not None for p in pitches[i - 1:i + 2]): smooth[i] = int(np.median(pitches[i - 1:i + 2]))
        elif pitches[i] is None and pitches[i - 1] == pitches[i + 1] and pitches[i - 1] is not None: smooth[i] = pitches[i - 1]
    segments = []
    start = 0
    for i in range(1, len(smooth) + 1):
        if i < len(smooth) and smooth[i] == smooth[start]: continue
        if smooth[start] is not None and (i - start) * hop / rate >= .07:
            segments.append({"note": smooth[start], "start": max(0, (start * hop + frame_size / 2 - hop / 2) / rate),
                "end": (i * hop + frame_size / 2 - hop / 2) / rate,
                "confidence": float(np.median(confidences[start:i]))})
        start = i
    if not segments: raise S.ScoreError("No stable melody detected: record one clear voice or whistle closer to the microphone")
    offset = segments[0]["start"]
    notes = []
    for seg in segments:
        at = round((seg["start"] - offset) * bpm / 60 / quantize) * quantize
        end = round((seg["end"] - offset) * bpm / 60 / quantize) * quantize
        end = max(end, at + quantize)
        if notes and at < notes[-1]["at"] + notes[-1]["duration"]:
            if at <= notes[-1]["at"]: at = notes[-1]["at"] + notes[-1]["duration"]
            else: notes[-1]["duration"] = at - notes[-1]["at"]
            end = max(end, at + quantize)
        notes.append({"id": f"hum_{len(notes)}", "at": at, "note": seg["note"], "duration": end - at, "velocity": .82})
    return {"notes": notes, "raw_segments": segments, "pitch_curve": curve,
            "leading_seconds_trimmed": offset, "quantize_beats": quantize, "bpm": bpm,
            "median_confidence": float(np.median([s["confidence"] for s in segments])), "analysis_rate": rate,
            "warnings": ["Short notes and slides are quantized to equal-tempered notes; review the original take."]}


def estimate_key(notes):
    chroma = np.zeros(12)
    for note in notes: chroma[S.pitch(note["note"]) % 12] += note["duration"]
    major = np.array([6.35,2.23,3.48,2.33,4.38,4.09,2.52,5.19,2.39,3.66,2.29,2.88])
    minor = np.array([6.33,2.68,3.52,5.38,2.60,3.53,2.54,4.75,3.98,2.69,3.34,3.17])
    scores = []
    for mode, profile in [("major", major), ("minor", minor)]:
        profile = (profile - profile.mean()) / np.linalg.norm(profile - profile.mean())
        for root in range(12): scores.append((float(np.dot(chroma, np.roll(profile, root))), root, mode))
    scores.sort(reverse=True)
    value, root, mode = scores[0]
    return root, mode, max(0., (value - scores[1][0]) / max(abs(value), 1e-8))


def accompany(score):
    """Transparent starter harmonization. The hummed lead is never rewritten."""
    result = copy.deepcopy(score)
    notes = result["patterns"]["melody"]["notes"]
    tonic, mode, confidence = estimate_key(notes)
    intervals = [(0, 3 if mode == "minor" else 4), (5, 3 if mode == "minor" else 4), (7, 4), (8 if mode == "minor" else 9, 4 if mode == "minor" else 3)]
    candidates = [[(tonic + root) % 12, ((tonic + root) % 12 + third) % 12, (tonic + root + 7) % 12] for root, third in intervals]
    bars = math.ceil(result["length"] / 4)
    dp, back = [], []
    for bar in range(bars):
        fit = []
        for chord in candidates:
            value = 0.
            for note in notes:
                overlap = max(0, min(note["at"] + note["duration"], (bar + 1) * 4) - max(note["at"], bar * 4))
                strength = 1.5 if abs(note["at"] % 1) < 1e-6 else 1
                value += overlap * strength * (2 if S.pitch(note["note"]) % 12 in chord else -1)
            fit.append(value)
        row, prev = [], []
        for ci, chord in enumerate(candidates):
            if bar == 0: row.append(fit[ci] + (.3 if ci == 0 else 0)); prev.append(0); continue
            choices = [dp[-1][pi] - .02 * min((chord[0] - old[0]) % 12, (old[0] - chord[0]) % 12) - (.10 if ci == pi else 0) for pi, old in enumerate(candidates)]
            best = int(np.argmax(choices)); row.append(choices[best] + fit[ci]); prev.append(best)
        dp.append(row); back.append(prev)
    selected = [int(np.argmax(dp[-1]))]
    for bar in range(bars - 1, 0, -1): selected.append(back[bar][selected[-1]])
    selected.reverse()
    lead = result["tracks"][0]
    bass = {"id": "bass", "slot": "triangle", "instrument": "bass", "gain_db": -9, "pan": 0, "clips": []}
    harmony = {"id": "arpeggio" if result["profile"] == "chip4" else "harmony", "instrument": "arp" if result["profile"] == "chip4" else "warm", "gain_db": -17, "pan": .18, "clips": []}
    if result["profile"] == "chip4": harmony["slot"] = "pulse2"
    drums = {"id": "percussion", "slot": "noise", "instrument": "hat", "gain_db": -17, "pan": -.1,
             "clips": [{"id": "drums", "pattern": "rhythm", "at": 0, "repeat": bars}]}
    result["patterns"]["rhythm"] = {"length": 4, "notes": [
        {"id": f"d{i}", "at": at, "duration": .1, "note": p, "instrument": instrument, "velocity": .6}
        for i, (at, p, instrument) in enumerate([(0,36,"kick"),(1,38,"snare"),(1.75,42,"hat"),(2,36,"kick"),(3,38,"snare"),(3.75,42,"hat")])]}
    progression = []
    for bar, chosen in enumerate(selected):
        pcs = candidates[chosen]
        root = pcs[0] + 48
        third = (pcs[1] - pcs[0]) % 12
        chord = [root, root + third, root + 7]
        progression.append({"bar": bar, "root_pc": pcs[0], "third": third})
        bname, cname = f"bass_{bar}", f"harmony_{bar}"
        result["patterns"][bname] = {"length": 4, "notes": [{"id": "root", "at": 0, "note": root - 12, "duration": 1.7, "velocity": .8}, {"id": "fifth", "at": 2, "note": root - 5, "duration": 1.7, "velocity": .7}]}
        if result["profile"] == "chip4":
            row = [{"id": f"a{i}", "at": i * .5, "note": chord[degree] + 12, "duration": .32, "velocity": .65} for i, degree in enumerate([0,1,2,1,0,2,1,2])]
        else: row = [{"id": "pad", "at": 0, "note": chord, "duration": 3.7, "velocity": .55}]
        result["patterns"][cname] = {"length": 4, "notes": row}
        bass["clips"].append({"id": f"bar_{bar}", "pattern": bname, "at": bar * 4})
        harmony["clips"].append({"id": f"bar_{bar}", "pattern": cname, "at": bar * 4})
    result["tracks"] = [lead, harmony, bass, drums]
    S.validate(result)
    return result, {"tonic_pc": tonic, "mode": mode, "key_margin": confidence, "progression": progression,
                    "note": "Starter accompaniment; lead melody unchanged, key and chords editable."}


def import_take(audio_path, out, bpm=120, quantize=.25, profile="chip4", arrange=True, title="Hummed theme"):
    if profile not in S.PROFILES: raise S.ScoreError("Unknown profile")
    audio_path, out = Path(audio_path).resolve(), Path(out).resolve()
    samples, audio_info = read_audio(audio_path)
    transcription = detect(samples, bpm, quantize)
    notes = transcription["notes"]
    length = max(4, math.ceil(max(n["at"] + n["duration"] for n in notes) / 4) * 4)
    data = {"version": 1, "title": title, "profile": profile, "bpm": bpm, "meter": [4,4], "length": length,
        "seed": 731, "master_gain_db": -3, "instruments": copy.deepcopy(S.DEFAULT_BANK),
        "patterns": {"melody": {"length": length, "notes": notes}},
        "tracks": [{"id": "lead", "slot": "pulse1", "instrument": "lead", "gain_db": -7, "pan": 0,
                    "clips": [{"id": "hum", "pattern": "melody", "at": 0}]}],
        "sections": [{"id": "hummed_phrase", "at": 0, "length": length}]}
    if profile == "console8":
        data["instruments"]["lead"] = {**copy.deepcopy(S.DEFAULT_BANK["warm"]), "adsr": [.004,.14,.55,.08]}
    harmony = None
    if arrange: data, harmony = accompany(data)
    S.validate(data)
    with engine.file_lock(out.parent / ".score.lock"):
        if out.exists(): raise S.ScoreError("Import destination exists; use a new take/revision")
        saved_audio = out.parent / f"take_{uuid.uuid4().hex[:12]}.wav"
        shutil.copyfile(audio_path, saved_audio)
        metadata = {**transcription, "audio": {**audio_info, "original": str(audio_path), "saved_take": str(saved_audio),
                    "sha256": engine.file_hash(saved_audio)}, "harmony": harmony, "score": str(out)}
        metadata_path = out.with_suffix(".transcription.json")
        engine.write_json(metadata_path, metadata)
        engine.write_json(out, data)
    return {**engine.inspect(out), "transcription": metadata, "transcription_path": str(metadata_path)}
