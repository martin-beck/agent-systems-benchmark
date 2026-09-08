// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

#[kani::proof]
fn retained_failure_fixture() {
    let value: bool = kani::any();
    assert!(value, "deliberate counterexample");
}
