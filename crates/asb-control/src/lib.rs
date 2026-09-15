// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded local control boundary for independent ASB frontends.
//!
//! The runner remains authoritative. Frontends negotiate this protocol over an
//! owner-only Unix-domain socket and may disconnect without owning run lifetime.

mod catalog;
mod endpoint;
mod frame;
mod handoff;
mod lifecycle;
mod protocol;
mod schema;
mod state;
mod transport;

pub use catalog::*;
pub use endpoint::*;
pub use frame::*;
pub use handoff::*;
pub use lifecycle::*;
pub use protocol::*;
pub use schema::{
    analysis_evidence_schema, control_event_schema, control_request_schema,
    control_request_schema_v1_2, control_request_schema_v1_3, control_request_schema_v1_4,
    control_request_schema_v1_5, control_response_schema, control_response_schema_v1_2,
    control_response_schema_v1_3, control_response_schema_v1_4, control_response_schema_v1_5,
    history_evidence_schema,
};
pub use state::*;
pub use transport::*;
