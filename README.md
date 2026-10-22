# Chiptune Studio

A native Rust music workstation for people and AI agents. Write structured scores, synthesize chiptune, or hum a melody into your microphone and turn it into editable notes.

The application runs locally on the CPU. The score compiler, synthesis, pitch detection, CLI, HTTP server, and MCP server are implemented in Rust; the browser interface is embedded in the executable. No Python installation or hosted generation service is needed.

## Features

- **Two sound profiles:** `chip4` has four monophonic chip-inspired channels; `console8` supports eight simultaneous gated voices with stereo wavetable instruments. These are musical constraints, not cycle-accurate hardware emulation.
- **Browser studio:** record a hummed or whistled melody, import WAV, inspect and edit notes, save revisions, play the mix, and download Ogg.
- **Agent interfaces:** JSON CLI and eight MCP tools for composition, revisions, transcription, rendering, and comparisons.
- **Editable composition:** patterns, clips, sections, chords, tempo, envelopes, vibrato, arpeggios, pitch slides, bit depth, and custom wavetables.
- **Exports:** game-ready **Ogg Vorbis**, stereo PCM16 WAV, separate WAV tracks, MIDI, score JSON, event timeline, and a hash manifest.
- **Repeatable rendering:** deterministic noise, content-checked cache, atomic revisions, source-hash checks, and OS file locks.

## Build

Use a current Rust toolchain and a C compiler. Windows requires the MSVC C++ build tools; Unix builds need a working C toolchain. The C compiler builds the permissively licensed Vorbis libraries linked into the executable; it is not required to run the finished application.

```sh
git clone https://github.com/CommissarYarrick/chiptune-studio.git
cd chiptune-studio
cargo build --release --locked
```

Run on Windows:

```powershell
.\studio.cmd
```

Run on Linux or macOS:

```sh
./target/release/chip studio
```

Open **http://127.0.0.1:8765**. The Windows launcher opens your browser; the binary command only starts the server. Projects default to `data/` under the working directory. Set an explicit `--root /path/to/projects` or `CHIPTUNE_DATA_DIR` to choose another location. The Windows launcher defaults to `data/` beside the repository and supports `-Port`, `-Root`, and `-OpenBrowser`.

`CHIPTUNE_BIN` overrides the executable used by the Windows launchers. The server listens on loopback only and is intended as a local tool.

To install the CLI on your PATH instead:

```sh
cargo install --path . --locked
chip describe
```

## Try an original composition

**The Bell Beneath the Reeds** is an 80-second dungeon-synth miniature: 32 bars at 96 BPM, with bells, reed melody, soft organ, triangle bass, and restrained percussion. Its introduction, contrasting middle, varied return, and coda are written note by note.

[Listen to a 20-second Ogg excerpt](demo/reed-bells-preview.ogg) ([WAV](demo/reed-bells-preview.wav)), or render the complete piece:

```sh
chip render examples/reed_bells.json --out data/reed-bells
```

The output contains `mix.ogg` for game playback and `mix.wav` for lossless editing, plus stems, MIDI, and the editable score. Without a PATH installation, use `./target/release/chip` on Unix, or `.\chip.cmd` on Windows, in place of `chip`.

The other examples, `examples/chip4.json` and `examples/console8.json`, demonstrate the same shorter composition in both profiles.

## Hum or whistle a melody

1. Click **Record idea** and allow microphone access.
2. Record one clear melodic line, then stop, or import a WAV.
3. Choose a profile and transcribe. **Original** timing (the default) preserves the recorded rhythm; optional grids snap notes to the BPM. Start without accompaniment to compare the melody.
4. Review the detected pitches and durations, save a revision, and render.

The studio offers two separate results:

- **Voice to chip:** follows continuous detected pitch and the recording's loudness envelope. It keeps the full take's duration, including rests, without snapping to notes or a tempo grid. This preview is melody-only.
- **Editable notes mix:** converts the take to semitone notes you can edit and arrange. **Add accompaniment** also works on an already open imported project and renders a new revision immediately.

