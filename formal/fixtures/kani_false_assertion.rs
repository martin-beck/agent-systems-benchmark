// SPDX-License-Identifier: MIT

#[kani::proof]
fn retained_failure_fixture() {
    let value: bool = kani::any();
    assert!(value, "deliberate counterexample");
}
