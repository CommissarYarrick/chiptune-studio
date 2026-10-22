#[cfg(feature = "native")]
use crate::engine;
use crate::score;
use anyhow::{Result, anyhow};
use rustfft::{FftPlanner, num_complex::Complex};
use serde_json::{Value, json};
#[cfg(feature = "native")]
use std::{fs, path::Path};

fn number(v: f64, lo: f64, hi: f64, name: &str) -> Result<f64> {
    if !v.is_finite() || v < lo || v > hi {
        Err(anyhow!("{} must be between {} and {}", name, lo, hi))
    } else {
        Ok(v)
    }
}
fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) * 0.5
    }
}
#[cfg(feature = "native")]
fn read_audio(path: &Path) -> Result<(Vec<f32>, Value)> {
    decode_wav(&fs::read(path)?)
}

/// Decode and normalize a PCM WAV entirely in memory, for native and browser use.
pub fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, Value)> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
        .map_err(|_| anyhow!("Use a PCM WAV recording"))?;
    let spec = reader.spec();
    let count = reader.duration() as usize;
    let rate = spec.sample_rate;
    let channels = spec.channels as usize;
    let width = spec.bits_per_sample.div_ceil(8);
    if !matches!(channels, 1 | 2)
        || !matches!(width, 1..=4)
        || !(8000..=192000).contains(&rate)
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(anyhow!("Use mono/stereo PCM WAV, 8..192kHz"));
    }
    let seconds = count as f64 / rate as f64;
    if !(0.15..=60.0).contains(&seconds) {
        return Err(anyhow!("Take duration must be 0.15..60 seconds"));
    }
    let raw: Vec<f64> = match spec.bits_per_sample {
        8 => reader
            .samples::<i8>()
            .map(|x| x.map(|v| v as f64 / 128.0))
            .collect::<std::result::Result<_, _>>()?,
        16 => reader
            .samples::<i16>()
            .map(|x| x.map(|v| v as f64 / 32768.0))
            .collect::<std::result::Result<_, _>>()?,
        24 => reader
            .samples::<i32>()
            .map(|x| x.map(|v| v as f64 / 8388608.0))
            .collect::<std::result::Result<_, _>>()?,
        32 => reader
            .samples::<i32>()
            .map(|x| x.map(|v| v as f64 / 2147483648.0))
            .collect::<std::result::Result<_, _>>()?,
        _ => return Err(anyhow!("Unsupported PCM bit depth")),
    };
    if raw.len() != count * channels {
        return Err(anyhow!("Truncated WAV"));
    }
    let mut mono: Vec<f64> = raw
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f64>() / channels as f64)
        .collect();
    let mean = mono.iter().sum::<f64>() / mono.len() as f64;
    mono.iter_mut().for_each(|x| *x -= mean);
    if rate != 16000 {
        mono = resample(&mono, rate, 16000);
    }
    Ok((
        mono.into_iter().map(|x| x as f32).collect(),
        json!({"source_rate":rate,"channels":channels,"seconds":seconds}),
    ))
}
fn resample(input: &[f64], from: u32, to: u32) -> Vec<f64> {
    let size = ((input.len() as f64 * to as f64 / from as f64).round_ties_even() as usize).max(1);
    let mut planner = FftPlanner::<f64>::new();
    let mut data: Vec<Complex<f64>> = input.iter().map(|&x| Complex::new(x, 0.0)).collect();
    planner.plan_fft_forward(input.len()).process(&mut data);
    let mut spectrum = vec![Complex::new(0.0, 0.0); size];
    spectrum[0] = data[0];
    let limit = (input.len() / 2).min(size / 2);
    for k in 1..=limit {
        if k < size {
            spectrum[k] = data[k];
        }
        if k < size && input.len() > k {
            spectrum[size - k] = data[input.len() - k];
        }
    }
    planner.plan_fft_inverse(size).process(&mut spectrum);
    spectrum
        .into_iter()
        .map(|x| x.re / input.len() as f64)
        .collect()
}
fn yin(frame: &[f32], rate: f64, low: f64, high: f64) -> (f64, f64) {
    let n = frame.len();
    let mean = frame.iter().map(|&x| x as f64).sum::<f64>() / n as f64;
    let x: Vec<f64> = frame.iter().map(|&v| v as f64 - mean).collect();
    let mut len = 1usize;
    while len < 2 * n {
        len *= 2;
    }
    let mut planner = FftPlanner::<f64>::new();
    let mut spec: Vec<Complex<f64>> = x.iter().map(|&v| Complex::new(v, 0.0)).collect();
    spec.resize(len, Complex::new(0.0, 0.0));
    let fft = planner.plan_fft_forward(len);
    fft.process(&mut spec);
    for c in &mut spec {
        *c = c.conj() * *c;
    }
    planner.plan_fft_inverse(len).process(&mut spec);
    let corr: Vec<f64> = spec[..n].iter().map(|c| c.re / len as f64).collect();
    let mut squares = vec![0.0; n + 1];
    for i in 0..n {
        squares[i + 1] = squares[i] + x[i] * x[i];
    }
    let last = (rate / low).floor() as usize;
    let first = (rate / high).floor().max(2.0) as usize;
    if first >= last || x.iter().map(|v| v.abs()).fold(0.0, f64::max) < 1e-7 {
        return (0.0, 0.0);
    }
    let mut diff = vec![0.0; last + 1];
    for tau in 1..=last {
        diff[tau] = (squares[n - tau] + squares[n] - squares[tau] - 2.0 * corr[tau]).max(0.0);
    }
    let mut cumulative = 0.0;
    let mut cmnd = vec![1.0; last + 1];
    for tau in 1..=last {
        cumulative += diff[tau];
        cmnd[tau] = if cumulative > 1e-20 {
            diff[tau] * tau as f64 / cumulative
        } else {
            1.0
        };
    }
    let mut candidate = None;
    let mut tau = first;
    while tau < last {
        if cmnd[tau] < 0.15 {
            while tau + 1 < last && cmnd[tau + 1] < cmnd[tau] {
                tau += 1;
            }
            candidate = Some(tau);
            break;
        }
        tau += 1;
    }
    let c = candidate.unwrap_or_else(|| {
        (first..last)
            .min_by(|&a, &b| cmnd[a].total_cmp(&cmnd[b]))
            .unwrap_or(first)
    });
    let conf = (1.0 - cmnd[c]).clamp(0.0, 1.0);
    let denom = cmnd[c - 1] - 2.0 * cmnd[c] + cmnd[c + 1];
    let corr = if denom.abs() > 1e-12 {
        (0.5 * (cmnd[c - 1] - cmnd[c + 1]) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    (rate / (c as f64 + corr), conf)
}

pub fn detect(samples: &[f32], bpm: f64, quantize: f64) -> Result<Value> {
    number(bpm, 30.0, 300.0, "bpm")?;
    validate_quantize(quantize)?;
    if samples.iter().any(|x| !x.is_finite()) {
        return Err(anyhow!("Recording contains nonfinite samples"));
    }
    const RATE: usize = 16000;
    const FRAME: usize = 1024;
    const HOP: usize = 160;
    if samples.len() < FRAME {
        return Err(anyhow!("Recording is too short"));
    }
    let starts: Vec<usize> = (0..=samples.len() - FRAME).step_by(HOP).collect();
    let energies: Vec<f64> = starts
        .iter()
        .map(|&s| {
            (samples[s..s + FRAME]
                .iter()
                .map(|&x| (x as f64).powi(2))
                .sum::<f64>()
                / FRAME as f64)
                .sqrt()
        })
        .collect();
    let mut sorted = energies.clone();
    let p20 = if sorted.is_empty() {
        0.0
    } else {
        sorted.sort_by(f64::total_cmp);
        sorted[((sorted.len() - 1) * 20) / 100]
    };
    // A quiet sustained note must not become its own noise floor. Cap the
    // noise estimate relative to the loudest frame; periodicity rejects noise.
    let peak_energy = energies.iter().copied().fold(0.0, f64::max);
    let gate = 1e-6f64.max((p20 * 2.0).min(peak_energy * 0.04));
    let mut pitches: Vec<Option<f64>> = Vec::new();
    let mut confs = Vec::new();
    let mut curve = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let (hz, conf) = if energies[i] >= gate {
            yin(&samples[s..s + FRAME], RATE as f64, 65.0, 2000.0)
        } else {
            (0.0, 0.0)
        };
        let midi = if hz > 0.0 && conf >= 0.65 {
            Some(69.0 + 12.0 * (hz / 440.0).log2())
        } else {
            None
        };
        let note = midi.filter(|m| (0.0..=127.0).contains(m));
        pitches.push(note);
        confs.push(conf);
        curve.push(json!({"seconds":(s as f64+FRAME as f64/2.0)/RATE as f64,"hz":(hz*1000.0).round()/1000.0,"confidence":(conf*10000.0).round()/10000.0}));
    }
    let mut smooth = pitches.clone();
    for i in 0..pitches.len() {
        if pitches[i].is_some() {
            let mut window: Vec<f64> = pitches[i.saturating_sub(2)..(i + 3).min(pitches.len())]
                .iter()
                .flatten()
                .copied()
                .collect();
            smooth[i] = Some(median(&mut window));
        } else if i > 0
            && i + 1 < pitches.len()
            && let (Some(a), Some(b)) = (pitches[i - 1], pitches[i + 1])
            && (a - b).abs() < 0.7
        {
            smooth[i] = Some((a + b) * 0.5);
        }
    }
    // Quantizing every frame before segmentation fragments normal vocal vibrato
    // at semitone boundaries. Apply hysteresis first, then round each note.
    let mut held: Option<i32> = None;
    let labels: Vec<Option<i32>> = smooth
        .iter()
        .map(|pitch| {
            held = pitch.map(|m| match held {
                Some(note) if (m - note as f64).abs() < 0.8 => note,
                _ => m.round_ties_even() as i32,
            });
            held
        })
        .collect();
    let mut segments = Vec::new();
    let mut start = 0;
    for i in 1..=labels.len() {
        if i < labels.len() && labels[i] == labels[start] {
            continue;
        }
        if labels[start].is_some() && (i - start) * HOP >= ((0.04 * RATE as f64) as usize) {
            let mut values: Vec<f64> = smooth[start..i].iter().flatten().copied().collect();
            let note = median(&mut values).round_ties_even() as i32;
            let mut cs = confs[start..i].to_vec();
            segments.push(json!({"note":note,"start":((start*HOP) as f64+FRAME as f64/2.0-HOP as f64/2.0).max(0.0)/RATE as f64,"end":(i as f64*HOP as f64+FRAME as f64/2.0-HOP as f64/2.0)/RATE as f64,"confidence":median(&mut cs)}));
        }
        start = i;
    }
    if segments.is_empty() {
        return Err(anyhow!(
            "No stable melody detected: record one clear voice or whistle closer to the microphone"
        ));
    }
    let offset = segments[0]["start"].as_f64().unwrap_or(0.0);
    let mut notes: Vec<Value> = Vec::new();
    for seg in &segments {
        let snap = |seconds: f64| {
            let beats = (seconds - offset) * bpm / 60.0;
            if quantize == 0.0 {
                beats
            } else {
                (beats / quantize).round_ties_even() * quantize
            }
        };
        let minimum = if quantize == 0.0 {
            HOP as f64 / RATE as f64 * bpm / 60.0
        } else {
            quantize
        };
        let at = snap(seg["start"].as_f64().unwrap_or(0.0)).max(0.0);
        let mut end = snap(seg["end"].as_f64().unwrap_or(0.0)).max(at + minimum);
        if let Some(prev) = notes.last_mut() {
            let pa = prev["at"].as_f64().unwrap_or(0.0);
            let pe = pa + prev["duration"].as_f64().unwrap_or(0.0);
            if at < pe {
                let actual_at = if at <= pa { pe } else { at };
                if at > pa {
                    prev["duration"] = json!(at - pa);
                }
                end = end.max(actual_at + minimum);
                notes.push(json!({"id":format!("hum_{}",notes.len()),"at":actual_at,"note":seg["note"],"duration":end-actual_at,"velocity":0.82}));
                continue;
            }
        }
        notes.push(json!({"id":format!("hum_{}",notes.len()),"at":at,"note":seg["note"],"duration":end-at,"velocity":0.82}));
    }
    let mut cs: Vec<f64> = segments
        .iter()
        .map(|s| s["confidence"].as_f64().unwrap_or(0.0))
        .collect();
    Ok(
        json!({"notes":notes,"raw_segments":segments,"pitch_curve":curve,"leading_seconds_trimmed":offset,"quantize_beats":quantize,"timing_mode":if quantize == 0.0 {"original"} else {"grid"},"bpm":bpm,"median_confidence":median(&mut cs),"voiced_fraction":pitches.iter().filter(|p| p.is_some()).count() as f64 / pitches.len() as f64,"analysis_rate":RATE,"warnings":["Pitch is transcribed to semitones; slides and timbre are not preserved. Compare with the original take."]}),
    )
}

fn validate_quantize(quantize: f64) -> Result<()> {
    if quantize == 0.0 {
        Ok(())
    } else {
        number(quantize, 0.0625, 2.0, "quantize (0 preserves timing)").map(|_| ())
    }
}

/// Resynthesize a monophonic take without converting its melody to MIDI notes.
/// Input is mono 16 kHz; output is stereo 32 kHz, with the complete take retained.
/// Voiceless sounds are gated, rather than replayed or turned into pitched noise.
pub fn voice_to_chip(samples: &[f32], analysis: &Value) -> Result<Vec<[f32; 2]>> {
    let curve = analysis["pitch_curve"]
        .as_array()
        .ok_or_else(|| anyhow!("Pitch analysis missing"))?;
    if curve.is_empty() || samples.is_empty() || samples.iter().any(|v| !v.is_finite()) {
        return Err(anyhow!("Invalid take for continuous resynthesis"));
    }
    let times: Vec<f64> = curve
        .iter()
        .map(|f| f["seconds"].as_f64().unwrap_or(0.))
        .collect();
    let pitches: Vec<Option<f64>> = curve
        .iter()
        .map(|f| {
            let hz = f["hz"].as_f64().unwrap_or(0.);
            ((65. ..=2000.).contains(&hz) && f["confidence"].as_f64().unwrap_or(0.) >= 0.65)
                .then(|| hz.log2())
        })
        .collect();
    let mut smooth = pitches.clone();
    for i in 0..pitches.len() {
        if pitches[i].is_some() {
            let mut window: Vec<f64> = pitches[i.saturating_sub(1)..(i + 2).min(pitches.len())]
                .iter()
                .flatten()
                .copied()
                .collect();
            smooth[i] = Some(median(&mut window));
        }
    }
    // Bridge only short dropouts between similar pitches, not real rests or jumps.
    let mut i = 0;
    while i < smooth.len() {
        if smooth[i].is_some() {
            i += 1;
            continue;
        }
        let start = i;
        while i < smooth.len() && smooth[i].is_none() {
            i += 1;
        }
        if start > 0
            && i < smooth.len()
            && i - start <= 3
            && let (Some(a), Some(b)) = (smooth[start - 1], smooth[i])
            && (a - b).abs() < 2. / 12.
        {
            for (k, pitch) in smooth.iter_mut().enumerate().take(i).skip(start) {
                *pitch = Some(a + (b - a) * (k - start + 1) as f64 / (i - start + 1) as f64);
            }
        }
    }
    let mut env = Vec::with_capacity(samples.len());
    let mut power = 0.;
    for &sample in samples {
        let value = (sample as f64).powi(2);
        let seconds: f64 = if value > power { 0.004 } else { 0.012 };
        power += (1. - (-1. / (16000. * seconds)).exp()) * (value - power);
        env.push(power.sqrt());
    }
    let reference = env.iter().copied().fold(0., f64::max).max(1e-8);
    let mut output = Vec::with_capacity(samples.len() * 2);
    let mut frame = 0;
    let mut phase = 0.;
    let mut voice = 0.;
    let mut frequency = 220.;
    let rate = 32000.;
    for n in 0..samples.len() * 2 {
        let time = n as f64 / rate;
        while frame + 1 < times.len() && times[frame + 1] <= time {
            frame += 1;
        }
        let next = (frame + 1).min(smooth.len() - 1);
        let t = if times[next] > times[frame] {
            ((time - times[frame]) / (times[next] - times[frame])).clamp(0., 1.)
        } else {
            0.
        };
        let (pitch, target) = match (smooth[frame], smooth[next]) {
            (Some(a), Some(b)) => (Some(a + (b - a) * t), 1.),
            (Some(a), None) => (Some(a), 1. - t),
            (None, Some(b)) => (Some(b), t),
            _ => (None, 0.),
        };
        if let Some(p) = pitch {
            frequency = 2f64.powf(p);
        }
        voice += (1. - (-1. / (rate * 0.003)).exp()) * (target - voice);
        let step = frequency / rate;
        let pulse = (if phase < 0.5 { 1. } else { -1. }) + crate::dsp::poly_blep(phase, step)
            - crate::dsp::poly_blep((phase + 0.5) % 1., step);
        phase = (phase + step) % 1.;
        let index = n / 2;
        let amplitude = (env[index] / reference).clamp(0., 1.);
        let edge = (n as f64 / (rate * 0.004))
            .min((samples.len() * 2 - 1 - n) as f64 / (rate * 0.004))
            .clamp(0., 1.);
        let value = (pulse * amplitude * voice * edge * 0.32) as f32;
        output.push([value, value]);
    }
    Ok(output)
}

fn estimate_key(notes: &[Value]) -> Result<(usize, &'static str, f64)> {
    let mut chroma = [0.0; 12];
    for n in notes {
        let pitch = score::pitch(n.get("note").ok_or_else(|| anyhow!("note missing pitch"))?)?;
        let dur = n.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
        chroma[pitch.rem_euclid(12) as usize] += dur;
    }
    let major = [
        6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
    ];
    let minor = [
        6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
    ];
    let mut scores = Vec::new();
    for (mode, profile) in [("major", major), ("minor", minor)] {
        let mean = profile.iter().sum::<f64>() / 12.0;
        let norm = profile
            .iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f64>()
            .sqrt();
        for root in 0..12 {
            let mut val = 0.0;
            for i in 0..12 {
                val += chroma[i] * (profile[(i + 12 - root) % 12] - mean) / norm;
            }
            scores.push((val, root, mode));
        }
    }
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    let value = scores[0].0;
    Ok((
        scores[0].1,
        scores[0].2,
        (value - scores[1].0) / value.abs().max(1e-8),
    ))
}
pub fn accompany(input: &Value) -> Result<(Value, Value)> {
    let mut result = input.clone();
    let backing_ids = ["bass", "arpeggio", "harmony", "percussion"];
    let mut tracks: Vec<Value> = result["tracks"]
        .as_array()
        .ok_or_else(|| anyhow!("tracks missing"))?
        .iter()
        .filter(|t| !backing_ids.contains(&t["id"].as_str().unwrap_or("")))
        .cloned()
        .collect();
    if !result["patterns"]["melody"].is_object() {
        return Err(anyhow!("melody pattern missing"));
    }
    result["tracks"] = json!(tracks);
    let notes: Vec<Value> = score::compile(&result)?
        .iter()
        .map(|e| json!({"at":e.at,"duration":e.duration,"note":e.note}))
        .collect();
    let (tonic, mode, confidence) = estimate_key(&notes)?;
    let intervals = if mode == "minor" {
        vec![(0, 3), (5, 3), (7, 4), (8, 4)]
    } else {
        vec![(0, 4), (5, 4), (7, 4), (9, 3)]
    };
    let candidates: Vec<Vec<i32>> = intervals
        .iter()
        .map(|&(root, third)| {
            let r = (tonic as i32 + root) % 12;
            vec![r, (r + third) % 12, (r + 7) % 12]
        })
        .collect();
    let bars = (result["length"].as_f64().unwrap_or(4.0) / 4.0).ceil() as usize;
    let mut dp: Vec<Vec<f64>> = Vec::new();
    let mut back: Vec<Vec<usize>> = Vec::new();
    for bar in 0..bars {
        let mut fit = Vec::new();
        for chord in &candidates {
            let mut value = 0.0;
            for n in &notes {
                let at = n["at"].as_f64().unwrap_or(0.0);
                let dur = n["duration"].as_f64().unwrap_or(0.0);
                let overlap = (at + dur).min((bar + 1) as f64 * 4.0) - at.max(bar as f64 * 4.0);
                if overlap > 0.0 {
                    let pc = score::pitch(&n["note"])? % 12;
                    value += overlap
                        * (if (at % 1.0).abs() < 1e-6 { 1.5 } else { 1.0 })
                        * if chord.contains(&pc) { 2.0 } else { -1.0 };
                }
            }
            fit.push(value);
        }
        let mut row = Vec::new();
        let mut prevs = Vec::new();
        for ci in 0..candidates.len() {
            if bar == 0 {
                row.push(fit[ci] + if ci == 0 { 0.3 } else { 0.0 });
                prevs.push(0);
            } else {
                let choices: Vec<f64> = candidates
                    .iter()
                    .enumerate()
                    .map(|(pi, old)| {
                        dp[bar - 1][pi]
                            - 0.02
                                * ((candidates[ci][0] - old[0])
                                    .rem_euclid(12)
                                    .min((old[0] - candidates[ci][0]).rem_euclid(12))
                                    as f64)
                            - if ci == pi { 0.10 } else { 0.0 }
                    })
                    .collect();
                let best = (0..choices.len())
                    .max_by(|&a, &b| choices[a].total_cmp(&choices[b]))
                    .unwrap();
                row.push(choices[best] + fit[ci]);
                prevs.push(best);
            }
        }
        dp.push(row);
        back.push(prevs);
    }
    let mut selected = vec![0; bars];
    if bars > 0 {
        selected[bars - 1] = (0..dp[bars - 1].len())
            .max_by(|&a, &b| dp[bars - 1][a].total_cmp(&dp[bars - 1][b]))
            .unwrap();
        for bar in (1..bars).rev() {
            selected[bar - 1] = back[bar][selected[bar]];
        }
    }
    let profile = result["profile"].as_str().unwrap_or("chip4").to_owned();
    let mut bass =
        json!({"id":"bass","slot":"triangle","instrument":"bass","gain_db":-9,"pan":0,"clips":[]});
    let mut harmony = json!({"id":if profile=="chip4"{"arpeggio"}else{"harmony"},"instrument":if profile=="chip4"{"arp"}else{"warm"},"gain_db":-17,"pan":0.18,"clips":[]});
    if profile == "chip4" {
        harmony["slot"] = json!("pulse2");
    }
    let drums = json!({"id":"percussion","slot":"noise","instrument":"hat","gain_db":-17,"pan":-0.1,"clips":[{"id":"drums","pattern":"rhythm","at":0,"repeat":bars}]});
    let rhythm = json!({"length":4,"notes":[{"id":"d0","at":0,"duration":0.1,"note":36,"instrument":"kick","velocity":0.6},{"id":"d1","at":1,"duration":0.1,"note":38,"instrument":"snare","velocity":0.6},{"id":"d2","at":1.75,"duration":0.1,"note":42,"instrument":"hat","velocity":0.6},{"id":"d3","at":2,"duration":0.1,"note":36,"instrument":"kick","velocity":0.6},{"id":"d4","at":3,"duration":0.1,"note":38,"instrument":"snare","velocity":0.6},{"id":"d5","at":3.75,"duration":0.1,"note":42,"instrument":"hat","velocity":0.6}]});
    result["patterns"]["rhythm"] = rhythm;
    let mut progression = Vec::new();
    for (bar, &chosen) in selected.iter().enumerate() {
        let pcs = &candidates[chosen];
        let root = pcs[0] + 48;
        let third = (pcs[1] - pcs[0]).rem_euclid(12);
        progression.push(json!({"bar":bar,"root_pc":pcs[0],"third":third}));
        let bname = format!("bass_{}", bar);
        let cname = format!("harmony_{}", bar);
        result["patterns"][&bname] = json!({"length":4,"notes":[{"id":"root","at":0,"note":root-12,"duration":1.7,"velocity":0.8},{"id":"fifth","at":2,"note":root-5,"duration":1.7,"velocity":0.7}]});
        let chord = [root, root + third, root + 7];
        let row = if profile == "chip4" {
            let arp_notes:Vec<Value>=[0,1,2,1,0,2,1,2].iter().enumerate().map(|(i,&degree)|json!({"id":format!("a{}",i),"at":i as f64*0.5,"note":chord[degree]+12,"duration":0.32,"velocity":0.65})).collect();
            json!({"length":4,"notes":arp_notes})
        } else {
            json!({"length":4,"notes":[{"id":"pad","at":0,"note":chord,"duration":3.7,"velocity":0.55}]})
        };
        result["patterns"][&cname] = row;
        bass["clips"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":format!("bar_{}",bar),"pattern":bname,"at":bar*4}));
        harmony["clips"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":format!("bar_{}",bar),"pattern":cname,"at":bar*4}));
    }
    // Recorded layers own their voices. Backing must never replace a theme.
    // Prefer a bass and rhythm before adding arpeggios when chip slots are scarce.
    let mut added = Vec::new();
    for backing in [bass, drums, harmony] {
        let available = if profile == "chip4" {
            !tracks.iter().any(|t| t["slot"] == backing["slot"])
        } else {
            let voices = if backing["id"] == "harmony" { 3 } else { 1 };
            tracks.len() + voices <= 8
        };
        if available {
            added.push(backing["id"].clone());
            tracks.push(backing);
        }
    }
    result["tracks"] = json!(tracks);
    let report = json!({"tonic_pc":tonic,"mode":mode,"key_margin":confidence,"progression":progression,"added_tracks":added,"note":"Starter accompaniment; recorded melody layers preserved. Backing uses available profile voices only."});
    score::validate(&result)?;
    Ok((result, report))
}

/// Build an editable melodic score from a pitch analysis without filesystem access.
pub fn melody_score(transcription: &Value, bpm: f64, profile: &str, title: &str) -> Result<Value> {
    number(bpm, 30., 300., "bpm")?;
    if !matches!(profile, "chip4" | "console8") {
        return Err(anyhow!("Unknown profile"));
    }
    let notes = transcription["notes"].as_array().unwrap();
    let end = notes
        .iter()
        .map(|n| n["at"].as_f64().unwrap_or(0.0) + n["duration"].as_f64().unwrap_or(0.0))
        .fold(0.0, f64::max);
    let length = 4.0f64.max((end / 4.0).ceil() * 4.0);
    let mut data = json!({"version":1,"title":title,"profile":profile,"bpm":bpm,"meter":[4,4],"length":length,"seed":731,"master_gain_db":-3,"instruments":score::default_bank(),"patterns":{"melody":{"length":length,"notes":notes}},"tracks":[{"id":"lead","slot":"pulse1","instrument":"lead","gain_db":-7,"pan":0,"clips":[{"id":"hum","pattern":"melody","at":0}]}],"sections":[{"id":"hummed_phrase","at":0,"length":length}]});
    if profile == "console8" {
        let mut lead = data["instruments"]["warm"].clone();
        lead["adsr"] = json!([0.004, 0.14, 0.55, 0.08]);
        data["instruments"]["lead"] = lead;
    }
    // Imported articulation should come from the take, not the demo preset's
    // synthetic vibrato, long release, and plucked decay.
    data["instruments"]["lead"]
        .as_object_mut()
        .unwrap()
        .remove("vibrato");
    data["instruments"]["lead"]["adsr"] = json!([0.004, 0.025, 0.9, 0.012]);
    score::validate(&data)?;
    Ok(data)
}

#[cfg(feature = "native")]
pub fn import_take(
    audio_path: &Path,
    out: &Path,
    bpm: f64,
    quantize: f64,
    profile: &str,
    arrange: bool,
    title: &str,
) -> Result<Value> {
    number(bpm, 30.0, 300.0, "bpm")?;
    validate_quantize(quantize)?;
    if !matches!(profile, "chip4" | "console8") {
        return Err(anyhow!("Unknown profile"));
    }
    let audio_path = audio_path.canonicalize()?;
    let out = if out.is_absolute() {
        out.to_path_buf()
    } else {
        std::env::current_dir()?.join(out)
    };
    let (samples, audio_info) = read_audio(&audio_path)?;
    let transcription = detect(&samples, bpm, quantize)?;
    let mut data = melody_score(&transcription, bpm, profile, title)?;
    let harmony = if arrange {
        let (d, h) = accompany(&data)?;
        data = d;
        Some(h)
    } else {
        None
    };
    score::validate(&data)?;
    let parent = out
        .parent()
        .ok_or_else(|| anyhow!("output path needs a parent directory"))?;
    fs::create_dir_all(parent)?;
    let _lock = engine::FileLock::acquire(&parent.join(".score.lock"))?;
    if out.exists() {
        return Err(anyhow!(
            "Import destination exists; use a new take/revision"
        ));
    }
    let saved = parent.join(format!(
        "take_{}.wav",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    fs::copy(&audio_path, &saved)?;
    let mut meta = transcription.clone();
    meta["audio"] = json!({"source_rate":audio_info["source_rate"],"channels":audio_info["channels"],"seconds":audio_info["seconds"],"original":audio_path.to_string_lossy(),"saved_take":saved.to_string_lossy(),"sha256":engine::file_hash(&saved)?});
    meta["harmony"] = harmony.unwrap_or(Value::Null);
    meta["score"] = json!(out.to_string_lossy());
    let metadata_path = out.with_extension("transcription.json");
    let faithful_path = parent.join("faithful.wav");
    let continuous = voice_to_chip(&samples, &transcription)?;
    engine::write_wave(&faithful_path, &continuous, 1., 32000)?;
    meta["faithful_preview"] = json!(faithful_path.to_string_lossy());
    meta["faithful_preview_mode"] =
        json!("Continuous pitch and recorded envelope; no grid, harmony, or voice samples.");
    engine::write_json(&metadata_path, &meta)?;
    engine::write_json(&out, &data)?;
    let mut result = engine::inspect(&out)?;
    result["transcription"] = meta;
    result["transcription_path"] = json!(metadata_path.to_string_lossy());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;
    #[test]
    fn resampling_preserves_sine_amplitude() {
        for (rate, count) in [(44100, 44100), (16000, 16000)] {
            let x: Vec<f64> = (0..count)
                .map(|i| (2.0 * PI * 440.0 * i as f64 / rate as f64).sin())
                .collect();
            let y = resample(&x, rate as u32, 16000);
            let peak = y.iter().map(|v| v.abs()).fold(0.0, f64::max);
            let rms = (y.iter().map(|v| v * v).sum::<f64>() / y.len() as f64).sqrt();
            assert!((peak - 1.0).abs() < 0.01);
            assert!((rms - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
        }
    }
    #[test]
    fn yin_finds_three_frequencies() {
        for hz in [220.0, 330.0, 440.0] {
            let x: Vec<f32> = (0..1024)
                .map(|i| (2.0 * PI * hz * i as f64 / 16000.0).sin() as f32)
                .collect();
            let (found, confidence) = yin(&x, 16000.0, 65.0, 2000.0);
            assert!((found - hz).abs() < 3.0);
            assert!(confidence > 0.9);
        }
    }
    #[test]
    fn silence_has_no_pitch() {
        assert_eq!(yin(&vec![0.0; 1024], 16000.0, 65.0, 2000.0), (0.0, 0.0));
    }
    #[test]
    fn detects_and_preserves_melody() {
        let mut x = vec![0.0f32; 16000 * 2];
        for (note, start, end) in [(220.0, 0.1, 0.65), (330.0, 0.7, 1.25), (440.0, 1.3, 1.9)] {
            for (i, sample) in x
                .iter_mut()
                .enumerate()
                .take((end * 16000.0) as usize)
                .skip((start * 16000.0) as usize)
            {
                *sample = (0.4 * (2.0 * PI * note * i as f64 / 16000.0).sin()) as f32;
            }
        }
        let t = detect(&x, 120.0, 0.25).unwrap();
        let ns = t["notes"].as_array().unwrap();
        assert!(ns.len() >= 3);
        assert_eq!(ns[0]["note"], json!(57));
        assert_eq!(ns[1]["note"], json!(64));
        assert_eq!(ns[2]["note"], json!(69));
    }
}
