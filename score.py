"""Validated, deterministic musical data shared by CLI, renderer and MCP."""
from __future__ import annotations

import copy
import hashlib
import json
import math
import re
from dataclasses import asdict, dataclass
from pathlib import Path

PROFILES = {
    "chip4": {"slots": ["pulse1", "pulse2", "triangle", "noise"], "voices": 4,
              "description": "Four monophonic chip-inspired channels, pulse/pulse/triangle/noise."},
    "console8": {"slots": None, "voices": 8,
                 "description": "Eight gated voices, stereo wavetable and chip instruments."},
}
WAVES = {"pulse", "triangle", "saw", "sine", "noise", "kick", "wavetable"}
DEFAULT_BANK = {
    "lead": {"wave": "pulse", "duty": .25, "adsr": [.003, .10, .62, .045], "vibrato": [5.5, 12, .16]},
    "arp": {"wave": "pulse", "duty": .125, "adsr": [.002, .08, .28, .025]},
    "bass": {"wave": "triangle", "bits": 4, "adsr": [.002, .06, .8, .025]},
    "hat": {"wave": "noise", "adsr": [.001, .026, 0, .008]},
    "snare": {"wave": "noise", "adsr": [.001, .075, 0, .02]},
    "kick": {"wave": "kick", "pitch_slide": 18, "slide_ms": 35, "adsr": [.001, .09, 0, .015]},
    "bell": {"wave": "sine", "adsr": [.002, .30, .06, .18]},
    "warm": {"wave": "wavetable", "table": [0,.36,.66,.86,.97,1,.97,.93,.88,.83,.76,.67,.56,.43,.29,.14,
                  0,-.14,-.29,-.43,-.56,-.67,-.76,-.83,-.88,-.93,-.97,-1,-.97,-.86,-.66,-.36],
             "adsr": [.025, .16, .62, .20], "vibrato": [4.5, 5, .2]},
}


class ScoreError(ValueError):
    pass


def number(value, lo, hi, where):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not lo <= value <= hi:
        raise ScoreError(f"{where}: expected finite number in [{lo}, {hi}]")
    return float(value)


def integer(value, lo, hi, where):
    number(value, lo, hi, where)
    if int(value) != value:
        raise ScoreError(f"{where}: expected integer")
    return int(value)


def keys(obj, allowed, where):
    if not isinstance(obj, dict):
        raise ScoreError(f"{where}: expected object")
    extra = obj.keys() - set(allowed.split())
    if extra:
        raise ScoreError(f"{where}: unknown fields {sorted(extra)}")


def pitch(value):
    if isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= 127:
        return value
    if isinstance(value, str):
        match = re.fullmatch(r"([A-Ga-g])([#b]?)(-?\d+)", value)
        if match:
            letter, accidental, octave = match.groups()
            midi = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}[letter.upper()]
            midi += (int(octave) + 1) * 12 + {"": 0, "#": 1, "b": -1}[accidental]
            if 0 <= midi <= 127:
                return midi
    raise ScoreError(f"Invalid note {value!r}; use MIDI 0..127 or C4/F#3/Bb2")


