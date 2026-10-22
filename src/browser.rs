//! Portable, memory-only studio operations. No files, network, or persistence.
use crate::{dsp, layers, score, transcribe};
use anyhow::{Result, anyhow, ensure};
use base64::Engine;
use serde_json::{Value, json};
use std::io::Cursor;

pub enum Output {
    Json(Value),
    Wave(Vec<u8>),
}

fn wave(audio: &[[f32; 2]], gain: f64) -> Result<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut bytes,
            hound::WavSpec {
                channels: 2,
                sample_rate: 32000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for frame in audio {
            for &sample in frame {
                ensure!(sample.is_finite(), "Invalid audio sample");
                writer.write_sample(
                    ((sample as f64 * gain).clamp(-1., 1.) * 32767.).round_ties_even() as i16,
                )?;
            }
        }
        writer.finalize()?;
    }
    Ok(bytes.into_inner())
}

pub fn run(request: &Value) -> Result<Output> {
    let input = &request["score"];
    match request["command"].as_str().unwrap_or("") {
        "transcribe" => {
            let encoded = request["wav_base64"]
                .as_str()
                .ok_or_else(|| anyhow!("Recording missing"))?;
            ensure!(
                encoded.len() <= 20 * 1024 * 1024,
                "Recording exceeds browser limit"
            );
            let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
            let (samples, _) = transcribe::decode_wav(&bytes)?;
            let bpm = request["bpm"].as_f64().unwrap_or(120.);
            let analysis =
                transcribe::detect(&samples, bpm, request["quantize"].as_f64().unwrap_or(0.))?;
            let data = transcribe::melody_score(
                &analysis,
                bpm,
                request["profile"].as_str().unwrap_or("chip4"),
                request["title"].as_str().unwrap_or("New track"),
            )?;
            Ok(Output::Json(json!({"score":data,"transcription":analysis})))
        }
        "render" => {
            let events = score::compile(input)?;
            let seconds = input["length"].as_f64().unwrap() * 60. / input["bpm"].as_f64().unwrap();
            ensure!(seconds <= 120., "Browser songs are limited to two minutes");
            let count = (seconds * 32000.).round_ties_even() as usize;
            let mut mix = vec![[0f32; 2]; count];
            let mut peak = 0f64;
            for track in input["tracks"].as_array().unwrap() {
                if track["mute"].as_bool() == Some(true) {
                    continue;
                }
                let audio = dsp::track_audio(input, track, &events, 32000)?;
                for (dest, src) in mix.iter_mut().zip(audio) {
                    for c in 0..2 {
                        peak = peak.max((src[c] as f64).abs());
                        dest[c] += src[c];
                    }
                }
            }
            peak = mix
                .iter()
                .flatten()
                .fold(peak, |p, &s| p.max((s as f64).abs()));
            let gain = 10f64.powf(input["master_gain_db"].as_f64().unwrap_or(-3.) / 20.);
            let ceiling = 10f64.powf(-1. / 20.);
            Ok(Output::Wave(wave(
                &mix,
                if peak * gain > ceiling {
                    ceiling / peak
                } else {
                    gain
                },
            )?))
        }
        "continuous" => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(
                request["wav_base64"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Recording missing"))?,
            )?;
            ensure!(
                bytes.len() <= 15 * 1024 * 1024,
                "Recording exceeds browser limit"
            );
            let (samples, _) = transcribe::decode_wav(&bytes)?;
            let audio = transcribe::voice_to_chip(&samples, &request["transcription"])?;
            Ok(Output::Wave(wave(&audio, 1.)?))
        }
        "layer" => Ok(Output::Json(layers::add_melody_layer(
            input,
            &request["take"],
            request["at_bar"].as_f64().unwrap_or(0.),
        )?)),
        "patch" => Ok(Output::Json(score::apply_patch(
            input,
            &request["operations"],
        )?)),
        "inspect" => Ok(Output::Json(
            json!({"score":input,"score_sha256":score::digest(input),"validation":score::validate(input)?}),
        )),
        "new" => {
            let profile = request["profile"].as_str().unwrap_or("chip4");
            let mut data: Value = serde_json::from_str(if profile == "console8" {
                include_str!("../examples/console8.json")
            } else {
                include_str!("../examples/chip4.json")
            })?;
            data["title"] = json!(request["title"].as_str().unwrap_or("New track"));
            if request["template"] != "lanterns" {
                data["patterns"] = json!({"melody":{"length":data["length"],"notes":[]}});
                data["tracks"] = json!([{"id":"lead","slot":"pulse1","instrument":"lead","clips":[{"pattern":"melody","at":0}]}]);
            }
            score::validate(&data)?;
            Ok(Output::Json(data))
        }
        _ => Err(anyhow!("Unknown browser operation")),
    }
}
