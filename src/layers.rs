//! Non-destructive composition of independently recorded melodic phrases.

use crate::score;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// Add an imported monophonic take at a zero-based bar position.
///
/// Note timing is converted to the base tempo without changing elapsed seconds.
/// Existing tracks, patterns, instruments, and sections remain untouched.
pub fn add_melody_layer(base: &Value, take: &Value, at_bar: f64) -> Result<Value> {
    score::validate(base).context("Invalid base score")?;
    score::validate(take).context("Invalid melody take")?;
    if !at_bar.is_finite() || at_bar < 0.0 {
        bail!("Layer position must be a finite, nonnegative bar number");
    }
    for input in [base, take] {
        if input.get("meter").is_some_and(|m| m != &json!([4, 4])) {
            bail!("Melody layers currently require 4/4 meter");
        }
    }
    let tracks = base["tracks"].as_array().context("Base tracks missing")?;
    if tracks.len() >= 8 {
        bail!("All eight tracks are occupied; remove a track before adding another melody");
    }
    let slot = if base["profile"] == "chip4" {
        Some(
            ["pulse1", "pulse2", "triangle"]
                .into_iter()
                .find(|slot| !tracks.iter().any(|t| t["slot"].as_str() == Some(slot)))
                .context("All three melodic CHIP 4 channels are occupied; switch to console8 before adding another melody")?,
        )
    } else {
        None
    };
    let source_track = take["tracks"]
        .as_array()
        .and_then(|ts| ts.first())
        .context("Melody take has no lead track")?;
    let source_instrument = source_track["instrument"]
        .as_str()
        .context("Melody lead instrument missing")?;
    let mut instrument = take["instruments"][source_instrument].clone();
    let mut pattern = take["patterns"]["melody"].clone();
    let factor = base["bpm"].as_f64().context("Base tempo missing")?
        / take["bpm"].as_f64().context("Take tempo missing")?;
    let phrase_length = pattern["length"]
        .as_f64()
        .context("Take needs a melody pattern")?
        * factor;
    let start = at_bar * 4.0;
    let length = base["length"]
        .as_f64()
        .context("Base length missing")?
        .max(start + phrase_length);
    if !length.is_finite() || length * 60.0 / base["bpm"].as_f64().unwrap() > 240.0 {
        bail!("Combined arrangement exceeds the 240-second render limit");
    }
    let index = (1..)
        .find(|n| {
            let id = format!("layer_{n}");
            !tracks.iter().any(|t| t["id"] == id)
                && base["patterns"].get(&id).is_none()
                && base["instruments"].get(format!("layer_lead_{n}")).is_none()
                && !base["sections"]
                    .as_array()
                    .is_some_and(|ss| ss.iter().any(|s| s["id"] == id))
        })
        .unwrap();
    let id = format!("layer_{index}");
    let instrument_id = format!("layer_lead_{index}");
    pattern["length"] = json!(phrase_length);
    let notes = pattern["notes"]
        .as_array_mut()
        .context("Melody notes missing")?;
    if notes.is_empty() {
        bail!("Melody take contains no notes to add");
    }
    for note in notes {
        if note
            .get("instrument")
            .is_some_and(|i| i.as_str() != Some(source_instrument))
        {
            bail!("A melody layer must use one lead instrument");
        }
        note["at"] = json!(note["at"].as_f64().context("Note start missing")? * factor);
        note["duration"] =
            json!(note["duration"].as_f64().context("Note duration missing")? * factor);
        note["instrument"] = json!(instrument_id);
    }
    let mut track = source_track.clone();
    track["id"] = json!(id);
    track["instrument"] = json!(instrument_id);
    track["clips"] = json!([{"id":"take","pattern":id,"at":start}]);
    track
        .as_object_mut()
        .context("Lead track must be an object")?
        .remove("mute");
    if let Some(slot) = slot {
        track["slot"] = json!(slot);
        instrument["wave"] = json!(if slot == "triangle" {
            "triangle"
        } else {
            "pulse"
        });
        let settings = instrument
            .as_object_mut()
            .context("Lead instrument missing")?;
        settings.remove("table");
        if slot == "triangle" {
            settings.remove("duty");
        }
    } else {
        track.as_object_mut().unwrap().remove("slot");
    }
    let mut result = base.clone();
    result["length"] = json!(length);
    result["patterns"][&id] = pattern;
    result["instruments"][&instrument_id] = instrument;
    result["tracks"].as_array_mut().unwrap().push(track);
    if result.get("sections").is_none() {
        result["sections"] = json!([]);
    }
    result["sections"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":id,"at":start,"length":phrase_length}));
    score::validate(&result)
        .context("Cannot add melody within the current score's voice and timing limits")?;
    Ok(result)
}
