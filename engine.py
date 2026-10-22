"""File-based agent API: inspect, revise, render, export and compare."""
from __future__ import annotations

import contextlib
import copy
import hashlib
import json
import math
import os
import struct
import time
import uuid
import wave
from pathlib import Path

import numpy as np

import dsp
import score as S

VERSION = "1.0.0"
HERE = Path(__file__).resolve().parent


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""): digest.update(block)
    return digest.hexdigest()


def code_hash():
    digest = hashlib.sha256()
    for path in sorted(HERE.glob("*.py")):
        digest.update(path.name.encode()); digest.update(path.read_bytes())
    return digest.hexdigest()


def write_json(path, data):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        temp.write_text(json.dumps(data, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")
        os.replace(temp, path)
    finally:
        if temp.exists(): temp.unlink()


@contextlib.contextmanager
def file_lock(path):
    """OS releases the lock even when an agent process dies."""
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as stream:
        if stream.tell() == 0: stream.write(b"0"); stream.flush()
        stream.seek(0)
        try:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error: raise S.ScoreError(f"Busy: {path.parent}; another render/edit is active") from error
        try: yield
        finally:
            if os.name == "nt":
                stream.seek(0); msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else: fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def create(path, profile="chip4", title="Lantern Circuit", template="lanterns"):
    if profile not in S.PROFILES: raise S.ScoreError("Unknown profile")
    if template not in {"lanterns", "empty"}: raise S.ScoreError("template must be lanterns or empty")
    path = Path(path)
    with file_lock(path.parent / ".score.lock"):
        if path.exists(): raise S.ScoreError("Score already exists; use patch to create a revision")
        data = json.loads((HERE / "examples" / f"{profile}.json").read_text(encoding="utf-8"))
        data["title"] = title
        if template == "empty":
            data["length"] = 16; data["patterns"] = {"melody": {"length": 16, "notes": []}}
            data["tracks"] = [{"id": "lead", "slot": "pulse1", "instrument": "lead", "gain_db": -7, "pan": 0,
                               "clips": [{"id": "melody", "pattern": "melody", "at": 0}]}]
            data["sections"] = [{"id": "A", "at": 0, "length": 16}]
        S.validate(data); write_json(path, data)
    return inspect(path)


def inspect(path):
    data = S.load(path)
    return {"path": str(Path(path).resolve()), **S.validate(data), "score": data,
            "tracks": [{"id": t["id"], "instrument": t["instrument"], "clips": len(t["clips"])} for t in data["tracks"]]}


def patch(path, operations, out, expected_sha256):
    path, out = Path(path), Path(out)
    with file_lock(out.parent / ".score.lock"):
        data = S.load(path)
        if S.digest(data) != expected_sha256: raise S.ScoreError("Score changed; inspect again before patching")
        result = S.apply_patch(data, operations)
        if out.exists() and out.resolve() != path.resolve(): raise S.ScoreError("Revision destination exists")
        write_json(out, result)
    return inspect(out)


def write_wave(path, audio, gain=1, sample_rate=32000):
    with wave.open(str(path), "wb") as stream:
        stream.setnchannels(2); stream.setsampwidth(2); stream.setframerate(sample_rate)
        for start in range(0, len(audio), 32768):
            block = np.asarray(audio[start:start + 32768]) * gain
            if not np.isfinite(block).all(): raise S.ScoreError("Nonfinite audio")
            stream.writeframesraw(np.rint(np.clip(block, -1, 1) * 32767).astype("<i2").tobytes())


def metrics(audio):
    peak = float(max(abs(float(audio.min())), abs(float(audio.max())))) if len(audio) else 0.
    sum_squares = 0.
    for start in range(0, len(audio), 32768): sum_squares += float(np.sum(np.square(audio[start:start + 32768], dtype=np.float64)))
    rms = math.sqrt(sum_squares / max(1, audio.size))
    indexes = np.arange(1, len(audio), max(1, len(audio) // 50000))
    derivative = np.abs(audio[indexes] - audio[indexes - 1])
    seam = float(np.max(np.abs(audio[0] - audio[-1]))) if len(audio) else 0.
    return {"peak": peak, "rms": rms, "rms_dbfs": 20 * math.log10(max(1e-12, rms)),
            "dc": [float(x) for x in np.mean(audio, axis=0)], "seam_delta": seam,
            "seam_vs_p99_derivative": seam / max(1e-9, float(np.percentile(derivative, 99))) if derivative.size else 0}


def _vlq(number):
    result = [number & 127]; number >>= 7
    while number: result.insert(0, 128 | (number & 127)); number >>= 7
    return bytes(result)


def write_midi(path, data, events, start_beat, end_beat):
    ppq = 480
    tracks = []
    meter = data.get("meter", [4, 4])
    tempo = round(60_000_000 / data["bpm"])
    header_events = b"\0\xff\x51\x03" + tempo.to_bytes(3, "big") + b"\0\xff\x58\x04" + bytes([meter[0], int(math.log2(meter[1])), 24, 8])
    tracks.append(header_events + b"\0\xff\x2f\0")
    for index, track in enumerate(data["tracks"]):
        channel = 9 if track.get("slot") == "noise" else index if index < 9 else index + 1
        rows = []
        for e in events:
            if e.track != track["id"] or e.at >= end_beat or e.at + e.duration <= start_beat: continue
            start = round((max(start_beat, e.at) - start_beat) * ppq)
            end = max(start + 1, round((min(end_beat, e.at + e.duration) - start_beat) * ppq))
            rows.extend([(start, 1, bytes([0x90 | channel, e.note, max(1, round(e.velocity * 127))])),
                         (end, 0, bytes([0x80 | channel, e.note, 0]))])
        body = b"\0\xff\x03" + _vlq(len(track["id"].encode())) + track["id"].encode()
        body += b"\0" + bytes([0xc0 | channel, 80])
        previous = 0
        for tick, _, msg in sorted(rows): body += _vlq(tick - previous) + msg; previous = tick
        tracks.append(body + b"\0\xff\x2f\0")
    result = b"MThd" + struct.pack(">IHHH", 6, 1, len(tracks), ppq)
    for track in tracks: result += b"MTrk" + struct.pack(">I", len(track)) + track
    Path(path).write_bytes(result)


def render(path, out, sample_rate=32000, from_bar=0, bars=None, stems=True, solo=None):
    sample_rate = S.integer(sample_rate, 16000, 48000, "sample_rate")
    data = S.load(path)
    if solo:
        if set(solo) - {t["id"] for t in data["tracks"]}: raise S.ScoreError("Unknown solo track")
        data = copy.deepcopy(data)
        for track in data["tracks"]: track["mute"] = track["id"] not in solo
    validation = S.validate(data)
    events = S.compile_score(data)
    duration = validation["seconds"]
    max_note = max((e.duration * 60 / data["bpm"] + 2 for e in events), default=0)
    estimated = duration * sample_rate * 2 * 4 * 4 + max_note * sample_rate * 8 * 12
    if duration > 240 or estimated > 512 * 1024**2: raise S.ScoreError("Render budget exceeded (240s / estimated512MiB); shorten the score or long notes")
    beat_per_bar = S.beat_per_bar(data)
    start_beat = S.number(from_bar, 0, validation["bars"], "from_bar") * beat_per_bar
    end_beat = data["length"] if bars is None else start_beat + S.number(bars, .0625, 1024, "bars") * beat_per_bar
    if start_beat >= data["length"] or end_beat > data["length"] + 1e-8: raise S.ScoreError("Preview window outside score")
    start = round(start_beat * 60 / data["bpm"] * sample_rate)
    end = round(end_beat * 60 / data["bpm"] * sample_rate)
    frames = round(duration * sample_rate)
    out = Path(out).resolve(); out.mkdir(parents=True, exist_ok=True)
    key = {"score_sha256": S.digest(data), "engine_sha256": code_hash(), "sample_rate": sample_rate,
           "from_bar": from_bar, "bars": bars, "stems": bool(stems), "solo": sorted(solo or [])}
    with file_lock(out / ".render.lock"):
        manifest_path = out / "manifest.json"
        if manifest_path.exists():
            old = json.loads(manifest_path.read_text(encoding="utf-8"))
            if old.get("cache_key") == key and all((out / name).is_file() and file_hash(out / name) == value for name, value in old.get("files", {}).items()) and old.get("files"):
                return {**old, "cached": True, "manifest": str(manifest_path)}
        began = time.perf_counter()
        mix = np.zeros((frames, 2), dtype=np.float32)
        stem_peak = 0.
        scratch = []
        try:
            for track in data["tracks"]:
                audio = dsp.track_audio(data, track, events, sample_rate)
                stem_peak = max(stem_peak, float(audio.max()), -float(audio.min()))
                mix += audio
                if stems:
                    temp = out / ("." + uuid.uuid4().hex + ".npy")
                    scratch.append((track["id"], temp)); np.save(temp, audio[start:end])
                del audio
            peak = max(float(mix.max()), -float(mix.min()), stem_peak)
            gain = 10 ** (data.get("master_gain_db", -3) / 20)
            target = 10 ** (-1 / 20)
            if peak * gain > target: gain = target / peak
            # Same mastering gain for mix and every stem; no per-stem normalization.
            mix *= gain
            selected = mix[start:end]
            report = metrics(selected)
            write_wave(out / "mix.wav", selected, sample_rate=sample_rate)
            outputs = ["mix.wav"]
            if stems:
                for track_id, temp in scratch:
                    raw = np.load(temp, mmap_mode="r")
                    try: write_wave(out / f"stem_{track_id}.wav", raw, gain, sample_rate)
                    finally: raw._mmap.close()
                    outputs.append(f"stem_{track_id}.wav")
            write_midi(out / "score.mid", data, events, start_beat, end_beat); outputs.append("score.mid")
            write_json(out / "score.json", data); outputs.append("score.json")
            write_json(out / "timeline.json", {"units": "quarter-note beats", "sections": data.get("sections", []), "events": S.timeline(data)})
            outputs.append("timeline.json")
            warnings = list(validation["warnings"])
            if report["rms"] < 1e-5: warnings.append("Silent or nearly silent render")
            full_loop = start == 0 and end == frames
            if full_loop and report["seam_vs_p99_derivative"] > 4: warnings.append("Loop boundary spike; inspect the loop by ear")
            manifest = {"engine": VERSION, "cache_key": key, "cached": False, "title": data["title"],
                "profile": data["profile"], "frames": end - start, "sample_rate": sample_rate, "seconds": (end-start)/sample_rate,
                "full_loop": full_loop, "window_beats": [start_beat, end_beat], "applied_gain": gain,
                "max_gated_voices": validation["max_gated_voices"], "metrics": report, "warnings": warnings,
                "files": {name: file_hash(out / name) for name in outputs}, "render_seconds": time.perf_counter() - began,
                "mix": str(out / "mix.wav"), "output": str(out)}
            write_json(manifest_path, manifest)
            return {**manifest, "manifest": str(manifest_path)}
        finally:
            for _, temp in scratch:
                if temp.exists(): temp.unlink()


def compare(paths):
    results = []
    for path in paths:
        manifest = json.loads(Path(path).read_text(encoding="utf-8"))
        results.append({"title": manifest["title"], "profile": manifest["profile"], "mix": manifest["mix"],
                        "metrics": manifest["metrics"], "warnings": manifest["warnings"], "cache_key": manifest["cache_key"]})
    return {"renders": results, "note": "Metrics are technical checks, not an artistic rating."}


def describe():
    return {"application": "chiptune-studio", "engine": VERSION, "profiles": S.PROFILES, "instruments": S.DEFAULT_BANK,
            "format": {"version": 1, "time": "quarter-note beats, zero-based; bar size = meter numerator*4/denominator",
                "note": "MIDI0..127 or C4/F#3/Bb2; lists are simultaneous chords", "patterns": "named {length,notes:[id,at,note,duration,velocity,instrument?]}",
                "tracks": "id,slot?,instrument,gain_db,pan,mute?,clips:[id,pattern,at,repeat?,transpose?,velocity?]",
                "instrument_fields": "wave,duty,adsr[a_s,d_s,sustain,r_s],vibrato[hz,cents,delay_s],bits,table,arpeggio,arp_hz,pitch_slide,slide_ms",
                "patch": "RFC6902 add/remove/replace/test; requires current score_sha256; validates whole revision before writing"},
            "workflow": ["create or transcribe", "inspect", "patch a revision", "validate", "render a short preview", "listen and revise", "render full mix and stems"]}
