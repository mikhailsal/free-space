//! Phase tracing is opt-in and supplies complete boundaries to the memory harness.
use serde_json::Value;
use std::fs;

mod common;
use common::Env;

#[test]
fn scan_profiles_each_engine_without_changing_json_output() {
    let env = Env::new();
    fs::create_dir(env.root()).unwrap();
    fs::write(env.root().join("file"), b"payload").unwrap();
    let baseline = env.run(&["scan", env.root().to_str().unwrap(), "--json"]);
    assert!(baseline.status.success());
    let baseline: Value = serde_json::from_slice(&baseline.stdout).unwrap();
    for low_memory in [false, true] {
        let path = env.tmp.path().join(format!("profile-{low_memory}.jsonl"));
        let mut command = env.cmd(&["scan", env.root().to_str().unwrap(), "--json"]);
        if low_memory {
            command.arg("--low-memory");
        }
        let output = command
            .env("FREESPACE_MEMORY_PROFILE", &path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["entries"], baseline["entries"]);
        assert_eq!(
            report.as_object().unwrap().keys().collect::<Vec<_>>(),
            baseline.as_object().unwrap().keys().collect::<Vec<_>>()
        );
        let events: Vec<Value> = fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let expected = if low_memory {
            vec!["walk", "write", "index", "publish", "done"]
        } else {
            vec!["walk", "finalize", "write", "index", "publish", "done"]
        };
        let actual: Vec<&str> = events
            .iter()
            .map(|event| event["phase"].as_str().unwrap())
            .collect();
        assert_eq!(actual, expected);
        assert!(events.iter().all(|e| e["rss_kib"].as_u64().unwrap() > 0));
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0]["elapsed_seconds"].as_f64()
                    <= pair[1]["elapsed_seconds"].as_f64())
        );
    }
}
