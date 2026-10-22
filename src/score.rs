use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub uid: String,
    pub track: String,
    pub instrument: String,
    pub at: f64,
    pub duration: f64,
    pub note: i32,
    pub velocity: f64,
}

const PROFILES: &[&str] = &["chip4", "console8"];
const WAVES: &[&str] = &[
    "pulse",
    "triangle",
    "saw",
    "sine",
    "noise",
    "kick",
    "wavetable",
];

pub fn default_bank() -> Value {
    json!({
        "lead":{"wave":"pulse","duty":0.25,"adsr":[0.003,0.10,0.62,0.045],"vibrato":[5.5,12,0.16]},
        "arp":{"wave":"pulse","duty":0.125,"adsr":[0.002,0.08,0.28,0.025]},
        "bass":{"wave":"triangle","bits":4,"adsr":[0.002,0.06,0.8,0.025]},
        "hat":{"wave":"noise","adsr":[0.001,0.026,0,0.008]},
        "snare":{"wave":"noise","adsr":[0.001,0.075,0,0.02]},
        "kick":{"wave":"kick","pitch_slide":18,"slide_ms":35,"adsr":[0.001,0.09,0,0.015]},
        "bell":{"wave":"sine","adsr":[0.002,0.30,0.06,0.18]},
        "warm":{"wave":"wavetable","table":[0,0.36,0.66,0.86,0.97,1,0.97,0.93,0.88,0.83,0.76,0.67,0.56,0.43,0.29,0.14,0,-0.14,-0.29,-0.43,-0.56,-0.67,-0.76,-0.83,-0.88,-0.93,-0.97,-1,-0.97,-0.86,-0.66,-0.36],"adsr":[0.025,0.16,0.62,0.20],"vibrato":[4.5,5,0.2]}
    })
}

pub fn load(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("reading score {}", path.display()))?;
    let text = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let score: Value = serde_json::from_slice(text).context("Malformed score JSON")?;
    validate(&score)?;
    Ok(score)
}

pub fn digest(score: &Value) -> String {
    // serde_json's default object map is key-sorted, and compact serialization is deterministic.
    let bytes = serde_json::to_vec(score).expect("serializing a JSON value cannot fail");
    hex::encode(Sha256::digest(bytes))
}

pub fn beat_per_bar(score: &Value) -> f64 {
    let meter = score.get("meter").and_then(Value::as_array);
    let (n, d) = meter
        .map(|m| {
            (
                m.first().and_then(Value::as_f64).unwrap_or(4.0),
                m.get(1).and_then(Value::as_f64).unwrap_or(4.0),
            )
        })
        .unwrap_or((4.0, 4.0));
    n * 4.0 / d
}

pub fn pitch(value: &Value) -> Result<i32> {
    if let Some(n) = value.as_i64()
        && !value.is_boolean()
        && (0..=127).contains(&n)
    {
        return Ok(n as i32);
    }
    if let Some(s) = value.as_str() {
        let bytes = s.as_bytes();
        if bytes.len() >= 2 {
            let letter = bytes[0].to_ascii_uppercase();
            let base = match letter {
                b'C' => 0,
                b'D' => 2,
                b'E' => 4,
                b'F' => 5,
                b'G' => 7,
                b'A' => 9,
                b'B' => 11,
                _ => -1,
            };
            if base >= 0 {
                let mut i = 1;
                let mut acc = 0;
                if bytes.get(i) == Some(&b'#') {
                    acc = 1;
                    i += 1;
                } else if bytes.get(i) == Some(&b'b') {
                    acc = -1;
                    i += 1;
                }
                if i < bytes.len() {
                    let octave_text = std::str::from_utf8(&bytes[i..]).unwrap_or("");
                    if let Ok(octave) = octave_text.parse::<i64>()
                        && !octave_text.is_empty()
                        && octave_text
                            .strip_prefix('-')
                            .unwrap_or(octave_text)
                            .bytes()
                            .all(|b| b.is_ascii_digit())
                    {
                        let midi = octave
                            .checked_add(1)
                            .and_then(|n| n.checked_mul(12))
                            .and_then(|n| n.checked_add((base + acc) as i64));
                        if let Some(midi) = midi.filter(|n| (0..=127).contains(n)) {
                            return Ok(midi as i32);
                        }
                    }
                }
            }
        }
    }
    bail!("Invalid note {value:?}; use MIDI 0..127 or C4/F#3/Bb2")
}

