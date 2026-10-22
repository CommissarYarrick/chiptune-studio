//! File-based composition, revision and deterministic audio export API.
use crate::{dsp, score};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct FileLock(File);
impl FileLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        fs::create_dir_all(parent(path))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        FileExt::try_lock_exclusive(&file).context("Busy: another edit or render is active")?;
        Ok(Self(file))
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
pub fn file_hash(path: &Path) -> Result<String> {
    let mut stream = BufReader::new(File::open(path)?);
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let n = stream.read(&mut block)?;
        if n == 0 {
            break;
        }
        hash.update(&block[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn code_hash() -> String {
    let mut hash = Sha256::new();
    for code in [
        include_str!("engine.rs"),
        include_str!("score.rs"),
        include_str!("dsp.rs"),
        include_str!("transcribe.rs"),
        include_str!("server.rs"),
        include_str!("main.rs"),
        include_str!("../Cargo.lock"),
    ] {
        hash.update(code.as_bytes());
    }
    hex::encode(hash.finalize())
}
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::create_dir_all(parent(path))?;
    let temp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut stream = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut stream, value)?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        stream.get_ref().sync_all()?;
        drop(stream);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub fn create(path: &Path, profile: &str, title: &str, template: &str) -> Result<Value> {
    ensure!(
        matches!(template, "lanterns" | "empty"),
        "Template must be lanterns or empty"
    );
    let source = match profile {
        "chip4" => include_str!("../examples/chip4.json"),
        "console8" => include_str!("../examples/console8.json"),
        _ => bail!("Unknown profile"),
    };
    let _lock = FileLock::acquire(&parent(path).join(".score.lock"))?;
    ensure!(
        !path.exists(),
        "Score already exists; use patch to create a revision"
    );
    let mut data: Value = serde_json::from_str(source)?;
    data["title"] = json!(title);
    if template == "empty" {
        data["length"] = json!(16);
        data["patterns"] = json!({"melody":{"length":16,"notes":[]}});
        data["tracks"] = json!([{"id":"lead","slot":"pulse1","instrument":"lead","gain_db":-7,"pan":0,"clips":[{"id":"melody","pattern":"melody","at":0}]}]);
        data["sections"] = json!([{"id":"A","at":0,"length":16}]);
    }
    score::validate(&data)?;
    write_json(path, &data)?;
    inspect(path)
}
pub fn inspect(path: &Path) -> Result<Value> {
    let data = score::load(path)?;
    let mut report = score::validate(&data)?;
    report["path"] = json!(path.canonicalize()?.to_string_lossy());
    report["tracks"] = json!(data["tracks"].as_array().context("Missing tracks")?.iter().map(|t| json!({"id":t["id"],"instrument":t["instrument"],"clips":t["clips"].as_array().map_or(0,Vec::len)})).collect::<Vec<_>>());
    report["score"] = data;
    Ok(report)
}
pub fn patch(path: &Path, operations: &Value, out: &Path, expected_sha256: &str) -> Result<Value> {
    let _lock = FileLock::acquire(&parent(out).join(".score.lock"))?;
    let data = score::load(path)?;
    ensure!(
        score::digest(&data) == expected_sha256,
        "Score changed; inspect again before patching"
    );
    let result = score::apply_patch(&data, operations)?;
    ensure!(
        !out.exists() || out.canonicalize()? == path.canonicalize()?,
        "Revision destination exists"
    );
    write_json(out, &result)?;
    inspect(out)
}

fn wav_writer(path: &Path, rate: u32) -> Result<hound::WavWriter<BufWriter<File>>> {
    Ok(hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?)
}
fn pcm(sample: f32, gain: f64) -> Result<i16> {
    ensure!(sample.is_finite(), "Nonfinite audio");
    Ok(((sample as f64 * gain).clamp(-1., 1.) * 32767.).round_ties_even() as i16)
}
pub fn write_wave(path: &Path, audio: &[[f32; 2]], gain: f64, rate: u32) -> Result<()> {
    let mut writer = wav_writer(path, rate)?;
    for frame in audio {
        for &sample in frame {
            writer.write_sample(pcm(sample, gain)?)?;
        }
    }
    writer.finalize()?;
    Ok(())
}
/// Native, deterministic Ogg Vorbis export. No external converter process.
pub fn write_ogg(
    path: &Path,
    audio: &[[f32; 2]],
    rate: u32,
    serial: i32,
    title: &str,
) -> Result<()> {
    use std::num::{NonZeroU8, NonZeroU32};
    use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};
    let sink = BufWriter::new(File::create(path)?);
    let mut builder = VorbisEncoderBuilder::new_with_serial(
        NonZeroU32::new(rate).context("Invalid sample rate")?,
        NonZeroU8::new(2).unwrap(),
        sink,
        serial,
    );
    builder.bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
        target_quality: 0.6,
    });
    builder.comment_tag("TITLE", title)?;
    builder.comment_tag("ENCODER", format!("Chiptune Studio {VERSION}"))?;
    let mut encoder = builder.build()?;
    for block in audio.chunks(1024) {
        let left: Vec<f32> = block.iter().map(|f| f[0]).collect();
        let right: Vec<f32> = block.iter().map(|f| f[1]).collect();
        encoder.encode_audio_block([left.as_slice(), right.as_slice()])?;
    }
    encoder.finish()?.flush()?;
    Ok(())
}
fn metrics(audio: &[[f32; 2]]) -> Value {
    let mut peak = 0f64;
    let mut squares = 0.;
    let mut dc = [0.; 2];
    for frame in audio {
        for c in 0..2 {
            let x = frame[c] as f64;
            peak = peak.max(x.abs());
            squares += x * x;
            dc[c] += x;
        }
    }
    let count = audio.len().max(1) as f64;
    for x in &mut dc {
        *x /= count;
    }
    let rms = (squares / (2. * count)).sqrt();
    let seam = if audio.is_empty() {
        0.
    } else {
        (audio[0][0] - audio[audio.len() - 1][0])
            .abs()
            .max((audio[0][1] - audio[audio.len() - 1][1]).abs()) as f64
    };
    let mut derivatives = Vec::new();
    for i in (1..audio.len()).step_by((audio.len() / 50000).max(1)) {
        for c in 0..2 {
            derivatives.push((audio[i][c] - audio[i - 1][c]).abs() as f64);
        }
    }
    derivatives.sort_by(f64::total_cmp);
    let p99 = if derivatives.is_empty() {
        0.
    } else {
        let p = (derivatives.len() - 1) as f64 * 0.99;
        let i = p.floor() as usize;
        derivatives[i]
            + (derivatives[(i + 1).min(derivatives.len() - 1)] - derivatives[i]) * (p - i as f64)
    };
    json!({"peak":peak,"rms":rms,"rms_dbfs":20.*rms.max(1e-12).log10(),"dc":dc,"seam_delta":seam,"seam_vs_p99_derivative":if p99>0. {seam/p99.max(1e-9)} else {0.}})
}
fn vlq(mut number: u32) -> Vec<u8> {
    let mut bytes = vec![(number & 127) as u8];
    number >>= 7;
    while number > 0 {
        bytes.insert(0, 128 | (number & 127) as u8);
        number >>= 7;
    }
    bytes
}
fn write_midi(
    path: &Path,
    data: &Value,
    events: &[score::Event],
    start: f64,
    end: f64,
) -> Result<()> {
    let tempo = (60_000_000. / data["bpm"].as_f64().unwrap()).round_ties_even() as u32;
    let num = data["meter"][0].as_u64().unwrap_or(4) as u8;
    let den = data["meter"][1].as_u64().unwrap_or(4) as f64;
    let mut header = vec![0, 255, 81, 3];
    header.extend_from_slice(&tempo.to_be_bytes()[1..]);
    header.extend_from_slice(&[0, 255, 88, 4, num, den.log2() as u8, 24, 8, 0, 255, 47, 0]);
    let mut tracks = vec![header];
    for (i, track) in data["tracks"].as_array().unwrap().iter().enumerate() {
        let channel = if track["slot"] == "noise" { 9 } else { i as u8 };
        let id = track["id"].as_str().unwrap();
        let mut rows: Vec<(u32, u8, [u8; 3])> = Vec::new();
        for e in events
            .iter()
            .filter(|e| e.track == id && e.at < end && e.at + e.duration > start)
        {
            let on = ((e.at.max(start) - start) * 480.).round_ties_even() as u32;
            let off = (((e.at + e.duration).min(end) - start) * 480.).round_ties_even() as u32;
            rows.push((
                on,
                1,
                [
                    0x90 | channel,
                    e.note as u8,
                    (e.velocity * 127.).round_ties_even().clamp(1., 127.) as u8,
                ],
            ));
            rows.push((off.max(on + 1), 0, [0x80 | channel, e.note as u8, 0]));
        }
        rows.sort_unstable();
        let mut body = vec![0, 255, 3];
        body.extend(vlq(id.len() as u32));
        body.extend_from_slice(id.as_bytes());
        body.extend_from_slice(&[0, 0xc0 | channel, 80]);
        let mut previous = 0;
        for (tick, _, msg) in rows {
            body.extend(vlq(tick - previous));
            body.extend(msg);
            previous = tick;
        }
        body.extend_from_slice(&[0, 255, 47, 0]);
        tracks.push(body);
    }
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(b"MThd")?;
    file.write_all(&6u32.to_be_bytes())?;
    file.write_all(&1u16.to_be_bytes())?;
    file.write_all(&(tracks.len() as u16).to_be_bytes())?;
    file.write_all(&480u16.to_be_bytes())?;
    for track in tracks {
        file.write_all(b"MTrk")?;
        file.write_all(&(track.len() as u32).to_be_bytes())?;
        file.write_all(&track)?;
    }
    file.flush()?;
    Ok(())
}
#[derive(Default)]
struct Scratch(Vec<(String, PathBuf)>);
impl Drop for Scratch {
    fn drop(&mut self) {
        for (_, p) in &self.0 {
            let _ = fs::remove_file(p);
        }
    }
}

