use chiptune_studio::transcribe;
use serde_json::Value;

const RATE: usize = 16_000;

#[derive(Clone, Copy)]
struct Tone {
    midi: i32,
    duration: f64,
}

fn midi_hz(midi: i32) -> f64 {
    440.0 * 2.0_f64.powf((midi as f64 - 69.0) / 12.0)
}

fn append_tone(samples: &mut Vec<f32>, midi: i32, duration: f64, amplitude: f64, rich: bool) {
    let count = (duration * RATE as f64).round() as usize;
    let mut phase = 0.0;
    for i in 0..count {
        let t = i as f64 / RATE as f64;
        let envelope = (t / 0.01).min((duration - t) / 0.01).clamp(0.0, 1.0);
        let vibrato = if rich {
            2.0_f64.powf(0.22 * (std::f64::consts::TAU * 5.0 * t).sin() / 12.0)
        } else {
            1.0
        };
        let fundamental = midi_hz(midi) * vibrato;
        phase += std::f64::consts::TAU * fundamental / RATE as f64;
        let harmonics = if rich {
            phase.sin() + 0.35 * (2.0 * phase).sin()
        } else {
            phase.sin()
        };
        samples.push((amplitude * envelope * harmonics) as f32);
    }
}

fn append_silence(samples: &mut Vec<f32>, duration: f64) {
    samples.resize(
        samples.len() + (duration * RATE as f64).round() as usize,
        0.0,
    );
}

fn sequence(tones: &[Tone], amplitude: f64, rich: bool) -> Vec<f32> {
    sequence_with_levels(
        &tones
            .iter()
            .map(|tone| (*tone, amplitude))
            .collect::<Vec<_>>(),
        rich,
    )
}

fn sequence_with_levels(tones: &[(Tone, f64)], rich: bool) -> Vec<f32> {
    let mut samples = Vec::new();
    append_silence(&mut samples, 0.12);
    for (i, (tone, amplitude)) in tones.iter().enumerate() {
        if i > 0 {
            append_silence(&mut samples, 0.08);
        }
        append_tone(&mut samples, tone.midi, tone.duration, *amplitude, rich);
    }
    samples
}

fn detected_notes(samples: &[f32], quantize: f64) -> Vec<Value> {
    transcribe::detect(samples, 120.0, quantize).unwrap()["notes"]
        .as_array()
        .unwrap()
        .clone()
}

fn seconds_at(note: &Value) -> f64 {
    note["at"].as_f64().unwrap() * 60.0 / 120.0
}

fn seconds_duration(note: &Value) -> f64 {
    note["duration"].as_f64().unwrap() * 60.0 / 120.0
}

fn note_pitch(note: &Value) -> i64 {
    note["note"].as_i64().unwrap()
}

#[test]
fn humming_keeps_three_pitches_and_approximately_tracks_timing() {
    let samples = sequence(
        &[
            Tone {
                midi: 57,
                duration: 0.37,
            },
            Tone {
                midi: 60,
                duration: 0.43,
            },
            Tone {
                midi: 62,
                duration: 0.31,
            },
        ],
        0.2,
        true,
    );
    let notes = detected_notes(&samples, 0.0);

    assert_eq!(
        notes.iter().map(note_pitch).collect::<Vec<_>>(),
        [57, 60, 62]
    );
    let expected_starts = [0.0, 0.45, 0.96];
    let expected_durations = [0.37, 0.43, 0.31];
    for ((note, expected_start), expected_duration) in
        notes.iter().zip(expected_starts).zip(expected_durations)
    {
        assert!((seconds_at(note) - expected_start).abs() <= 0.07);
        assert!((seconds_duration(note) - expected_duration).abs() <= 0.07);
    }
    for pair in notes.windows(2) {
        let previous_end = seconds_at(&pair[0]) + seconds_duration(&pair[0]);
        assert!(previous_end <= seconds_at(&pair[1]) + 0.01);
    }
}

#[test]
fn quiet_clean_tones_are_detected_above_a_ten_microvolt_floor() {
    let samples = sequence_with_levels(
        &[
            (
                Tone {
                    midi: 57,
                    duration: 0.4,
                },
                0.0008,
            ),
            (
                Tone {
                    midi: 64,
                    duration: 0.4,
                },
                0.0004,
            ),
        ],
        false,
    );
    let notes = detected_notes(&samples, 0.0);
    assert_eq!(notes.iter().map(note_pitch).collect::<Vec<_>>(), [57, 64]);
}

#[test]
fn zero_quantize_preserves_onset_off_the_quarter_beat_grid() {
    let samples = sequence(
        &[
            Tone {
                midi: 57,
                duration: 0.37,
            },
            Tone {
                midi: 60,
                duration: 0.43,
            },
        ],
        0.2,
        true,
    );
    let raw = detected_notes(&samples, 0.0);
    let quantized = detected_notes(&samples, 0.25);
    assert_eq!(raw.len(), 2);
    assert_eq!(quantized.len(), 2);
    assert!((seconds_at(&raw[1]) - seconds_at(&quantized[1])).abs() > 0.025);
}