fn number(v: &Value, lo: f64, hi: f64, where_: &str) -> Result<f64> {
    let n = v
        .as_f64()
        .filter(|_| !v.is_boolean())
        .ok_or_else(|| anyhow!("{where_}: expected finite number in [{lo}, {hi}]"))?;
    if !n.is_finite() || n < lo || n > hi {
        bail!("{where_}: expected finite number in [{lo}, {hi}]");
    }
    Ok(n)
}
fn integer(v: &Value, lo: i64, hi: i64, where_: &str) -> Result<i64> {
    let n = number(v, lo as f64, hi as f64, where_)?;
    if n.fract() != 0.0 {
        bail!("{where_}: expected integer");
    }
    Ok(n as i64)
}
fn object<'a>(v: &'a Value, where_: &str) -> Result<&'a Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| anyhow!("{where_}: expected object"))
}
fn fields(v: &Value, allowed: &str, where_: &str) -> Result<()> {
    let o = object(v, where_)?;
    let allowed: HashSet<&str> = allowed.split_whitespace().collect();
    if let Some(k) = o.keys().filter(|k| !allowed.contains(k.as_str())).min() {
        bail!("{where_}: unknown fields [{k:?}]");
    }
    Ok(())
}
fn arr<'a>(v: &'a Value, where_: &str) -> Result<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| anyhow!("{where_}: expected list"))
}
fn get<'a>(v: &'a Value, key: &str, where_: &str) -> Result<&'a Value> {
    v.get(key)
        .ok_or_else(|| anyhow!("{where_}: missing field {key}"))
}
fn strval<'a>(v: &'a Value, where_: &str) -> Result<&'a str> {
    v.as_str()
        .ok_or_else(|| anyhow!("{where_}: expected string"))
}
fn pitch_i(v: &Value) -> Result<i32> {
    pitch(v)
}

fn compile_unchecked(score: &Value) -> Result<Vec<Event>> {
    let patterns = score["patterns"]
        .as_object()
        .context("patterns must be an object")?;
    let mut events = Vec::new();
    for track in score["tracks"]
        .as_array()
        .context("tracks must be a list")?
    {
        if track.get("mute").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let tid = strval(get(track, "id", "track")?, "track.id")?;
        let ti = strval(get(track, "instrument", "track")?, "track.instrument")?;
        for (ci, clip) in track["clips"]
            .as_array()
            .context("track.clips")?
            .iter()
            .enumerate()
        {
            let pname = strval(get(clip, "pattern", "clip")?, "clip.pattern")?;
            let pat = &patterns[pname];
            let plen = pat["length"].as_f64().context("pattern.length")?;
            let at = clip["at"].as_f64().context("clip.at")?;
            let repeat = clip.get("repeat").and_then(Value::as_u64).unwrap_or(1);
            let transpose = clip.get("transpose").and_then(Value::as_i64).unwrap_or(0) as i32;
            let cv = clip.get("velocity").and_then(Value::as_f64).unwrap_or(1.0);
            let clipid = clip
                .get("id")
                .map(id_string)
                .unwrap_or_else(|| ci.to_string());
            for r in 0..repeat {
                for (ni, note) in pat["notes"]
                    .as_array()
                    .context("pattern.notes")?
                    .iter()
                    .enumerate()
                {
                    let notes = match &note["note"] {
                        Value::Array(a) => a.as_slice(),
                        v => std::slice::from_ref(v),
                    };
                    let nid = note
                        .get("id")
                        .map(id_string)
                        .unwrap_or_else(|| ni.to_string());
                    let inst = note.get("instrument").and_then(Value::as_str).unwrap_or(ti);
                    for (pi, p) in notes.iter().enumerate() {
                        events.push(Event {
                            uid: format!("{tid}/{clipid}/{r}/{nid}/{pi}"),
                            track: tid.to_owned(),
                            instrument: inst.to_owned(),
                            at: at + r as f64 * plen + note["at"].as_f64().context("note.at")?,
                            duration: note["duration"].as_f64().context("note.duration")?,
                            note: pitch_i(p)? + transpose,
                            velocity: note.get("velocity").and_then(Value::as_f64).unwrap_or(0.8)
                                * cv,
                        });
                    }
                }
            }
        }
    }
    events.sort_by(|a, b| {
        a.at.total_cmp(&b.at)
            .then(a.track.cmp(&b.track))
            .then(a.uid.cmp(&b.uid))
    });
    Ok(events)
}
fn id_string(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        s.to_owned()
    } else {
        v.to_string()
    }
}

