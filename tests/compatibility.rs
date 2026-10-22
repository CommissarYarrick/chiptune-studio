use chiptune_studio::{engine, score, transcribe};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn temp() -> PathBuf {
    let path = std::env::temp_dir().join(format!("chip-compat-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}
fn read_wav(path: &std::path::Path) -> (u32, Vec<i16>) {
    let mut reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().bits_per_sample, 16);
    (
        reader.spec().sample_rate,
        reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
    )
}

#[test]
fn all_original_scores_preserve_event_counts_and_voice_budgets() {
    for (source, events, voices) in [
        (include_str!("../examples/chip4.json"), 214, 4),
        (include_str!("../examples/console8.json"), 182, 7),
        (include_str!("../examples/reed_bells.json"), 397, 7),
    ] {
        let s: Value = serde_json::from_str(source).unwrap();
        let report = score::validate(&s).unwrap();
        assert_eq!(report["events"], events);
        assert_eq!(report["max_gated_voices"], voices);
    }
}
#[test]
fn mix_is_sum_of_stems_and_preview_is_exact_full_render_slice() {
    let dir = temp();
    let src = dir.join("score.json");
    engine::create(&src, "chip4", "Compatibility", "lanterns").unwrap();
    let full = engine::render(&src, &dir.join("full"), 16000, 0., None, true, &[]).unwrap();
    let (_, mix) = read_wav(&dir.join("full/mix.wav"));
    let mut sum = vec![0i32; mix.len()];
    for key in full["files"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| k.starts_with("stem_"))
    {
        let (_, pcm) = read_wav(&dir.join("full").join(key));
        for (total, sample) in sum.iter_mut().zip(pcm) {
            *total += i32::from(sample);
        }
    }
    assert!(
        sum.iter()
            .zip(&mix)
            .all(|(&a, &b)| (a - i32::from(b)).abs() <= 3)
    );
    let bpm = score::load(&src).unwrap()["bpm"].as_f64().unwrap();
    engine::render(&src, &dir.join("preview"), 16000, 1., Some(1.), false, &[]).unwrap();
    let (rate, preview) = read_wav(&dir.join("preview/mix.wav"));
    assert_eq!(rate, 16000);
    let start = (4. * 60. / bpm * 16000.).round_ties_even() as usize * 2;
    assert_eq!(&mix[start..start + preview.len()], preview);
    assert!(full["metrics"]["peak"].as_f64().unwrap() < 0.9);
}
#[test]
fn render_rejects_resource_overflow_before_audio_allocation() {
    let dir = temp();
    let src = dir.join("score.json");
    engine::create(&src, "console8", "Long", "empty").unwrap();
    let mut s = score::load(&src).unwrap();
    s["length"] = json!(1024);
    s["bpm"] = json!(30);
    engine::write_json(&src, &s).unwrap();
    assert!(
        engine::render(&src, &dir.join("out"), 48000, 0., None, false, &[])
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
}
#[test]
fn malformed_notes_and_failed_patch_never_panic_or_mutate_source() {
    for p in ["C2147483647", "D-2147483648", "Z4", "C+4", ""] {
        assert!(score::pitch(&json!(p)).is_err());
    }
    let s: Value = serde_json::from_str(include_str!("../examples/console8.json")).unwrap();
    assert!(
        score::apply_patch(
            &s,
            &json!([{"op":"replace","path":"/tracks/999","value":null}])
        )
        .is_err()
    );
    assert_eq!(s["title"], "Lantern Circuit");
}
#[test]
fn accompaniment_preserves_imported_melody_in_both_profiles() {
    for profile in ["chip4", "console8"] {
        let s = json!({"version":1,"title":"Sung phrase","profile":profile,"bpm":120,"meter":[4,4],"length":4,"seed":731,"instruments":score::default_bank(),"patterns":{"melody":{"length":4,"notes":[{"at":0,"note":60,"duration":1,"velocity":0.8},{"at":1.5,"note":64,"duration":1,"velocity":0.8},{"at":3,"note":67,"duration":0.75,"velocity":0.8}]}},"tracks":[{"id":"lead","slot":"pulse1","instrument":"lead","gain_db":-12,"pan":0,"clips":[{"pattern":"melody","at":0}]}]});
        let (arranged, _) = transcribe::accompany(&s).unwrap();
        assert_eq!(arranged["patterns"]["melody"], s["patterns"]["melody"]);
        assert!(score::validate(&arranged).is_ok());
    }
}

#[test]
fn ogg_export_is_deterministic_and_decodes_with_exact_frame_count() {
    let dir = temp();
    let audio: Vec<[f32; 2]> = (0..32000)
        .map(|i| {
            let s = (std::f64::consts::TAU * 440. * i as f64 / 32000.).sin() as f32 * 0.3;
            [s, s * 0.8]
        })
        .collect();
    let path = dir.join("mix.ogg");
    engine::write_ogg(&path, &audio, 32000, 731, "Loop").unwrap();
    let first = fs::read(&path).unwrap();
    assert_eq!(&first[..4], b"OggS");
    engine::write_ogg(&dir.join("repeat.ogg"), &audio, 32000, 731, "Loop").unwrap();
    assert_eq!(first, fs::read(dir.join("repeat.ogg")).unwrap());
    let file = fs::File::open(path).unwrap();
    let mut decoder = vorbis_rs::VorbisDecoder::new(file).unwrap();
    assert_eq!(decoder.sampling_frequency().get(), 32000);
    assert_eq!(decoder.channels().get(), 2);
    let mut count = 0;
    let mut squares = 0f64;
    while let Some(block) = decoder.decode_audio_block().unwrap() {
        let samples = block.samples();
        count += samples[0].len();
        for &s in samples[0] {
            assert!(s.is_finite());
            squares += (s as f64).powi(2);
        }
    }
    assert_eq!(count, audio.len());
    assert!((squares / count as f64).sqrt() > 0.19);
}
