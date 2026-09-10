// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Print the independent-frontend capability schema.

fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&asb_cli::capabilities::capability_schema())
            .expect("capability schema is serializable")
    );
}
