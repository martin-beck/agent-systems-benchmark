// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded local control boundary for independent ASB frontends.
//!
//! The runner remains authoritative. Frontends negotiate this protocol over an
//! owner-only Unix-domain socket and may disconnect without owning run lifetime.

mod endpoint;
mod frame;
mod protocol;
mod schema;
mod state;
mod transport;

pub use endpoint::*;
pub use frame::*;
pub use protocol::*;
pub use schema::{
    analysis_evidence_schema, control_event_schema, control_request_schema,
    control_request_schema_v1_2, control_response_schema, control_response_schema_v1_2,
    history_evidence_schema,
};
pub use state::*;
pub use transport::*;
