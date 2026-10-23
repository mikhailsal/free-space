//! `skill/schema.json` must equal `freespace schema --json` (T22). Regenerate
//! with `UPDATE_SCHEMA=1 cargo test -p freespace-core --test schema`.

use std::path::Path;

use freespace_core::queries::types::schema_document;

#[test]
fn schema_json_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../skill/schema.json");
    let want = serde_json::to_string_pretty(&schema_document()).unwrap() + "\n";
    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "skill/schema.json is stale: run UPDATE_SCHEMA=1 cargo test -p freespace-core --test schema"
    );
}

#[test]
fn every_entry_requires_newest_mtime() {
    let doc = schema_document();
    let req = doc["$defs"]["EntryOut"]["required"].as_array().unwrap();
    assert!(req.iter().any(|v| v == "newest_mtime"));
    for c in ["status", "ls", "top", "find", "diff", "doctor", "error"] {
        let r = doc["commands"][c]["$ref"].as_str().unwrap();
        let name = r.trim_start_matches("#/$defs/");
        assert!(doc["$defs"].get(name).is_some(), "{c} → {name}");
    }
}
