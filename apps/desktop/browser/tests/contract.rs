//! Cross-edition contract checks.
//!
//! The canonical action list lives in `packages/tool_contracts/browser.json` so
//! the Rust and Python editions cannot silently grow different browser
//! vocabularies. This test is the Rust half; `scripts/check-browser-parity.py`
//! is the Python/Docker half.

use std::collections::BTreeSet;

use mixd_browser::{
    browser_contract, browser_tool_spec, desktop_agent_actions, desktop_contract_actions,
};

fn spec_actions() -> Vec<String> {
    browser_tool_spec()["parameters"]["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[test]
fn the_contract_is_valid_and_versioned() {
    let contract = browser_contract();
    assert_eq!(contract["version"], 1);
    assert!(
        contract["actions"]
            .as_object()
            .is_some_and(|map| !map.is_empty()),
        "contract must declare actions"
    );
    assert!(
        contract["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "contract must declare stable error codes"
    );
}

#[test]
fn the_tool_spec_exposes_exactly_the_model_facing_contract() {
    // The surface exposes every action, but `manual_only` ones (raw viewport
    // clicks and typing into whatever has focus) are for the live view, not
    // the model. The spec must be the contract minus that list.
    assert_eq!(spec_actions(), desktop_agent_actions());
    let contract = browser_contract();
    let manual = contract["surfaces"]["desktop"]["manual_only"]
        .as_array()
        .expect("manual_only")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    for action in manual {
        assert!(
            !spec_actions().iter().any(|a| a == action),
            "manual-only action {action} must not be advertised to the model"
        );
    }
    let all = desktop_contract_actions();
    let unique: BTreeSet<&String> = all.iter().collect();
    assert_eq!(
        unique.len(),
        all.len(),
        "the desktop contract contains duplicates"
    );
}

#[test]
fn every_declared_action_has_a_description_and_args() {
    let contract = browser_contract();
    for (name, action) in contract["actions"].as_object().expect("actions object") {
        assert!(
            action["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "{name} is missing a description"
        );
        assert!(
            action["args"].is_object(),
            "{name} is missing an args object"
        );
    }
}

#[test]
fn the_docker_surface_does_not_reference_unknown_actions() {
    let contract = browser_contract();
    let known: BTreeSet<String> = contract["actions"]
        .as_object()
        .expect("actions object")
        .keys()
        .cloned()
        .collect();
    let docker = &contract["surfaces"]["docker"];
    for (tool, action) in docker["tools"].as_object().expect("docker tools") {
        assert!(
            known.contains(action.as_str().expect("action name")),
            "{tool} maps to unknown action {action}"
        );
    }
    for action in docker["unsupported"].as_array().expect("unsupported") {
        assert!(
            known.contains(action.as_str().expect("action name")),
            "unsupported list contains unknown action {action}"
        );
    }
}
