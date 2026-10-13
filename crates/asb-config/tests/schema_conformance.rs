// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Schema-level positive and hostile fixtures for the explicit setup contract.

use jsonschema::validator_for;
use serde_json::json;

const CONFIG_SCHEMA: &str = include_str!("../schema/v1/explicit-setup-config.schema.json");
const PLAN_SCHEMA: &str = include_str!("../schema/v1/config-bound-plan.schema.json");

#[test]
fn schemas_accept_uppercase_digests_and_reject_duplicate_selections() {
    let config_schema = validator_for(&serde_json::from_str(CONFIG_SCHEMA).expect("schema"))
        .expect("compile schema");
    let value = json!({
        "schema_version": 1, "config_id": "demo",
        "agents": [{"id":"agent","executable":"agent-bin","adapter":"native"}],
        "providers": [{"id":"mock","adapter":"local-mock","credential":{"kind":"helper","locator_sha256":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}}],
        "models": [{"id":"model","provider":"mock"}], "workloads": [{"id":"workload","revision":"v1"}],
        "monitors": ["wall_time"], "measures": ["score"],
        "run": {"repetitions":1,"max_attempts":1,"timeout_seconds":1},
        "execution": {"results_root":"results","live":false,"network":false,"cost_acknowledged":false},
        "registries": {"provider":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","workload":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","measurement":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
    });
    assert!(config_schema.is_valid(&value));
    let mut duplicate = value.clone();
    duplicate["measures"] = json!(["score", "score"]);
    assert!(!config_schema.is_valid(&duplicate));
}

#[test]
fn plan_schema_rejects_duplicate_agents() {
    let schema =
        validator_for(&serde_json::from_str(PLAN_SCHEMA).expect("schema")).expect("compile schema");
    let value = json!({"schema_version":1,"plan_id":"plan","config_id":"demo","config_sha256":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","workloads":["workload"],"agents":["agent","agent"]});
    assert!(!schema.is_valid(&value));
}
