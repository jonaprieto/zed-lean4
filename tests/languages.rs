//! Checks the files Zed reads from `languages/lean4` that no build step parses.
use std::{collections::HashSet, path::Path};

use zed_extension_api::serde_json::{self, Value};

fn read(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("languages/lean4")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn tasks_have_unique_labels_and_cover_the_lake_commands() {
    let tasks = read("tasks.json");
    let tasks = tasks.as_array().expect("a list of tasks");
    let labels: HashSet<&str> = tasks.iter().filter_map(|t| t["label"].as_str()).collect();
    assert_eq!(labels.len(), tasks.len(), "every task has its own label");
    let lake: Vec<Vec<&str>> = tasks
        .iter()
        .filter(|t| t["command"] == "lake")
        .map(|t| {
            let args = t["args"].as_array().expect("lake tasks pass args");
            args.iter().filter_map(Value::as_str).collect()
        })
        .collect();
    for wanted in [
        vec!["build"],
        vec!["update"],
        vec!["clean"],
        vec!["exe", "cache", "get"],
    ] {
        assert!(
            lake.contains(&wanted),
            "no `lake {}` task",
            wanted.join(" ")
        );
    }
}

#[test]
fn semantic_token_rules_are_a_list_of_styled_types() {
    let rules = read("semantic_token_rules.json");
    for rule in rules.as_array().expect("a list of rules") {
        assert!(rule["token_type"].is_string(), "{rule}");
        assert!(rule["style"].is_array(), "{rule}");
    }
}
