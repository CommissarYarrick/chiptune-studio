"""Offline regression tests for score validation, synthesis, rendering and transcription."""
from __future__ import annotations

import copy
import hashlib
import json
import sys
import tempfile
import unittest
import wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dsp
import engine
import score
import transcribe


HERE = Path(__file__).resolve().parent


def score_fixture(profile="chip4", notes=None):
    source = json.loads((HERE / "examples" / f"{profile}.json").read_text(encoding="utf-8"))
    source.update(title="Unit fixture", bpm=120, length=1, delay={"mix": 0})
    source["patterns"] = {"melody": {"length": 1, "notes": notes if notes is not None else [
        {"id": "n0", "at": 0, "note": 60, "duration": .5, "velocity": .8}]}}
    source["tracks"] = [{"id": "lead", "slot": "pulse1", "instrument": "lead", "gain_db": -18,
                         "pan": 0, "clips": [{"id": "c", "pattern": "melody", "at": 0}]}]
    source["sections"] = [{"id": "A", "at": 0, "length": 1}]
    return source


def write_score(path, data):
    path.write_text(json.dumps(data, allow_nan=False), encoding="utf-8")
    return path


def read_pcm(path):
    with wave.open(str(path), "rb") as stream:
        channels, rate, frames = stream.getnchannels(), stream.getframerate(), stream.getnframes()
        samples = np.frombuffer(stream.readframes(frames), dtype="<i2").reshape(-1, channels)
    return samples, rate


def write_hum(path, tones, rate=16000, note_seconds=.5, gap_seconds=.12, amplitude=.2,
              harmonics=()):
    signal = []
    fade_samples = round(.003 * rate)
    for frequency in tones:
        count = round(note_seconds * rate)
        time = np.arange(count) / rate
        segment = np.sin(2 * np.pi * frequency * time)
        for harmonic, gain in harmonics:
            segment += gain * np.sin(2 * np.pi * frequency * harmonic * time)
        envelope = np.ones(count)
        envelope[:fade_samples] = np.linspace(0, 1, fade_samples, endpoint=False)
        envelope[-fade_samples:] = np.linspace(1, 0, fade_samples, endpoint=False)
        signal.extend(np.rint(np.clip(segment * envelope * amplitude, -1, 1) * 32767).astype("<i2"))
        signal.extend(np.zeros(round(gap_seconds * rate), dtype="<i2"))
    pcm = np.asarray(signal, dtype="<i2")
    with wave.open(str(path), "wb") as stream:
        stream.setnchannels(1)
        stream.setsampwidth(2)
        stream.setframerate(rate)
        stream.writeframes(pcm.tobytes())
    return hashlib.sha256(path.read_bytes()).hexdigest()


class ScoreTests(unittest.TestCase):
    def test_example_profiles_validate_with_expected_voice_peaks(self):
        for profile, peak in (("chip4", 4), ("console8", 7)):
            with self.subTest(profile=profile):
                data = json.loads((HERE / "examples" / f"{profile}.json").read_text(encoding="utf-8"))
                result = score.validate(data)
                self.assertTrue(result["valid"])
                self.assertEqual(result["max_gated_voices"], peak)

    def test_rejects_unknown_fields_and_nonfinite_values(self):
        data = score_fixture()
        data["unexpected"] = True
        with self.assertRaises(score.ScoreError):
            score.validate(data)
        data = score_fixture()
        data["bpm"] = float("nan")
        with self.assertRaises(score.ScoreError):
            score.validate(data)

    def test_rejects_overlapping_mono_channel_and_transposed_pitch_out_of_range(self):
        notes = [{"id": "a", "at": 0, "note": 60, "duration": .6},
                 {"id": "b", "at": .5, "note": 62, "duration": .4}]
        with self.assertRaisesRegex(score.ScoreError, "overlapping"):
            score.validate(score_fixture(notes=notes))
        notes = [{"id": "top", "at": 0, "note": 127, "duration": .5}]
        data = score_fixture(notes=notes)
        data["tracks"][0]["clips"][0]["transpose"] = 1
        with self.assertRaisesRegex(score.ScoreError, "outside MIDI"):
            score.validate(data)

    def test_patch_creates_valid_copy_and_preserves_original(self):
        with tempfile.TemporaryDirectory() as directory:
            original = write_score(Path(directory) / "original.json", score_fixture())
            revised = Path(directory) / "revision.json"
            before = original.read_bytes()
            result = engine.patch(original, [{"op": "replace", "path": "/title", "value": "Revised"}],
                                  revised, score.digest(json.loads(before)))
            self.assertEqual(json.loads(before), score.load(original))
            self.assertEqual(score.load(revised)["title"], "Revised")
            self.assertTrue(result["valid"])

    def test_patch_test_failure_and_stale_hash_never_write_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            original = write_score(Path(directory) / "original.json", score_fixture())
            failed = Path(directory) / "failed.json"
            with self.assertRaises(score.ScoreError):
                engine.patch(original, [{"op": "test", "path": "/title", "value": "wrong"}],
                             failed, score.digest(score.load(original)))
            stale = Path(directory) / "stale.json"
            with self.assertRaisesRegex(score.ScoreError, "changed"):
                engine.patch(original, [{"op": "replace", "path": "/title", "value": "x"}], stale, "0" * 64)
            self.assertFalse(failed.exists())
            self.assertFalse(stale.exists())

    def test_voice_budget_rejects_bad_score(self):
        notes = [{"id": "chord", "at": 0, "note": [60, 62, 64, 65, 67], "duration": .5}]
        with self.assertRaisesRegex(score.ScoreError, "voice budget"):
            score.validate(score_fixture(notes=notes))