def digest(score):
    return hashlib.sha256(json.dumps(score, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def load(path):
    try:
        score = json.loads(Path(path).read_text(encoding="utf-8-sig"))
        validate(score)
        return score
    except (KeyError, TypeError) as error:
        raise ScoreError(f"Malformed score: {error}") from error


def beat_per_bar(score):
    meter = score.get("meter", [4, 4])
    return meter[0] * 4 / meter[1]


@dataclass(frozen=True)
class Event:
    uid: str
    track: str
    instrument: str
    at: float
    duration: float
    note: int
    velocity: float


def compile_score(score):
    events = []
    for track in score["tracks"]:
        if track.get("mute", False):
            continue
        for ci, clip in enumerate(track["clips"]):
            pattern = score["patterns"][clip["pattern"]]
            for repeat in range(clip.get("repeat", 1)):
                for ni, note in enumerate(pattern["notes"]):
                    notes = note["note"] if isinstance(note["note"], list) else [note["note"]]
                    for pi, p in enumerate(notes):
                        uid = f"{track['id']}/{clip.get('id', ci)}/{repeat}/{note.get('id', ni)}/{pi}"
                        events.append(Event(uid, track["id"], note.get("instrument", track["instrument"]),
                            clip["at"] + repeat * pattern["length"] + note["at"], note["duration"],
                            pitch(p) + clip.get("transpose", 0), note.get("velocity", .8) * clip.get("velocity", 1)))
    return sorted(events, key=lambda e: (e.at, e.track, e.uid))


def validate(score):
    keys(score, "version title profile bpm meter length seed instruments patterns tracks sections master_gain_db delay", "score")
    if score.get("version") != 1 or isinstance(score.get("version"), bool):
        raise ScoreError("version must be 1")
    if not isinstance(score.get("title"), str) or not score["title"].strip():
        raise ScoreError("title must be nonempty")
    if score.get("profile") not in PROFILES:
        raise ScoreError(f"profile must be one of {list(PROFILES)}")
    number(score["bpm"], 30, 300, "bpm"); number(score["length"], .25, 1024, "length")
    integer(score.get("seed", 0), 0, 2**32 - 1, "seed")
    number(score.get("master_gain_db", -3), -60, 12, "master_gain_db")
    meter = score.get("meter", [4, 4])
    if not isinstance(meter, list) or len(meter) != 2:
        raise ScoreError("meter must be [numerator, denominator]")
    integer(meter[0], 1, 16, "meter numerator")
    if meter[1] not in (2, 4, 8, 16):
        raise ScoreError("meter denominator must be 2,4,8,16")
    bank = score.get("instruments", {})
    if not isinstance(bank, dict) or not bank or len(bank) > 64:
        raise ScoreError("instruments must contain 1..64 named instruments")
    for name, ins in bank.items():
        keys(ins, "wave duty adsr vibrato pitch_slide slide_ms bits table arpeggio arp_hz", f"instrument {name}")
        if ins.get("wave") not in WAVES:
            raise ScoreError(f"instrument {name}: unknown wave")
        adsr = ins.get("adsr", [.003, .1, .6, .04])
        if not isinstance(adsr, list) or len(adsr) != 4:
            raise ScoreError(f"instrument {name}: adsr needs four values")
        for i, v in enumerate(adsr): number(v, 0, 1 if i == 2 else 2, f"{name}.adsr[{i}]")
        vib = ins.get("vibrato", [0, 0, 0])
        if not isinstance(vib, list) or len(vib) != 3: raise ScoreError(f"{name}.vibrato needs [hz,cents,delay_s]")
        for v, hi in zip(vib, [20, 100, 5]): number(v, 0, hi, f"{name}.vibrato")
        number(ins.get("duty", .5), .0625, .9375, f"{name}.duty")
        number(ins.get("pitch_slide", 0), -36, 36, f"{name}.pitch_slide")
        number(ins.get("slide_ms", 40), 1, 1000, f"{name}.slide_ms")
        integer(ins.get("bits", 16), 4, 16, f"{name}.bits")
        arp = ins.get("arpeggio", [0])
        if not isinstance(arp, list) or not 1 <= len(arp) <= 16: raise ScoreError(f"{name}.arpeggio needs 1..16 semitone offsets")
        for offset in arp: integer(offset, -24, 24, f"{name}.arpeggio")
        number(ins.get("arp_hz", 24), 1, 60, f"{name}.arp_hz")
        if ins["wave"] == "wavetable":
            table = ins.get("table", [])
            if not isinstance(table, list) or not 8 <= len(table) <= 256: raise ScoreError(f"{name}.table needs 8..256 values")
            for v in table: number(v, -1, 1, f"{name}.table")
    patterns = score.get("patterns")
    if not isinstance(patterns, dict) or not patterns or len(patterns) > 256: raise ScoreError("patterns needs 1..256 entries")
    note_count = 0
    for name, pattern in patterns.items():
        keys(pattern, "length notes", f"pattern {name}")
        number(pattern["length"], .0625, 1024, f"{name}.length")
        if not isinstance(pattern["notes"], list): raise ScoreError(f"{name}.notes must be list")
        ids = set()
        for ni, note in enumerate(pattern["notes"]):
            keys(note, "id at note duration velocity instrument", f"{name}.notes[{ni}]")
            ident = str(note.get("id", ni))
            if ident in ids: raise ScoreError(f"{name}: duplicate note id {ident}")
            ids.add(ident)
            number(note["at"], 0, pattern["length"], "note.at")
            number(note["duration"], .001, pattern["length"], "note.duration")
            number(note.get("velocity", .8), .001, 1, "note.velocity")
            if note["at"] + note["duration"] > pattern["length"] + 1e-8: raise ScoreError(f"{name}: note extends beyond pattern")
            chord = note["note"] if isinstance(note["note"], list) else [note["note"]]
            if not 1 <= len(chord) <= 8: raise ScoreError("note chord needs 1..8 pitches")
            for p in chord: pitch(p)
            if note.get("instrument", next(iter(bank))) not in bank: raise ScoreError("unknown note instrument")
            note_count += len(chord)
    if note_count > 8192: raise ScoreError("Too many pattern notes (max8192)")
    tracks = score.get("tracks")
    if not isinstance(tracks, list) or not 1 <= len(tracks) <= 8: raise ScoreError("tracks must contain 1..8 entries")
    track_ids, slots = set(), set()
    expansion = 0
    for track in tracks:
        keys(track, "id slot instrument gain_db pan clips mute", "track")
        if not isinstance(track.get("id"), str) or not re.fullmatch(r"[a-zA-Z0-9_-]{1,40}", track["id"]): raise ScoreError("track.id needs a short filename-safe identifier")
        if track["id"] in track_ids: raise ScoreError("duplicate track.id")
        track_ids.add(track["id"])
        if track.get("instrument") not in bank: raise ScoreError("unknown track instrument")
        number(track.get("gain_db", -6), -60, 12, "track.gain_db"); number(track.get("pan", 0), -1, 1, "track.pan")
        if not isinstance(track.get("mute", False), bool): raise ScoreError("track.mute must be boolean")
        if not isinstance(track.get("clips"), list) or len(track["clips"]) > 1024: raise ScoreError("track.clips must be list, max1024")
        if score["profile"] == "chip4":
            slot = track.get("slot")
            if slot not in PROFILES["chip4"]["slots"] or slot in slots: raise ScoreError("chip4 needs unique pulse1/pulse2/triangle/noise slots")
            slots.add(slot)
        clip_ids = set()
        for ci, clip in enumerate(track["clips"]):
            keys(clip, "id pattern at repeat transpose velocity", "clip")
            ident = str(clip.get("id", ci))
            if ident in clip_ids: raise ScoreError("duplicate clip.id")
            clip_ids.add(ident)
            if clip.get("pattern") not in patterns: raise ScoreError("unknown clip pattern")
            number(clip["at"], 0, score["length"], "clip.at")
            repeat = integer(clip.get("repeat", 1), 1, 256, "clip.repeat")
            integer(clip.get("transpose", 0), -48, 48, "clip.transpose"); number(clip.get("velocity", 1), .001, 1, "clip.velocity")
            pat = patterns[clip["pattern"]]
            if clip["at"] + repeat * pat["length"] > score["length"] + 1e-8: raise ScoreError("clip extends beyond score.length")
            expansion += repeat * sum(len(n["note"]) if isinstance(n["note"], list) else 1 for n in pat["notes"])
    if expansion > 32768: raise ScoreError("Expanded score exceeds32768 notes")
    section_ids = set()
    for section in score.get("sections", []):
        keys(section, "id at length", "section")
        if not isinstance(section.get("id"), str) or section["id"] in section_ids: raise ScoreError("sections need unique ids")
        section_ids.add(section["id"])
        number(section["at"], 0, score["length"], "section.at"); number(section["length"], .001, score["length"], "section.length")
        if section["at"] + section["length"] > score["length"] + 1e-8: raise ScoreError("section beyond score")
    delay = score.get("delay", {})
    keys(delay, "beats mix feedback", "delay")
    number(delay.get("beats", .75), .0625, 8, "delay.beats"); number(delay.get("mix", 0), 0, .5, "delay.mix"); number(delay.get("feedback", .3), 0, .7, "delay.feedback")
    events = compile_score(score)
    sweep, per_track = [], {}
    by_id = {t["id"]: t for t in tracks}
    for event in events:
        if not 0 <= event.note <= 127: raise ScoreError("transposition moves note outside MIDI0..127")
        wave = bank[event.instrument]["wave"]
        if score["profile"] == "chip4":
            slot = by_id[event.track]["slot"]
            allowed = {"pulse1": {"pulse"}, "pulse2": {"pulse"}, "triangle": {"triangle"}, "noise": {"noise", "kick"}}[slot]
            if wave not in allowed: raise ScoreError(f"{event.track}: {wave} not available in {slot}")
        changes = [(event.at, 1), (event.at + event.duration, -1)]
        sweep.extend(changes); per_track.setdefault(event.track, []).extend(changes)
    def peak_poly(changes):
        active = peak = 0
        for _, delta in sorted(changes): active += delta; peak = max(peak, active)
        return peak
    max_poly = peak_poly(sweep)
    if max_poly > PROFILES[score["profile"]]["voices"]: raise ScoreError(f"voice budget exceeded: {max_poly}")
    if score["profile"] == "chip4" and any(peak_poly(v) > 1 for v in per_track.values()): raise ScoreError("chip4 channel contains overlapping gates/chords")
    return {"valid": True, "score_sha256": digest(score), "events": len(events), "max_gated_voices": max_poly,
            "seconds": score["length"] * 60 / score["bpm"], "bars": score["length"] / beat_per_bar(score),
            "profile": score["profile"], "warnings": ["Score contains no sounding notes"] if not events else []}


def timeline(score):
    return [asdict(e) for e in compile_score(score)]


def apply_patch(score, operations):
    """Atomic RFC6902 add/remove/replace/test subset; caller validates before saving."""
    result = copy.deepcopy(score)
    if not isinstance(operations, list) or len(operations) > 256: raise ScoreError("patch requires at most256 operations")
    for op in operations:
        keys(op, "op path value", "patch operation")
        if op.get("op") not in {"add", "remove", "replace", "test"}: raise ScoreError("unsupported patch operation")
        path = op.get("path")
        if not isinstance(path, str) or not path.startswith("/"): raise ScoreError("patch path must be a JSON pointer below root")
        parts = [p.replace("~1", "/").replace("~0", "~") for p in path[1:].split("/")]
        node = result
        try:
            for part in parts[:-1]: node = node[int(part)] if isinstance(node, list) else node[part]
            key = parts[-1]
            if isinstance(node, list):
                idx = len(node) if key == "-" and op["op"] == "add" else int(key)
                if idx < 0 or idx > len(node): raise IndexError(key)
                if op["op"] == "add": node.insert(idx, op["value"])
                elif op["op"] == "remove": node.pop(idx)
                elif op["op"] == "test":
                    if node[idx] != op["value"]: raise ScoreError("patch test failed")
                else: node[idx] = op["value"]
            elif isinstance(node, dict):
                if op["op"] != "add" and key not in node: raise KeyError(key)
                if op["op"] == "remove": del node[key]
                elif op["op"] == "test":
                    if node[key] != op["value"]: raise ScoreError("patch test failed")
                else: node[key] = op["value"]
            else: raise ScoreError("patch parent is not container")
        except (KeyError, IndexError, ValueError, TypeError) as error:
            raise ScoreError(f"patch {path}: {error}") from error
    validate(result)
    return result
