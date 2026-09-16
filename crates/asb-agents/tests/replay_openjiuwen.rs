// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Strict, credential-free replay qualification for the OpenJiuwen boundary.

use asb_replay::{
    CassetteLimits, Header, ProviderDialect, ReplayError, ReplayHttpRequest, ReplayLimits,
    ReplayRoute, StrictReplayService, decode_cassette,
};
use serde_json::Value;

const CASSETTE: &[u8] = include_bytes!("fixtures/openjiuwen-cassette.json");

fn service() -> StrictReplayService {
    let cassette = decode_cassette(CASSETTE, CassetteLimits::default()).unwrap();
    StrictReplayService::new(cassette, ReplayLimits::default()).unwrap()
}

fn request() -> ReplayHttpRequest {
    let fixture: Value = serde_json::from_slice(CASSETTE).unwrap();
    let request = &fixture["contents"]["interactions"][0]["request"];
    ReplayHttpRequest {
        method: request["method"].as_str().unwrap().into(),
        path: request["path"].as_str().unwrap().into(),
        headers: request["headers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|header| Header {
                name: header["name"].as_str().unwrap().into(),
                value: header["value"].as_str().unwrap().into(),
            })
            .collect(),
        body: serde_json::to_vec(&request["body"]).unwrap(),
    }
}

fn route() -> ReplayRoute {
    ReplayRoute {
        session_id: "gemini-public-session".into(),
        attempt_id: "attempt-1".into(),
        dialect: ProviderDialect::GeminiGenerateContent,
    }
}

#[test]
fn strict_openjiuwen_replay_matches_sealed_cassette_without_network() {
    let response = service().handle(&route(), request()).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.segments.len(), 1);
    assert!(
        response.segments[0]
            .windows(b"done".len())
            .any(|window| window == b"done")
    );
}

#[test]
fn strict_openjiuwen_replay_rejects_wrong_route_and_corruption() {
    let wrong = ReplayRoute {
        attempt_id: "stale-attempt".into(),
        ..route()
    };
    assert!(matches!(
        service().handle(&wrong, request()),
        Err(ReplayError::UnknownRoute)
    ));

    let mut corrupt = CASSETTE.to_vec();
    let midpoint = corrupt.len() / 2;
    corrupt[midpoint] ^= 1;
    assert!(decode_cassette(&corrupt, CassetteLimits::default()).is_err());
}
