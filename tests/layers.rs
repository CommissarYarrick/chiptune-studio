use chiptune_studio::{layers::add_melody_layer, score};
use serde_json::{Value, json};

fn phrase(profile: &str, bpm: f64) -> Value {
    json!({"version":1,"title":"Phrase","profile":profile,"bpm":bpm,"meter":[4,4],"length":4,
        "instruments":{"lead":{"wave":"pulse","duty":0.25}},
        "patterns":{"melody":{"length":4,"notes":[{"at":0.5,"duration":0.5,"note":"C4"}]}},
        "tracks":[{"id":"lead","slot":"pulse1","instrument":"lead","clips":[{"pattern":"melody","at":0}]}],
        "sections":[{"id":"original","at":0,"length":4}]})
}

#[test]
fn preserves_existing_score_and_seconds_at_different_tempos() {
    let base = phrase("console8", 120.0);
    let take = phrase("console8", 60.0);
    let result = add_melody_layer(&base, &take, 1.5).unwrap();
    assert_eq!(result["tracks"][0], base["tracks"][0]);
    assert_eq!(result["patterns"]["melody"], base["patterns"]["melody"]);
    assert_eq!(result["instruments"]["lead"], base["instruments"]["lead"]);
    assert_eq!(result["sections"][0], base["sections"][0]);
    assert_eq!(result["tracks"][1]["clips"][0]["at"], 6.0);
    assert_eq!(result["patterns"]["layer_1"]["notes"][0]["at"], 1.0);
    assert_eq!(result["patterns"]["layer_1"]["notes"][0]["duration"], 1.0);
    assert_eq!(result["length"], 14.0);
    score::validate(&result).unwrap();
}

#[test]
fn chip4_assigns_distinct_melodic_channels_then_refuses_overflow() {
    let take = phrase("chip4", 120.0);
    let first = add_melody_layer(&take, &take, 0.0).unwrap();
    let second = add_melody_layer(&first, &take, 0.0).unwrap();
    assert_eq!(first["tracks"][1]["slot"], "pulse2");
    assert_eq!(second["tracks"][2]["slot"], "triangle");
    assert_eq!(second["instruments"]["layer_lead_2"]["wave"], "triangle");
    assert!(
        add_melody_layer(&second, &take, 0.0)
            .unwrap_err()
            .to_string()
            .contains("console8")
    );
    score::validate(&second).unwrap();
}

#[test]
fn repeated_layers_use_unique_ids_without_overwriting_backing() {
    let base = phrase("console8", 120.0);
    let first = add_melody_layer(&base, &base, 0.0).unwrap();
    let second = add_melody_layer(&first, &base, 1.0).unwrap();
    assert_eq!(second["tracks"][1], first["tracks"][1]);
    assert_eq!(second["tracks"][2]["id"], "layer_2");
    assert!(second["patterns"].get("layer_1").is_some());
    assert!(second["patterns"].get("layer_2").is_some());
    score::validate(&second).unwrap();
}

#[test]
fn refuses_invalid_positions_meter_and_render_duration() {
    let base = phrase("console8", 120.0);
    for at in [-1.0, f64::NAN, f64::INFINITY, 200.0] {
        assert!(add_melody_layer(&base, &base, at).is_err());
    }
    let mut waltz = base.clone();
    waltz["meter"] = json!([3, 4]);
    assert!(add_melody_layer(&base, &waltz, 0.0).is_err());
}