pub fn compile(score: &Value) -> Result<Vec<Event>> {
    validate(score)?;
    compile_unchecked(score)
}

pub fn validate(score: &Value) -> Result<Value> {
    fields(
        score,
        "version title profile bpm meter length seed instruments patterns tracks sections master_gain_db delay",
        "score",
    )?;
    if score.get("version") != Some(&json!(1)) {
        bail!("version must be 1");
    }
    if !score
        .get("title")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
    {
        bail!("title must be nonempty");
    }
    let profile = strval(get(score, "profile", "score")?, "profile")?;
    if !PROFILES.contains(&profile) {
        bail!("profile must be one of [\"chip4\", \"console8\"]");
    }
    number(get(score, "bpm", "score")?, 30.0, 300.0, "bpm")?;
    let length = number(get(score, "length", "score")?, 0.25, 1024.0, "length")?;
    integer(
        score.get("seed").unwrap_or(&json!(0)),
        0,
        4294967295,
        "seed",
    )?;
    number(
        score.get("master_gain_db").unwrap_or(&json!(-3)),
        -60.0,
        12.0,
        "master_gain_db",
    )?;
    let default_meter = json!([4, 4]);
    let meter = arr(score.get("meter").unwrap_or(&default_meter), "meter")?;
    if meter.len() != 2 {
        bail!("meter must be [numerator, denominator]");
    }
    integer(&meter[0], 1, 16, "meter numerator")?;
    if ![2, 4, 8, 16].iter().any(|d| meter[1].as_i64() == Some(*d)) {
        bail!("meter denominator must be 2,4,8,16");
    }
    let bank = object(get(score, "instruments", "score")?, "instruments")?;
    if bank.is_empty() || bank.len() > 64 {
        bail!("instruments must contain 1..64 named instruments");
    }
    for (name, ins) in bank {
        fields(
            ins,
            "wave duty adsr vibrato pitch_slide slide_ms bits table arpeggio arp_hz",
            &format!("instrument {name}"),
        )?;
        let wave = strval(get(ins, "wave", name)?, &format!("instrument {name}.wave"))?;
        if !WAVES.contains(&wave) {
            bail!("instrument {name}: unknown wave");
        }
        let default_adsr = json!([0.003, 0.1, 0.6, 0.04]);
        let adsr = arr(
            ins.get("adsr").unwrap_or(&default_adsr),
            &format!("instrument {name}.adsr"),
        )?;
        if adsr.len() != 4 {
            bail!("instrument {name}: adsr needs four values");
        }
        for (i, v) in adsr.iter().enumerate() {
            number(
                v,
                0.0,
                if i == 2 { 1.0 } else { 2.0 },
                &format!("{name}.adsr[{i}]"),
            )?;
        }
        let default_vibrato = json!([0, 0, 0]);
        let vib = arr(
            ins.get("vibrato").unwrap_or(&default_vibrato),
            &format!("instrument {name}.vibrato"),
        )?;
        if vib.len() != 3 {
            bail!("{name}.vibrato needs [hz,cents,delay_s]");
        }
        for (v, hi) in vib.iter().zip([20.0, 100.0, 5.0]) {
            number(v, 0.0, hi, &format!("{name}.vibrato"))?;
        }
        number(
            ins.get("duty").unwrap_or(&json!(0.5)),
            0.0625,
            0.9375,
            &format!("{name}.duty"),
        )?;
        number(
            ins.get("pitch_slide").unwrap_or(&json!(0)),
            -36.0,
            36.0,
            &format!("{name}.pitch_slide"),
        )?;
        number(
            ins.get("slide_ms").unwrap_or(&json!(40)),
            1.0,
            1000.0,
            &format!("{name}.slide_ms"),
        )?;
        integer(
            ins.get("bits").unwrap_or(&json!(16)),
            4,
            16,
            &format!("{name}.bits"),
        )?;
        let default_arpeggio = json!([0]);
        let arp = arr(
            ins.get("arpeggio").unwrap_or(&default_arpeggio),
            &format!("{name}.arpeggio"),
        )?;
        if arp.is_empty() || arp.len() > 16 {
            bail!("{name}.arpeggio needs 1..16 semitone offsets");
        }
        for v in arp {
            integer(v, -24, 24, &format!("{name}.arpeggio"))?;
        }
        number(
            ins.get("arp_hz").unwrap_or(&json!(24)),
            1.0,
            60.0,
            &format!("{name}.arp_hz"),
        )?;
        if wave == "wavetable" {
            let table = arr(
                ins.get("table").unwrap_or(&Value::Null),
                &format!("{name}.table"),
            )?;
            if table.len() < 8 || table.len() > 256 {
                bail!("{name}.table needs 8..256 values");
            }
            for v in table {
                number(v, -1.0, 1.0, &format!("{name}.table"))?;
            }
        }
    }
    let patterns = object(get(score, "patterns", "score")?, "patterns")?;
    if patterns.is_empty() || patterns.len() > 256 {
        bail!("patterns needs 1..256 entries");
    }
    let mut note_count = 0usize;
    for (name, pat) in patterns {
        fields(pat, "length notes", &format!("pattern {name}"))?;
        let plen = number(
            get(pat, "length", name)?,
            0.0625,
            1024.0,
            &format!("{name}.length"),
        )?;
        let notes = arr(get(pat, "notes", name)?, &format!("{name}.notes"))?;
        let mut ids = HashSet::new();
        for (ni, n) in notes.iter().enumerate() {
            fields(
                n,
                "id at note duration velocity instrument",
                &format!("{name}.notes[{ni}]"),
            )?;
            let ident = n.get("id").map(id_string).unwrap_or_else(|| ni.to_string());
            if !ids.insert(ident.clone()) {
                bail!("{name}: duplicate note id {ident}");
            }
            let at = number(get(n, "at", "note")?, 0.0, plen, "note.at")?;
            let dur = number(get(n, "duration", "note")?, 0.001, plen, "note.duration")?;
            number(
                n.get("velocity").unwrap_or(&json!(0.8)),
                0.001,
                1.0,
                "note.velocity",
            )?;
            if at + dur > plen + 1e-8 {
                bail!("{name}: note extends beyond pattern");
            }
            let chord = match n.get("note").context("note: missing field note")? {
                Value::Array(a) => a.as_slice(),
                v => std::slice::from_ref(v),
            };
            if chord.is_empty() || chord.len() > 8 {
                bail!("note chord needs 1..8 pitches");
            }
            for p in chord {
                pitch(p)?;
            }
            let inst = n
                .get("instrument")
                .and_then(Value::as_str)
                .unwrap_or_else(|| bank.keys().next().map(String::as_str).unwrap_or(""));
            if !bank.contains_key(inst) {
                bail!("unknown note instrument");
            }
            note_count += chord.len();
        }
    }
    if note_count > 8192 {
        bail!("Too many pattern notes (max8192)");
    }
    let tracks = arr(get(score, "tracks", "score")?, "tracks")?;
    if tracks.is_empty() || tracks.len() > 8 {
        bail!("tracks must contain 1..8 entries");
    }
    let mut track_ids = HashSet::new();
    let mut slots = HashSet::new();
    let mut expansion = 0usize;
    for t in tracks {
        fields(t, "id slot instrument gain_db pan clips mute", "track")?;
        let tid = strval(get(t, "id", "track")?, "track.id")?;
        if tid.is_empty()
            || tid.len() > 40
            || !tid
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            bail!("track.id needs a short filename-safe identifier");
        }
        if !track_ids.insert(tid) {
            bail!("duplicate track.id");
        }
        let inst = strval(get(t, "instrument", "track")?, "track.instrument")?;
        if !bank.contains_key(inst) {
            bail!("unknown track instrument");
        }
        number(
            t.get("gain_db").unwrap_or(&json!(-6)),
            -60.0,
            12.0,
            "track.gain_db",
        )?;
        number(t.get("pan").unwrap_or(&json!(0)), -1.0, 1.0, "track.pan")?;
        if !t.get("mute").unwrap_or(&json!(false)).is_boolean() {
            bail!("track.mute must be boolean");
        }
        let clips = arr(get(t, "clips", "track")?, "track.clips")?;
        if clips.len() > 1024 {
            bail!("track.clips must be list, max1024");
        }
        if profile == "chip4" {
            let slot = strval(get(t, "slot", "track")?, "track.slot")?;
            if !["pulse1", "pulse2", "triangle", "noise"].contains(&slot) || !slots.insert(slot) {
                bail!("chip4 needs unique pulse1/pulse2/triangle/noise slots");
            }
        }
        let mut clip_ids = HashSet::new();
        for (ci, c) in clips.iter().enumerate() {
            fields(c, "id pattern at repeat transpose velocity", "clip")?;
            let cid = c.get("id").map(id_string).unwrap_or_else(|| ci.to_string());
            if !clip_ids.insert(cid) {
                bail!("duplicate clip.id");
            }
            let pname = strval(get(c, "pattern", "clip")?, "clip.pattern")?;
            let pat = patterns
                .get(pname)
                .ok_or_else(|| anyhow!("unknown clip pattern"))?;
            let at = number(get(c, "at", "clip")?, 0.0, length, "clip.at")?;
            let repeat = integer(c.get("repeat").unwrap_or(&json!(1)), 1, 256, "clip.repeat")?;
            integer(
                c.get("transpose").unwrap_or(&json!(0)),
                -48,
                48,
                "clip.transpose",
            )?;
            number(
                c.get("velocity").unwrap_or(&json!(1)),
                0.001,
                1.0,
                "clip.velocity",
            )?;
            let plen = pat["length"].as_f64().context("pattern.length")?;
            if at + repeat as f64 * plen > length + 1e-8 {
                bail!("clip extends beyond score.length");
            }
            let nc = pat["notes"]
                .as_array()
                .context("pattern.notes")?
                .iter()
                .map(|n| {
                    if let Some(a) = n.get("note").and_then(Value::as_array) {
                        a.len()
                    } else {
                        1
                    }
                })
                .sum::<usize>();
            expansion += repeat as usize * nc;
        }
    }
    if expansion > 32768 {
        bail!("Expanded score exceeds32768 notes");
    }
    let mut section_ids = HashSet::new();
    for s in score
        .get("sections")
        .unwrap_or(&json!([]))
        .as_array()
        .context("sections must be list")?
    {
        fields(s, "id at length", "section")?;
        let id = strval(get(s, "id", "section")?, "section.id")?;
        if !section_ids.insert(id) {
            bail!("sections need unique ids");
        }
        let at = number(get(s, "at", "section")?, 0.0, length, "section.at")?;
        let sl = number(
            get(s, "length", "section")?,
            0.001,
            length,
            "section.length",
        )?;
        if at + sl > length + 1e-8 {
            bail!("section beyond score");
        }
    }
    let default_delay = json!({});
    let delay = score.get("delay").unwrap_or(&default_delay);
    fields(delay, "beats mix feedback", "delay")?;
    number(
        delay.get("beats").unwrap_or(&json!(0.75)),
        0.0625,
        8.0,
        "delay.beats",
    )?;
    number(delay.get("mix").unwrap_or(&json!(0)), 0.0, 0.5, "delay.mix")?;
    number(
        delay.get("feedback").unwrap_or(&json!(0.3)),
        0.0,
        0.7,
        "delay.feedback",
    )?;
    let events = compile_unchecked(score)?;
    let mut changes = Vec::new();
    let mut per_track: BTreeMap<String, Vec<(f64, i32)>> = BTreeMap::new();
    let by_id: BTreeMap<&str, &Value> = tracks
        .iter()
        .filter_map(|t| Some((t.get("id")?.as_str()?, t)))
        .collect();
    for e in &events {
        if !(0..=127).contains(&e.note) {
            bail!("transposition moves note outside MIDI0..127");
        }
        let wave = bank
            .get(&e.instrument)
            .and_then(|i| i.get("wave"))
            .and_then(Value::as_str)
            .context("instrument wave")?;
        if profile == "chip4" {
            let slot = by_id[e.track.as_str()]["slot"]
                .as_str()
                .context("track slot")?;
            let allowed = match slot {
                "pulse1" | "pulse2" => &["pulse"][..],
                "triangle" => &["triangle"][..],
                _ => &["noise", "kick"][..],
            };
            if !allowed.contains(&wave) {
                bail!("{}: {} not available in {}", e.track, wave, slot);
            }
        }
        changes.push((e.at, 1));
        changes.push((e.at + e.duration, -1));
        let v = per_track.entry(e.track.clone()).or_default();
        v.push((e.at, 1));
        v.push((e.at + e.duration, -1));
    }
    fn peak(changes: &mut [(f64, i32)]) -> i32 {
        // A computed end (start + duration) and an identical next onset can
        // differ by floating-point roundoff. Order endpoints on a nanobeat
        // grid, with note-offs before note-ons. Real overlaps still count.
        let instant = |beat: f64| (beat * 1_000_000_000.).round() as i64;
        changes.sort_by(|a, b| instant(a.0).cmp(&instant(b.0)).then(a.1.cmp(&b.1)));
        let (mut active, mut max) = (0, 0);
        for (_, d) in changes {
            active += *d;
            max = max.max(active);
        }
        max
    }
    let max = peak(&mut changes);
    let voices = if profile == "chip4" { 4 } else { 8 };
    if max > voices {
        bail!("voice budget exceeded: {max}");
    }
    if profile == "chip4" && per_track.values_mut().any(|v| peak(v) > 1) {
        bail!("chip4 channel contains overlapping gates/chords");
    }
    let beats = beat_per_bar(score);
    let bpm = score["bpm"].as_f64().context("bpm")?;
    Ok(
        json!({"valid":true,"score_sha256":digest(score),"events":events.len(),"max_gated_voices":max,"seconds":length*60.0/bpm,"bars":length/beats,"profile":profile,"warnings":if events.is_empty(){vec!["Score contains no sounding notes"]}else{vec![]}}),
    )
}