class SynthesisTests(unittest.TestCase):
    def test_sine_voice_has_a440_fundamental(self):
        event = score.Event("sine", "lead", "tone", 0, 1, 69, 1)
        audio = dsp.voice(event, {"wave": "sine", "adsr": [.003, .05, 1, 0]}, 16000, 120, 1)
        window = audio[800:15200].astype(np.float64)
        spectrum = np.abs(np.fft.rfft(window * np.hanning(len(window))))
        frequencies = np.fft.rfftfreq(len(window), 1 / 16000)
        dominant = frequencies[np.argmax(spectrum)]
        self.assertAlmostEqual(dominant, 440, delta=2)

    def test_noise_is_deterministic_by_event_uid_and_seed(self):
        instrument = {"wave": "noise", "adsr": [.003, .05, .7, .02]}
        event = score.Event("noise-a", "drum", "noise", 0, .25, 48, .8)
        same = dsp.voice(event, instrument, 16000, 120, 731)
        repeated = dsp.voice(event, instrument, 16000, 120, 731)
        other = dsp.voice(score.Event("noise-b", "drum", "noise", 0, .25, 48, .8), instrument,
                          16000, 120, 731)
        np.testing.assert_array_equal(same, repeated)
        self.assertFalse(np.array_equal(same, other))


class RenderTests(unittest.TestCase):
    def test_render_outputs_correct_duration_finite_stereo_pcm_and_midi(self):
        with tempfile.TemporaryDirectory() as directory:
            score_path = write_score(Path(directory) / "score.json", score_fixture())
            out = Path(directory) / "render"
            manifest = engine.render(score_path, out, sample_rate=16000, stems=True)
            pcm, rate = read_pcm(out / "mix.wav")
            self.assertEqual(rate, 16000)
            self.assertEqual(pcm.shape, (8000, 2))
            self.assertTrue(np.isfinite(pcm).all())
            self.assertLess(np.max(np.abs(pcm.astype(np.float64))) / 32768, .9)
            self.assertEqual(manifest["frames"], 8000)
            self.assertEqual((out / "score.mid").read_bytes()[:4], b"MThd")

    def test_mix_matches_sum_of_pcm_stems_with_rounding_tolerance(self):
        data = score_fixture(notes=[{"id": "a", "at": 0, "note": 60, "duration": .5},
                                    {"id": "b", "at": .5, "note": 67, "duration": .5}])
        data["tracks"].append({"id": "bass", "slot": "triangle", "instrument": "bass", "gain_db": -22,
                               "pan": .2, "clips": [{"id": "b", "pattern": "melody", "at": 0}]})
        with tempfile.TemporaryDirectory() as directory:
            score_path = write_score(Path(directory) / "score.json", data)
            out = Path(directory) / "render"
            engine.render(score_path, out, sample_rate=16000, stems=True)
            mix, _ = read_pcm(out / "mix.wav")
            summed = read_pcm(out / "stem_lead.wav")[0].astype(np.int32)
            summed += read_pcm(out / "stem_bass.wav")[0].astype(np.int32)
            self.assertLessEqual(int(np.max(np.abs(mix.astype(np.int32) - summed))), 3)

    def test_cache_hit_reuses_mix_and_tampering_invalidates_it(self):
        with tempfile.TemporaryDirectory() as directory:
            score_path = write_score(Path(directory) / "score.json", score_fixture())
            out = Path(directory) / "render"
            first = engine.render(score_path, out, sample_rate=16000)
            stamp = (out / "mix.wav").stat().st_mtime_ns
            second = engine.render(score_path, out, sample_rate=16000)
            self.assertTrue(second["cached"])
            self.assertEqual((out / "mix.wav").stat().st_mtime_ns, stamp)
            with (out / "mix.wav").open("ab") as stream:
                stream.write(b"tampered")
            third = engine.render(score_path, out, sample_rate=16000)
            self.assertFalse(third["cached"])

    def test_mix_is_identical_with_stems_enabled_or_disabled(self):
        with tempfile.TemporaryDirectory() as directory:
            score_path = write_score(Path(directory) / "score.json", score_fixture())
            with_stems = Path(directory) / "with-stems"
            without_stems = Path(directory) / "without-stems"
            engine.render(score_path, with_stems, sample_rate=16000, stems=True)
            engine.render(score_path, without_stems, sample_rate=16000, stems=False)
            np.testing.assert_array_equal(read_pcm(with_stems / "mix.wav")[0],
                                          read_pcm(without_stems / "mix.wav")[0])


