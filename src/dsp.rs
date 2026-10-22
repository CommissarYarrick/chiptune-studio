use crate::score::Event;
use anyhow::{Result, anyhow};
use rustfft::{FftPlanner, num_complex::Complex};
use serde_json::Value;
use std::f64::consts::PI;

fn num(v: &Value, key: &str, fallback: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(fallback)
}
fn arr_nums(v: Option<&Value>, fallback: &[f64]) -> Vec<f64> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<f64>>())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| fallback.to_vec())
}
pub(crate) fn poly_blep(p: f64, step: f64) -> f64 {
    if p < step {
        let x = p / step;
        2.0 * x - x * x - 1.0
    } else if p > 1.0 - step {
        let x = (p - 1.0) / step;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}
fn table_wave(kind: &str, harmonics: usize, points: &[f64]) -> Vec<f64> {
    const N: usize = 2048;
    let mut planner = FftPlanner::<f64>::new();
    let mut spectrum = vec![Complex::new(0.0, 0.0); N];
    if kind == "triangle" {
        for k in (1..=harmonics.min(N / 2 - 1)).step_by(2) {
            let sign = if ((k - 1) / 2) % 2 == 0 { 1.0 } else { -1.0 };
            let c = Complex::new(0.0, -sign / (k * k) as f64);
            spectrum[k] = c;
            spectrum[N - k] = c.conj();
        }
    } else {
        let len = points.len().max(1);
        let mut cycle = Vec::with_capacity(N);
        for i in 0..N {
            let x = i as f64 * len as f64 / N as f64;
            let j = x.floor() as usize % len;
            let frac = x - x.floor();
            let a = points.get(j).copied().unwrap_or(0.0);
            let b = points.get((j + 1) % len).copied().unwrap_or(0.0);
            cycle.push(Complex::new(a * (1.0 - frac) + b * frac, 0.0));
        }
        planner.plan_fft_forward(N).process(&mut cycle);
        for k in 0..=harmonics.min(N / 2) {
            spectrum[k] = cycle[k];
            if k > 0 && k < N / 2 {
                spectrum[N - k] = cycle[N - k];
            }
        }
        spectrum[0] = Complex::new(0.0, 0.0);
    }
    let mut out = spectrum;
    planner.plan_fft_inverse(N).process(&mut out);
    let peak = out
        .iter()
        .map(|x| x.re.abs())
        .fold(0.0, f64::max)
        .max(1e-12);
    let mut vals: Vec<f64> = out.iter().map(|x| x.re / peak).collect();
    vals.push(vals[0]);
    vals
}
fn splitmix_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}
fn seed_for(uid: &str, seed: u64) -> u64 {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(uid.as_bytes());
    seed ^ u64::from_le_bytes(h[..8].try_into().unwrap())
}
fn envelope(gate: usize, release: usize, rate: u32, adsr: &[f64]) -> Vec<f64> {
    let attack = adsr.first().copied().unwrap_or(0.003);
    let decay = adsr.get(1).copied().unwrap_or(0.1);
    let sustain = adsr.get(2).copied().unwrap_or(0.6);
    let mut e = Vec::with_capacity(gate + release);
    for i in 0..gate {
        let t = i as f64 / rate as f64;
        e.push(if t < attack {
            t / attack.max(1.0 / rate as f64)
        } else {
            sustain
                + (1.0 - sustain)
                    * (-5.0 * (t - attack).max(0.0) / decay.max(1.0 / rate as f64)).exp()
        });
    }
    if let Some(last) = e.last().copied() {
        for i in 0..release {
            let p = if release <= 1 {
                1.0
            } else {
                i as f64 / (release - 1) as f64
            };
            e.push(last * (-5.0 * p).exp() * (1.0 - p));
        }
    }
    if let Some(first) = e.first_mut() {
        *first = 0.0;
    }
    e
}
fn voice(
    event: &Event,
    instrument: &Value,
    rate: u32,
    bpm: f64,
    seed: u64,
    max_samples: Option<usize>,
) -> Vec<f32> {
    let adsr = arr_nums(instrument.get("adsr"), &[0.003, 0.1, 0.6, 0.04]);
    let gate = ((event.duration * 60.0 / bpm * rate as f64).round_ties_even() as usize).max(1);
    let mut release =
        (adsr.get(3).copied().unwrap_or(0.04) * rate as f64).round_ties_even() as usize;
    if let Some(max) = max_samples {
        release = release.min(max.saturating_sub(gate));
    }
    let n = gate + release;
    let offsets = arr_nums(instrument.get("arpeggio"), &[0.0]);
    let arp_hz = num(instrument, "arp_hz", 24.0);
    let slide = num(instrument, "pitch_slide", 0.0);
    let slide_ms = (num(instrument, "slide_ms", 40.0) / 1000.0).max(1e-9);
    let vib = arr_nums(instrument.get("vibrato"), &[0.0, 0.0, 0.0]);
    let mut phases = Vec::with_capacity(n);
    let mut steps = Vec::with_capacity(n);
    let mut phase = 0.0;
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let sem = offsets[((t * arp_hz).floor() as usize) % offsets.len()]
            + slide * (-5.0 * t / slide_ms).exp()
            + vib.get(1).copied().unwrap_or(0.0) / 100.0
                * (2.0 * PI * vib.first().copied().unwrap_or(0.0) * t).sin()
                * ((t - vib.get(2).copied().unwrap_or(0.0)) / 0.05).clamp(0.0, 1.0);
        let hz = (440.0 * 2.0f64.powf((event.note as f64 - 69.0 + sem) / 12.0))
            .clamp(10.0, rate as f64 * 0.45);
        let step = hz / rate as f64;
        phases.push(phase);
        steps.push(step);
        phase = (phase + step) % 1.0;
    }
    let kind = instrument
        .get("wave")
        .and_then(Value::as_str)
        .unwrap_or("sine");
    let mut sig = vec![0.0; n];
    match kind {
        "pulse" => {
            let duty = num(instrument, "duty", 0.5).clamp(0.01, 0.99);
            for i in 0..n {
                let p = phases[i];
                sig[i] = (if p < duty { 1.0 } else { -1.0 }) + poly_blep(p, steps[i])
                    - poly_blep((p - duty).rem_euclid(1.0), steps[i])
                    - (2.0 * duty - 1.0);
            }
        }
        "saw" => {
            for i in 0..n {
                sig[i] = 2.0 * phases[i] - 1.0 - poly_blep(phases[i], steps[i]);
            }
        }
        "triangle" | "wavetable" => {
            let harmonics = ((rate as f64 * 0.46
                / steps.iter().map(|s| s * rate as f64).fold(0.0, f64::max))
            .floor() as usize)
                .clamp(1, 512);
            let pts = arr_nums(instrument.get("table"), &[]);
            let table = table_wave(kind, harmonics, &pts);
            for i in 0..n {
                let pos = phases[i] * (table.len() - 1) as f64;
                let j = pos.floor() as usize;
                let f = pos - j as f64;
                sig[i] = table[j] * (1.0 - f) + table[j + 1] * f;
            }
        }
        "noise" => {
            let mut state = seed_for(&event.uid, seed);
            let mut prev = 0.0;
            for (i, sample) in sig.iter_mut().enumerate() {
                let x =
                    ((splitmix_next(&mut state) >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0;
                let t = i as f64 / rate as f64;
                *sample = if event.instrument.contains("hat") {
                    let y = (x - 0.8 * prev) / 1.8;
                    prev = x;
                    y
                } else {
                    0.8 * x + 0.2 * (2.0 * PI * 180.0 * t).sin()
                };
            }
        }
        "kick" => {
            for i in 0..n {
                sig[i] = (1.7 * (2.0 * PI * phases[i]).sin()).tanh() / 1.7f64.tanh();
            }
        }
        _ => {
            for i in 0..n {
                sig[i] = (2.0 * PI * phases[i]).sin();
            }
        }
    }
    let bits = num(instrument, "bits", 16.0) as i32;
    let env = envelope(gate, release, rate, &adsr);
    sig.iter()
        .zip(env)
        .map(|(s, e)| {
            let mut x = *s;
            if bits < 16 {
                let levels = 2f64.powi(bits) - 1.0;
                x = ((x.clamp(-1.0, 1.0) + 1.0) * 0.5 * levels).round_ties_even() / levels * 2.0
                    - 1.0;
            }
            (x * e * event.velocity) as f32
        })
        .collect()
}
fn add_circular(dest: &mut [[f32; 2]], signal: &[f32], start: usize, pan: f64) {
    let left = ((pan + 1.0) * PI / 4.0).cos() as f32;
    let right = ((pan + 1.0) * PI / 4.0).sin() as f32;
    for (i, &x) in signal.iter().enumerate() {
        let j = (start + i) % dest.len();
        dest[j][0] += x * left;
        dest[j][1] += x * right;
    }
}

pub fn voice_event(
    event: &Event,
    instrument: &Value,
    sample_rate: u32,
    bpm: f64,
    seed: u64,
    max_samples: Option<usize>,
) -> Vec<f32> {
    voice(event, instrument, sample_rate, bpm, seed, max_samples)
}
pub fn track_audio(
    data: &Value,
    track: &Value,
    events: &[Event],
    sample_rate: u32,
) -> Result<Vec<[f32; 2]>> {
    if sample_rate == 0 {
        return Err(anyhow!("sample rate must be positive"));
    }
    let bpm = data
        .get("bpm")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("score bpm missing"))?;
    let length = data
        .get("length")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("score length missing"))?;
    let count = (length * 60.0 / bpm * sample_rate as f64).round_ties_even() as usize;
    if count == 0 {
        return Err(anyhow!("score has no audio duration"));
    }
    let id = track.get("id").and_then(Value::as_str).unwrap_or("");
    let mut notes: Vec<&Event> = events.iter().filter(|e| e.track == id).collect();
    notes.sort_by(|a, b| a.at.total_cmp(&b.at));
    let instruments = data
        .get("instruments")
        .ok_or_else(|| anyhow!("score instruments missing"))?;
    let mut output = vec![[0.0f32; 2]; count];
    let chip4 = data.get("profile").and_then(Value::as_str) == Some("chip4");
    for i in 0..notes.len() {
        let e = notes[i];
        let start = (e.at * 60.0 / bpm * sample_rate as f64)
            .round_ties_even()
            .max(0.0) as usize;
        let max = if chip4 {
            let next = if i + 1 < notes.len() {
                notes[i + 1].at
            } else {
                notes[0].at + length
            };
            Some(
                ((next - e.at) * 60.0 / bpm * sample_rate as f64)
                    .round_ties_even()
                    .max(1.0) as usize,
            )
        } else {
            None
        };
        let inst = instruments
            .get(&e.instrument)
            .ok_or_else(|| anyhow!("missing instrument {}", e.instrument))?;
        let samples = voice(
            e,
            inst,
            sample_rate,
            bpm,
            data.get("seed").and_then(Value::as_u64).unwrap_or(0),
            max,
        );
        add_circular(&mut output, &samples, start, num(track, "pan", 0.0));
    }
    let gain = 10f32.powf((num(track, "gain_db", -6.0) / 20.0) as f32);
    for frame in &mut output {
        frame[0] *= gain;
        frame[1] *= gain;
    }
    let mut means = [0.0f64; 2];
    for f in &output {
        means[0] += f[0] as f64;
        means[1] += f[1] as f64;
    }
    means[0] /= count as f64;
    means[1] /= count as f64;
    for f in &mut output {
        f[0] -= means[0] as f32;
        f[1] -= means[1] as f32;
    }
    let delay = data.get("delay").unwrap_or(&Value::Null);
    let wet = num(delay, "mix", 0.0);
    if wet != 0.0 {
        let dry = output.clone();
        let step = (num(delay, "beats", 0.75) * 60.0 / bpm * sample_rate as f64).round_ties_even()
            as usize;
        let feedback = num(delay, "feedback", 0.3);
        for echo in 1..=6 {
            let offset = step.saturating_mul(echo);
            for (i, frame) in output.iter_mut().enumerate() {
                let j = (i + count - offset % count) % count;
                let echo_gain = (wet * feedback.powi(echo as i32 - 1)) as f32;
                for (sample, dry_sample) in frame.iter_mut().zip(dry[j]) {
                    *sample += dry_sample * echo_gain;
                }
            }
        }
    }
    Ok(output)
}