pub fn apply_patch(score: &Value, operations: &Value) -> Result<Value> {
    let ops = operations
        .as_array()
        .ok_or_else(|| anyhow!("patch requires at most256 operations"))?;
    if ops.len() > 256 {
        bail!("patch requires at most256 operations");
    }
    let mut result = score.clone();
    for op in ops {
        fields(op, "op path value", "patch operation")?;
        let kind = strval(get(op, "op", "patch operation")?, "patch op")?;
        if !["add", "remove", "replace", "test"].contains(&kind) {
            bail!("unsupported patch operation");
        }
        let path = strval(get(op, "path", "patch operation")?, "patch path")?;
        if !path.starts_with('/') || path == "/" {
            bail!("patch path must be a JSON pointer below root");
        }
        let parts: Vec<String> = path[1..]
            .split('/')
            .map(|p| p.replace("~1", "/").replace("~0", "~"))
            .collect();
        let (last, parents) = parts.split_last().unwrap();
        let mut node = &mut result;
        for part in parents {
            node = match node {
                Value::Array(a) => {
                    let ix = part
                        .parse::<usize>()
                        .map_err(|_| anyhow!("patch {path}: invalid array index"))?;
                    a.get_mut(ix)
                        .ok_or_else(|| anyhow!("patch {path}: array index out of bounds"))?
                }
                Value::Object(o) => o
                    .get_mut(part)
                    .ok_or_else(|| anyhow!("patch {path}: missing object field"))?,
                _ => bail!("patch parent is not container"),
            };
        }
        let value = op.get("value");
        if kind != "remove" && value.is_none() {
            bail!("patch operation missing value");
        }
        let value = value.unwrap_or(&Value::Null);
        if let Some(a) = node.as_array_mut() {
            let ix = if *last == "-" && kind == "add" {
                a.len()
            } else {
                last.parse::<usize>()
                    .map_err(|_| anyhow!("patch {path}: invalid array index"))?
            };
            if ix > a.len() || (kind != "add" && ix >= a.len()) {
                bail!("patch {path}: array index out of bounds");
            }
            match kind {
                "add" => a.insert(ix, value.clone()),
                "remove" => {
                    a.remove(ix);
                }
                "replace" => a[ix] = value.clone(),
                "test" => {
                    if a[ix] != *value {
                        bail!("patch test failed")
                    }
                }
                _ => unreachable!(),
            }
        } else if let Some(o) = node.as_object_mut() {
            match kind {
                "add" => {
                    o.insert(last.clone(), value.clone());
                }
                "remove" => {
                    o.remove(last)
                        .ok_or_else(|| anyhow!("patch {path}: missing object field"))?;
                }
                "replace" => {
                    if !o.contains_key(last) {
                        bail!("patch {path}: missing object field");
                    }
                    o.insert(last.clone(), value.clone());
                }
                "test" => {
                    if o.get(last) != Some(value) {
                        bail!("patch test failed")
                    }
                }
                _ => unreachable!(),
            }
        } else {
            bail!("patch parent is not container");
        }
    }
    validate(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn basic() -> Value {
        json!({"version":1,"title":"test","profile":"console8","bpm":120,"length":4,"instruments":{"lead":{"wave":"pulse"}},"patterns":{"p":{"length":1,"notes":[{"at":0,"duration":0.5,"note":"C4"}] }},"tracks":[{"id":"a","instrument":"lead","clips":[{"pattern":"p","at":0,"transpose":2}]}]})
    }
    #[test]
    fn rejects_unknown_fields() {
        let mut s = basic();
        s["extra"] = json!(true);
        assert!(validate(&s).is_err());
    }
    #[test]
    fn transpose_compiles() {
        let e = compile(&basic()).unwrap();
        assert_eq!(e[0].note, 62);
    }
    #[test]
    fn rejects_gatepoly() {
        let mut s = basic();
        s["profile"] = json!("chip4");
        s["tracks"][0]["slot"] = json!("pulse1");
        s["tracks"][0]["clips"]
            .as_array_mut()
            .unwrap()
            .push(json!({"pattern":"p","at":0.25}));
        assert!(
            validate(&s)
                .unwrap_err()
                .to_string()
                .contains("overlapping")
        );
    }

    #[test]
    fn touching_notes_do_not_exceed_chip_gate_budget() {
        let mut s = basic();
        s["profile"] = json!("chip4");
        s["tracks"][0]["slot"] = json!("pulse1");
        s["patterns"]["p"]["notes"] = json!([
            {"id":"a", "at":0.1, "duration":0.2, "note":60},
            {"id":"b", "at":0.3, "duration":0.2, "note":62}
        ]);
        assert!(validate(&s).is_ok());
        s["patterns"]["p"]["notes"][1]["at"] = json!(0.299);
        assert!(
            validate(&s).is_err(),
            "A real millibeat overlap must still fail"
        );
    }
    #[test]
    fn chip4_slots_are_unique() {
        let mut s = basic();
        s["profile"] = json!("chip4");
        s["tracks"][0]["slot"] = json!("pulse1");
        s["tracks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"b","instrument":"lead","slot":"pulse1","clips":[]}));
        assert!(validate(&s).is_err());
    }
    #[test]
    fn patch_failure_is_atomic() {
        let s = basic();
        let patch = json!([{"op":"replace","path":"/title","value":"changed"},{"op":"remove","path":"/missing"}]);
        assert!(apply_patch(&s, &patch).is_err());
        assert_eq!(s["title"], "test");
    }
}