class TranscriptionTests(unittest.TestCase):
    def test_hum_import_finds_three_notes_on_quantized_grid_and_saves_source_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "hum.wav"
            source_hash = write_hum(source, [261.63, 329.63, 392.0])
            out = Path(directory) / "melody.json"
            result = transcribe.import_take(source, out, bpm=120, quantize=.25, arrange=False)
            notes = score.load(out)["patterns"]["melody"]["notes"]
            self.assertEqual(len(notes), 3)
            expected = [60, 64, 67]
            for note, pitch in zip(notes, expected):
                self.assertLessEqual(abs(note["note"] - pitch), 1)
                self.assertAlmostEqual(note["at"] / .25, round(note["at"] / .25))
                self.assertAlmostEqual(note["duration"] / .25, round(note["duration"] / .25))
            for previous, current in zip(notes, notes[1:]):
                self.assertLessEqual(previous["at"] + previous["duration"], current["at"] + 1e-9)
            self.assertEqual(result["transcription"]["audio"]["sha256"], source_hash)
            self.assertEqual(Path(result["transcription"]["audio"]["saved_take"]).read_bytes(), source.read_bytes())

    def test_silence_fails_without_creating_import_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "silence.wav"
            write_hum(source, [0], note_seconds=.5, amplitude=0)
            out = Path(directory) / "silent.json"
            with self.assertRaises(score.ScoreError):
                transcribe.import_take(source, out, arrange=False)
            self.assertFalse(out.exists())
            self.assertFalse(out.with_suffix(".transcription.json").exists())
            self.assertEqual(list(Path(directory).glob("take_*.wav")), [])

    def test_louder_second_harmonic_does_not_raise_detected_fundamental(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "harmonic.wav"
            write_hum(source, [220], note_seconds=.8, gap_seconds=0, amplitude=.12, harmonics=[(2, 1.8)])
            result = transcribe.import_take(source, Path(directory) / "harmonic.json", arrange=False)
            detected = result["score"]["patterns"]["melody"]["notes"][0]["note"]
            self.assertLessEqual(abs(detected - 57), 1)

    def test_accompaniment_preserves_lead_and_validates_for_both_profiles(self):
        notes = [{"id": "c", "at": 0, "note": 60, "duration": .75},
                 {"id": "e", "at": 1, "note": 64, "duration": .75},
                 {"id": "g", "at": 2, "note": 67, "duration": .75}]
        for profile in ("chip4", "console8"):
            with self.subTest(profile=profile):
                source = score_fixture(profile, copy.deepcopy(notes))
                source["length"] = 4
                source["patterns"]["melody"]["length"] = 4
                source["sections"] = [{"id": "A", "at": 0, "length": 4}]
                self.assertTrue(score.validate(source)["valid"])
                expected = copy.deepcopy(source["patterns"]["melody"]["notes"])
                arranged, metadata = transcribe.accompany(source)
                self.assertEqual(arranged["patterns"]["melody"]["notes"], expected)
                self.assertTrue(score.validate(arranged)["valid"])
                self.assertIn("progression", metadata)


if __name__ == "__main__":
    unittest.main()
