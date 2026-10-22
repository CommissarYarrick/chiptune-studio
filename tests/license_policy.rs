use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn dependency_inventory_covers_the_exact_lockfile() {
    let inventory: Value =
        serde_json::from_str(include_str!("../licenses/inventory.json")).unwrap();
    let lock = include_str!("../Cargo.lock").replace("\r\n", "\n");
    assert_eq!(
        inventory["cargo_lock_sha256"],
        hex::encode(Sha256::digest(lock.as_bytes())),
        "Refresh the license inventory after reviewing Cargo.lock changes"
    );
    let packages = inventory["packages"].as_array().unwrap();
    let mut count = 0;
    for block in lock.split("[[package]]").skip(1) {
        if !block.lines().any(|l| l.starts_with("source = ")) {
            continue;
        }
        let field = |name: &str| {
            block
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{name} = \"")))
                .and_then(|l| l.strip_suffix('"'))
                .unwrap()
        };
        let name = field("name");
        let version = field("version");
        assert!(
            packages
                .iter()
                .any(|p| p["name"] == name && p["version"] == version),
            "Missing license review: {name} {version}"
        );
        count += 1;
    }
    assert_eq!(count, packages.len());
    assert!(!include_str!("../licenses/THIRD_PARTY.txt").is_empty());
}

#[test]
fn only_reviewed_permissive_license_selections_are_used() {
    let inventory: Value =
        serde_json::from_str(include_str!("../licenses/inventory.json")).unwrap();
    let reviewed = inventory["reviewed_expressions"].as_object().unwrap();
    for package in inventory["packages"].as_array().unwrap() {
        let upstream = package["upstream_license"].as_str().unwrap();
        let selected = package["selected_license"].as_str().unwrap();
        let expected = match upstream {
            "MIT"
            | "MIT OR Apache-2.0"
            | "Apache-2.0 OR MIT"
            | "MIT/Apache-2.0"
            | "Unlicense OR MIT"
            | "Zlib OR Apache-2.0 OR MIT"
            | "MIT OR Apache-2.0 OR LGPL-2.1-or-later" => "MIT",
            "BSD-3-Clause" => "BSD-3-Clause",
            "Apache-2.0" => "Apache-2.0",
            "(MIT OR Apache-2.0) AND Unicode-3.0" => "MIT AND Unicode-3.0",
            _ => panic!("Unreviewed license expression: {upstream}"),
        };
        assert_eq!(selected, expected);
        assert_eq!(reviewed[upstream], expected);
        assert!(!package["notice_files"].as_array().unwrap().is_empty());
    }
}