pub fn render(
    path: &Path,
    out: &Path,
    sample_rate: u32,
    from_bar: f64,
    bars: Option<f64>,
    stems: bool,
    solo: &[String],
) -> Result<Value> {
    ensure!(
        (16000..=48000).contains(&sample_rate),
        "Sample rate must be 16000..48000"
    );
    let mut data = score::load(path)?;
    if !solo.is_empty() {
        let tracks = data["tracks"].as_array_mut().unwrap();
        for id in solo {
            ensure!(
                tracks.iter().any(|t| t["id"] == id.as_str()),
                "Unknown solo track: {id}"
            );
        }
        for t in tracks {
            t["mute"] = json!(!solo.iter().any(|id| t["id"] == id.as_str()));
        }
    }
    let validation = score::validate(&data)?;
    let events = score::compile(&data)?;
    let bpm = data["bpm"].as_f64().unwrap();
    let duration = validation["seconds"].as_f64().unwrap();
    let max_note = events
        .iter()
        .map(|e| e.duration * 60. / bpm + 2.)
        .fold(0., f64::max);
    let estimated = duration * sample_rate as f64 * 32. + max_note * sample_rate as f64 * 96.;
    ensure!(
        duration <= 240. && estimated <= 512. * 1024. * 1024.,
        "Render budget exceeded (240s / estimated 512MiB)"
    );
    let beats = data["length"].as_f64().unwrap();
    let bpb = score::beat_per_bar(&data);
    ensure!(
        from_bar.is_finite() && from_bar >= 0. && from_bar < beats / bpb,
        "Preview start outside score"
    );
    if let Some(n) = bars {
        ensure!(
            n.is_finite() && (0.0625..=1024.).contains(&n),
            "Invalid preview length"
        );
    }
    let start_beat = from_bar * bpb;
    let end_beat = bars.map_or(beats, |n| start_beat + n * bpb);
    ensure!(end_beat <= beats + 1e-8, "Preview window outside score");
    let frames = (duration * sample_rate as f64).round_ties_even() as usize;
    let start = (start_beat * 60. / bpm * sample_rate as f64).round_ties_even() as usize;
    let end = ((end_beat * 60. / bpm * sample_rate as f64).round_ties_even() as usize).min(frames);
    ensure!(start < end, "Empty preview window");
    fs::create_dir_all(out)?;
    let out = out.canonicalize()?;
    let _lock = FileLock::acquire(&out.join(".render.lock"))?;
    let mut selected_solo = solo.to_vec();
    selected_solo.sort();
    let key = json!({"score_sha256":score::digest(&data),"engine_sha256":code_hash(),"sample_rate":sample_rate,"from_bar":from_bar,"bars":bars,"stems":stems,"solo":selected_solo});
    let manifest_path = out.join("manifest.json");
    if let Ok(bytes) = fs::read(&manifest_path)
        && let Ok(mut old) = serde_json::from_slice::<Value>(&bytes)
    {
        let valid = old["files"]
            .as_object()
            .filter(|m| !m.is_empty())
            .is_some_and(|files| {
                files.iter().all(|(name, hash)| {
                    Path::new(name).components().count() == 1
                        && !name.contains(['/', '\\'])
                        && file_hash(&out.join(name))
                            .is_ok_and(|h| Some(h.as_str()) == hash.as_str())
                })
            });
        if old["cache_key"] == key && valid {
            old["cached"] = json!(true);
            old["manifest"] = json!(manifest_path.to_string_lossy());
            return Ok(old);
        }
    }
    let began = Instant::now();
    let mut mix = vec![[0f32; 2]; frames];
    let mut stem_peak = 0f64;
    let mut scratch = Scratch::default();
    for track in data["tracks"].as_array().unwrap() {
        let audio = dsp::track_audio(&data, track, &events, sample_rate)?;
        ensure!(audio.len() == frames, "Synth returned wrong frame count");
        for (a, b) in mix.iter_mut().zip(&audio) {
            for c in 0..2 {
                ensure!(b[c].is_finite(), "Nonfinite audio");
                stem_peak = stem_peak.max((b[c] as f64).abs());
                a[c] += b[c];
            }
        }
        if stems {
            let temp = out.join(format!(".{}.f32", uuid::Uuid::new_v4().simple()));
            scratch
                .0
                .push((track["id"].as_str().unwrap().to_owned(), temp.clone()));
            let mut stream = BufWriter::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?,
            );
            for frame in &audio[start..end] {
                for sample in frame {
                    stream.write_all(&sample.to_le_bytes())?;
                }
            }
            stream.flush()?;
        }
    }
    let peak = mix
        .iter()
        .flatten()
        .fold(stem_peak, |p, &s| p.max((s as f64).abs()));
    let mut gain = 10f64.powf(data["master_gain_db"].as_f64().unwrap_or(-3.) / 20.);
    let target = 10f64.powf(-1. / 20.);
    if peak * gain > target {
        gain = target / peak;
    }
    for frame in &mut mix {
        for s in frame {
            *s = (*s as f64 * gain) as f32;
        }
    }
    let selected = &mix[start..end];
    let report = metrics(selected);
    write_wave(&out.join("mix.wav"), selected, 1., sample_rate)?;
    let serial = i32::from_le_bytes(
        Sha256::digest(serde_json::to_vec(&key)?)[..4]
            .try_into()
            .unwrap(),
    );
    write_ogg(
        &out.join("mix.ogg"),
        selected,
        sample_rate,
        serial,
        data["title"].as_str().unwrap(),
    )?;
    let mut outputs = vec!["mix.wav".to_owned(), "mix.ogg".to_owned()];
    for (id, temp) in &scratch.0 {
        let name = format!("stem_{id}.wav");
        let mut reader = BufReader::new(File::open(temp)?);
        let mut writer = wav_writer(&out.join(&name), sample_rate)?;
        let mut bytes = [0; 4];
        for _ in 0..(end - start) * 2 {
            reader.read_exact(&mut bytes)?;
            writer.write_sample(pcm(f32::from_le_bytes(bytes), gain)?)?;
        }
        writer.finalize()?;
        outputs.push(name);
    }
    write_midi(&out.join("score.mid"), &data, &events, start_beat, end_beat)?;
    outputs.push("score.mid".into());
    write_json(&out.join("score.json"), &data)?;
    outputs.push("score.json".into());
    write_json(
        &out.join("timeline.json"),
        &json!({"units":"quarter-note beats","sections":data.get("sections").cloned().unwrap_or(json!([])),"events":events}),
    )?;
    outputs.push("timeline.json".into());
    let mut warnings = validation["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if report["rms"].as_f64().unwrap_or(0.) < 1e-5 {
        warnings.push(json!("Silent or nearly silent render"));
    }
    let full_loop = start == 0 && end == frames;
    if full_loop && report["seam_vs_p99_derivative"].as_f64().unwrap_or(0.) > 4. {
        warnings.push(json!("Loop boundary spike; inspect the loop by ear"));
    }
    let mut files = serde_json::Map::new();
    for name in outputs {
        files.insert(name.clone(), json!(file_hash(&out.join(name))?));
    }
    let mut manifest = json!({"engine":VERSION,"implementation":"rust","cache_key":key,"cached":false,"title":data["title"],"profile":data["profile"],"frames":end-start,"sample_rate":sample_rate,"seconds":(end-start) as f64/sample_rate as f64,"full_loop":full_loop,"window_beats":[start_beat,end_beat],"applied_gain":gain,"max_gated_voices":validation["max_gated_voices"],"metrics":report,"warnings":warnings,"files":files,"render_seconds":began.elapsed().as_secs_f64(),"mix":out.join("mix.wav").to_string_lossy(),"output":out.to_string_lossy()});
    manifest["ogg"] = json!(out.join("mix.ogg").to_string_lossy());
    manifest["ogg_quality"] = json!(0.6);
    write_json(&manifest_path, &manifest)?;
    manifest["manifest"] = json!(manifest_path.to_string_lossy());
    Ok(manifest)
}
pub fn compare(paths: &[PathBuf]) -> Result<Value> {
    let mut reports = Vec::new();
    for p in paths {
        let m: Value = serde_json::from_slice(&fs::read(p)?)?;
        reports.push(json!({"title":m["title"],"profile":m["profile"],"mix":m["mix"],"metrics":m["metrics"],"warnings":m["warnings"],"cache_key":m["cache_key"]}));
    }
    Ok(json!({"renders":reports,"note":"Metrics are technical checks, not an artistic rating."}))
}
pub fn describe() -> Value {
    json!({"application":"chiptune-studio","engine":VERSION,"implementation":"rust","profiles":{"chip4":{"slots":["pulse1","pulse2","triangle","noise"],"voices":4,"description":"Four monophonic chip-inspired channels."},"console8":{"slots":null,"voices":8,"description":"Eight gated voices, stereo wavetable and chip instruments."}},"instruments":score::default_bank(),"format":{"version":1,"time":"quarter-note beats, zero-based","meter":"[numerator, denominator]","note":"MIDI 0..127 or C4/F#3/Bb2; lists are chords","patterns":"named {length,notes:[id,at,note,duration,velocity,instrument?]}","tracks":"id,slot?,instrument,gain_db,pan,mute?,clips:[id,pattern,at,repeat?,transpose?,velocity?]","instrument_fields":"wave,duty,adsr[a_s,d_s,sustain,r_s],vibrato[hz,cents,delay_s],bits,table,arpeggio,arp_hz,pitch_slide,slide_ms","patch":"RFC6902 add/remove/replace/test; requires current score_sha256"},"workflow":["create or transcribe","inspect","patch a revision","validate","render a short preview","listen and revise","render full mix and stems"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("chip-rust-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn failed_revision_preserves_source() {
        let p = temp();
        let src = p.join("s.json");
        create(&src, "chip4", "Original", "empty").unwrap();
        let before = fs::read(&src).unwrap();
        assert!(
            patch(
                &src,
                &json!([{"op":"replace","path":"/title","value":"Changed"}]),
                &p.join("revision.json"),
                "stale"
            )
            .is_err()
        );
        assert_eq!(before, fs::read(&src).unwrap());
        assert!(!p.join("revision.json").exists());
    }
    #[test]
    fn render_cache_and_tamper_repair() {
        let p = temp();
        let src = p.join("s.json");
        create(&src, "chip4", "Test", "lanterns").unwrap();
        let out = p.join("render");
        let r = render(&src, &out, 16000, 0., Some(0.25), true, &[]).unwrap();
        assert_eq!(r["frames"], json!(7273));
        assert_eq!(
            render(&src, &out, 16000, 0., Some(0.25), true, &[]).unwrap()["cached"],
            true
        );
        fs::write(out.join("mix.wav"), b"bad").unwrap();
        assert_eq!(
            render(&src, &out, 16000, 0., Some(0.25), true, &[]).unwrap()["cached"],
            false
        );
        assert_eq!(&fs::read(out.join("score.mid")).unwrap()[..4], b"MThd");
    }
    #[test]
    fn lock_rejects_concurrent_writer() {
        let p = temp().join(".lock");
        let _guard = FileLock::acquire(&p).unwrap();
        assert!(FileLock::acquire(&p).is_err());
    }
}
