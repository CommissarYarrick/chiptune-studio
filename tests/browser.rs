use chiptune_studio::browser::{self, Output};
use serde_json::{Value, json};

fn value(request: Value) -> Value {
    match browser::run(&request).unwrap() {
        Output::Json(value) => value,
        _ => panic!("Expected a score"),
    }
}

#[test]
fn browser_creates_valid_empty_scores_without_files() {
    for profile in ["chip4", "console8"] {
        let score = value(json!({"command":"new","profile":profile,"title":"Session song"}));
        assert_eq!(score["profile"], profile);
        assert!(chiptune_studio::score::validate(&score).is_ok());
        assert_eq!(
            score["patterns"]["melody"]["notes"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}

#[test]
fn browser_transcribes_and_renders_memory_only() {
    use base64::Engine;
    let mut bytes = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut bytes,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..16000 {
            let sample =
                (0.3 * (std::f64::consts::TAU * 220. * i as f64 / 16000.).sin() * 32767.) as i16;
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
    let take = value(json!({"command":"transcribe","wav_base64":encoded}));
    assert_eq!(take["score"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(take["score"]["patterns"]["melody"]["notes"][0]["note"], 57);
    assert!(take.get("path").is_none());
    for request in [
        json!({"command":"render","score":take["score"]}),
        json!({"command":"continuous","wav_base64":encoded,"transcription":take["transcription"]}),
    ] {
        let Output::Wave(wave) = browser::run(&request).unwrap() else {
            panic!("Expected WAV")
        };
        let mut reader = hound::WavReader::new(std::io::Cursor::new(wave)).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 32000);
        assert!(
            reader
                .samples::<i16>()
                .map(|x| (x.unwrap() as i32).abs())
                .max()
                .unwrap()
                > 1000
        );
    }
}

#[test]
fn browser_rejects_excessive_song_duration() {
    let mut score = value(json!({"command":"new"}));
    score["length"] = json!(512);
    assert!(browser::run(&json!({"command":"render","score":score})).is_err());
}