To build a composition from several recordings, open the first take, select **Add as a new layer to the open project**, choose its zero-based start bar, then record and transcribe the next theme. Each take remains a separate track. User-recorded parts take priority over automatic backing voices. CHIP 4 has three melodic channels plus noise; CONSOLE 8 supports eight voices.

Both modes depend on monophonic pitch detection: speech, breath noise, multiple simultaneous voices, and octave ambiguity can still need correction. Keep the original-recording player available for comparison. These are local operations; recordings are not uploaded to a service.


The original take is preserved. Local YIN pitch detection recognizes a **single melodic line**, not chords or full recordings. Recorded takes are limited to 60 seconds. An optional deterministic accompaniment provides a starting point, with the original melody unchanged. Review uncertain notes against the original take.

## CLI and score format

Commands return JSON; failures return a JSON error and exit status 2.

```sh
chip describe
chip new data/demo/score.json --profile chip4 --title "Lanterns" --template lanterns
chip inspect data/demo/score.json
chip validate data/demo/score.json
chip render data/demo/score.json --out data/demo/preview --bars 2
chip transcribe take.wav --out data/hummed/score.json --bpm 96 --profile console8
```

Times in a score are zero-based **quarter-note beats**. Pitches accept MIDI numbers, names such as `C4`, `F#3`, and `Bb2`, and lists for chords. Envelope and release times are seconds. `describe` provides the schema guide, profiles, and instrument bank. Existing version-1 JSON scores remain editable.

Apply RFC 6902 `add`, `remove`, `replace`, and `test` operations using the `score_sha256` from `inspect` and a new revision path:

```sh
chip patch data/demo/score.json --ops changes.json --out data/demo/revision.json --expected-sha256 YOUR_SOURCE_HASH
```

The whole result is validated before saving. Preview bars with `--from-bar` and `--bars`; select tracks with `--solo lead,bass`; omit separate tracks with `--no-stems`. Rendering is capped at 240 seconds and an estimated 512 MiB of working memory. All stems and the mix share a mastering gain; the manifest identifies the current output files and hashes.

Ogg uses the native Vorbis encoder at VBR quality 0.6. WAV and stems retain the uncompressed mastering output. Ogg serialization is deterministic for a given native render, with title metadata and a stable stream serial. The Rust noise generator uses SplitMix64 seeded by the event UID and score seed: it is repeatable, but old Python-generated noise waveforms and render hashes are not promised to be byte-identical.

## MCP

Configure your MCP client to launch the executable with `mcp --root /absolute/path/to/data`. Adapt `mcp.example.json` to your executable and data paths; it does not register the server automatically.

Tools: `chip_describe`, `chip_new_score`, `chip_read_score`, `chip_validate_score`, `chip_patch_score`, `chip_render_score`, `chip_import_hum`, and `chip_compare_renders`. The `chip://guide` resource describes the format and workflow. Paths are restricted to the configured data root. MCP uses JSON-RPC over stdio.

## Local verification

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all --check
```

Tests cover schema and polyphony constraints, revisions, synthesis, pitch detection, resampling, melody preservation, stems, preview slices, resource limits, cache repair, path confinement, and deterministic Ogg round-trips. Browser recording requires microphone permission and an AudioWorklet-capable browser. Real microphones and singing styles need individual review. Linux/macOS builds have not yet been run on those systems.

Audio metrics check clipping, DC offset, and loop continuity; they do not measure musical quality. Output directories are not recursively cleaned: the manifest is authoritative for a render's current files.

There is **no GitHub CI**. Verification is run locally.

## License

Original code and bundled example compositions use the **MIT License**. Dependencies retain their permissive licenses and attribution requirements. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), [LICENSE_POLICY.md](LICENSE_POLICY.md), and the bundled notices under `licenses/`.
