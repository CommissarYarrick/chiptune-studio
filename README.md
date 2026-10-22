# Chiptune Studio

A local, agent-friendly music workstation: write structured scores, synthesize chiptune, or hum a melody into your microphone and turn it into editable notes.

The synth and transcription run on your CPU. No hosted generation API, GPU, model weights, soundfonts, or subscription is required.

## Features

- **Two sound profiles:** `chip4` has four monophonic chip-inspired channels; `console8` supports eight simultaneous gated voices with stereo wavetable instruments. These are musical constraints, not cycle-accurate hardware emulation.
- **Browser studio:** record a hummed or whistled melody, import WAV, inspect and edit notes, save revisions, and play rendered audio.
- **Agent interfaces:** JSON CLI and eight Model Context Protocol tools for reading, editing, transcribing, rendering, and comparing scores.
- **Editable composition:** patterns, clips, sections, chords, tempo, envelopes, vibrato, arpeggios, pitch slides, bit depth, and custom wavetables.
- **Exports:** stereo PCM16 WAV, separate tracks, MIDI, score JSON, event timeline, and a hash manifest.
- **Repeatable rendering:** deterministic noise, content-checked cache, atomic revisions, source-hash checks, and OS file locks.

## Install

Use **Python 3.12 or newer**. Clone the repository and create a virtual environment:

```sh
git clone https://github.com/CommissarYarrick/chiptune-studio.git
cd chiptune-studio
python -m venv .venv
```

On Windows:

```powershell
.\.venv\Scripts\python.exe -m pip install -r requirements.txt
.\studio.cmd
```

On Linux or macOS:

```sh
.venv/bin/python -m pip install -r requirements.txt
.venv/bin/python chip.py studio
```

Open **http://127.0.0.1:8765**. The Windows launcher opens your browser; the Python command only starts the server. Projects are stored in `data/` beside the source by default. Override this with `--root /path/to/projects` or `CHIPTUNE_DATA_DIR`.

The Windows PowerShell launcher supports `-Port`, `-Root`, and `-OpenBrowser`; `CHIPTUNE_PYTHON` overrides its Python executable. The studio listens on loopback only. It is a local tool, not a public multi-user service.

`requirements.lock.txt` records the original Windows development environment. For a portable installation, use `requirements.txt`; the lock includes Windows-specific dependencies.

## Try an original composition

**The Bell Beneath the Reeds** is an 80-second dungeon-synth miniature: 32 bars at 96 BPM, with bells, reed melody, soft organ, triangle bass, and restrained percussion. Its introduction, contrasting middle, varied return, and coda are written note by note.

[Listen to a 20-second excerpt](demo/reed-bells-preview.wav), or render the complete piece from its editable score:

```sh
python chip.py render examples/reed_bells.json --out data/reed-bells
```

Listen to `data/reed-bells/mix.wav`. The other examples, `examples/chip4.json` and `examples/console8.json`, demonstrate the same short composition in both profiles. Run commands with the virtual environment's Python, or activate it first. Windows users can use `chip.cmd` instead of `python chip.py`.

## Hum or whistle a melody

1. Click **Record idea** and allow microphone access.
2. Record one clear melodic line, then stop. Alternatively, import a WAV.
3. Set the BPM and quantization, choose a profile, and transcribe.
4. Review the detected pitches and durations. Save a revision, render, and listen.

The original take is preserved. Local YIN pitch estimation recognizes a **single melodic line**, not chords or full recordings. Recorded takes are limited to 60 seconds. An optional deterministic accompaniment provides a starting point; it does not guarantee a finished arrangement. The interface displays recognition confidence to help identify uncertain notes.

## CLI

Commands return JSON; failures return a JSON error and exit status 2.

```sh
python chip.py describe
python chip.py new data/demo/score.json --profile chip4 --title "Lanterns" --template lanterns
python chip.py inspect data/demo/score.json
python chip.py validate data/demo/score.json
python chip.py render data/demo/score.json --out data/demo/preview --bars 2
python chip.py transcribe take.wav --out data/hummed/score.json --bpm 96 --profile console8
```

Times in a score are zero-based **quarter-note beats**. Pitches accept MIDI numbers, names such as `C4`, `F#3`, or `Bb2`, and lists for chords. Instrument envelope and release times are seconds. `describe` provides the schema guide, profiles, and instrument bank.

Use `patch` with RFC 6902 `add`, `remove`, `replace`, and `test` operations. Supply the `score_sha256` from `inspect` and a new revision path:

```sh
python chip.py patch data/demo/score.json --ops changes.json --out data/demo/revision.json --expected-sha256 YOUR_SOURCE_HASH
```

The entire result is validated before being saved. Preview selected bars with `--from-bar` and `--bars`; select tracks with `--solo lead,bass`; omit stems with `--no-stems`. Rendering is capped at 240 seconds and an estimated 512 MiB of working memory. Stems and the mix use the same mastering gain. The render manifest is authoritative for the current output files.

## MCP

Run `mcp_server.py --root /absolute/path/to/data` using the virtual environment's Python. Adapt the absolute placeholder paths in `mcp.example.json` to your MCP client. This file does not register the server automatically.

Tools: `chip_describe`, `chip_new_score`, `chip_read_score`, `chip_validate_score`, `chip_patch_score`, `chip_render_score`, `chip_import_hum`, and `chip_compare_renders`. The `chip://guide` resource describes the workflow. MCP paths are restricted to the configured project root.

## Tests and limitations

```sh
python -m unittest -v test_chiptune
```

The initial implementation passed 16 offline unit tests, 10 MCP integration checks, and a browser workflow with a synthetic microphone melody. Those checks cover transcription, revisions, rendering, source preservation, and access restrictions. Real microphones, singing styles, browsers, and operating systems may behave differently; Linux/macOS launch commands have not yet been verified on those systems.

Audio metrics check clipping, DC offset, and loop continuity. They do not measure musical quality. Browser recording requires microphone permission and an AudioWorklet-capable browser. Existing output files are not recursively cleaned; use the manifest to identify a render's files.

## License

Original code and bundled example compositions are released under the **MIT License**. Dependencies retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
