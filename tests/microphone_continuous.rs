use chiptune_studio::transcribe;

const INPUT_RATE: usize = 16_000;
const OUTPUT_RATE: usize = 32_000;

fn voiced_chirp() -> Vec<f32> {
    let mut samples = vec![0.0; (0.15 * INPUT_RATE as f64) as usize];
    let voiced_len = (1.1 * INPUT_RATE as f64) as usize;
    let mut phase = 0.0;
    for i in 0..voiced_len {
        let t = i as f64 / INPUT_RATE as f64;
        let frequency = 220.0 + (300.0 - 220.0) * t / 1.1;
        phase += std::f64::consts::TAU * frequency / INPUT_RATE as f64;
        let amplitude = if t < 0.55 { 0.2 } else { 0.07 };
        samples.push((amplitude * phase.sin()) as f32);
    }
    samples.resize(samples.len() + (0.25 * INPUT_RATE as f64) as usize, 0.0);
    samples
}

fn tone_with_gaps() -> Vec<f32> {
    let mut samples = Vec::new();
    for (frequency, duration) in [(220.0, 0.35), (0.0, 0.2), (330.0, 0.35)] {
        let count = (duration * INPUT_RATE as f64) as usize;
        for i in 0..count {
            let t = i as f64 / INPUT_RATE as f64;
            let value = if frequency == 0.0 {
                0.0
            } else {
                let edge = (t / 0.01).min((duration - t) / 0.01).clamp(0.0, 1.0);
                (0.2 * edge * (std::f64::consts::TAU * frequency * t).sin()) as f32
            };
            samples.push(value);
        }
    }
    samples
}

fn rms_mono_window(stereo: &[[f32; 2]], start: f64, end: f64) -> f64 {
    let first = (start * OUTPUT_RATE as f64) as usize;
    let last = (end * OUTPUT_RATE as f64) as usize;
    (stereo[first..last]
        .iter()
        .map(|frame| (frame[0] as f64).powi(2))
        .sum::<f64>()
        / (last - first) as f64)
        .sqrt()
}

fn positive_crossing_rate(stereo: &[[f32; 2]], time: f64, half_window: f64) -> f64 {
    let first = ((time - half_window) * OUTPUT_RATE as f64) as usize;
    let last = ((time + half_window) * OUTPUT_RATE as f64) as usize;
    let crossings = stereo[first..last]
        .windows(2)
        .filter(|pair| pair[0][0] <= 0.0 && pair[1][0] > 0.0)
        .count();
    crossings as f64 / (2.0 * half_window)
}

#[test]
fn continuous_resynthesis_keeps_length_chirp_dynamics_and_silence() {
    let input = voiced_chirp();
    let analysis = transcribe::detect(&input, 120.0, 0.0).unwrap();
    let stereo = transcribe::voice_to_chip(&input, &analysis).unwrap();

    assert_eq!(stereo.len(), input.len() * 2);
    assert!(
        stereo
            .iter()
            .flatten()
            .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
    );
    assert!(rms_mono_window(&stereo, 1.35, 1.5) < 0.001);

    let loud = rms_mono_window(&stereo, 0.35, 0.55);
    let soft = rms_mono_window(&stereo, 0.9, 1.1);
    assert!(loud > 0.02);
    assert!(soft <= loud * 0.75);

    for (time, ideal_hz) in [
        (0.4, 220.0 + 80.0 * 0.25 / 1.1),
        (0.8, 220.0 + 80.0 * 0.65 / 1.1),
    ] {
        let measured = positive_crossing_rate(&stereo, time, 0.1);
        assert!(
            (measured - ideal_hz).abs() <= 8.0,
            "at {time}s: {measured}Hz vs {ideal_hz}Hz"
        );
    }
}

#[test]
fn continuous_resynthesis_gates_the_rest_between_two_tones() {
    let input = tone_with_gaps();
    let analysis = transcribe::detect(&input, 120.0, 0.0).unwrap();
    let stereo = transcribe::voice_to_chip(&input, &analysis).unwrap();

    assert_eq!(stereo.len(), input.len() * 2);
    assert!(rms_mono_window(&stereo, 0.42, 0.49) < 0.003);
    assert!(rms_mono_window(&stereo, 0.08, 0.3) > 0.02);
    assert!(rms_mono_window(&stereo, 0.62, 0.82) > 0.02);
}
