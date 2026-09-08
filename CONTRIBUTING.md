# Contributing

Follow [the development process](docs/DEVELOPMENT.md) and
[quality requirements](docs/QUALITY.md). Each change needs a coordination AR,
focused branch, matching Signed-off-by trailer and applicable exact-head evidence.

Sign each commit with an SSH key registered in `config/allowed_signers`. The
repository checks every introduced commit for both that cryptographic signature
and the matching authorship trailer.

The source-header policy requires exactly one canonical adjacent Huawei/MIT pair in every tracked
first-party Rust, Python, shell, TLA+, and Alloy source and in the extensionless `tools/awq`
launcher, after any required shebang or module declaration. Standalone matching copyright or SPDX
lines elsewhere in source or test data are allowed; a second adjacent canonical pair is rejected.
