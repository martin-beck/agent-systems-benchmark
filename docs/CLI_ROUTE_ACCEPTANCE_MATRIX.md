# CLI route acceptance matrix

This matrix is the AR-1770 regression surface for command-owned directory
preparation and stream separation. Every route must keep JSON stdout machine
readable, keep human progress and recovery guidance on stderr, and fail closed
without mutating a destination when dry-run, permission, read-only, symlink,
or rollback preconditions are not satisfied.

| Route family | Human and machine coverage | Filesystem and failure coverage |
| --- | --- | --- |
| `setup` / `config` | `human_cli::unicode_and_combining_output_obeys_terminal_display_columns`; `setup_contract` | `setup_preflight_is_machine_readable_and_side_effect_free`; atomic configuration tests |
| `project` / `tool` | `human_cli::project_init_human_output_distinguishes_initialization_from_recovery`; tool outcome tests | `project_init_rejects_symlink_layout_entries`; tool install replacement and rollback tests |
| `plan` | `human_cli::plan_create_new_options_are_discoverable_and_fail_closed`; workflow transcript | `plan_create_selects_catalog_workload_and_round_trips_validation`; unavailable and malformed output rejection |
| `run` / `sweep` | `human_cli::executable_outcome_matrix_covers_product_dependency_host_failed_and_inconclusive`; JSON selector tests | `attempt_timeout_is_bounded_and_cleans_the_private_workspace`; descriptor-safe private roots and prompt creation |
| `report` | `guide_examples::documented_offline_workflow_produces_validated_artifacts` | bounded run-root inspection and missing-artifact failures |
| `record` / `campaign` | recording workflow transcript and typed error tests | `record_campaign_requires_exact_matrix_before_offline_ready`; staging rollback and symlink denial |
| `easy` lifecycle | `easy_lifecycle_local_journey_covers_build_install_update_test_rollback_remove` | dry-run confirmation, atomic marker publication, rollback and cleanup |
| `tui` | TUI lifecycle and human output integration tests | PTY/inherited-fd, read-only, stale-tool, symlink and bounded cleanup tests |

The applicable local gate is:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

The route tests are intentionally offline and use bounded local fixtures. They
do not claim live-provider, native-kernel, or public-release evidence.

The helper-level negative evidence is `safe_fs::tests::file_parent_fails_without_creating_children`
for inaccessible/non-directory parents and
`safe_fs::tests::concurrent_child_creation_reopens_the_winner_without_following_a_link`
for concurrent reuse. Existing `filesystem_object_boundaries_fail_closed`,
`destination_material_is_never_overwritten`, and recording rollback tests cover
permission-shaped, read-only, replacement, and rollback outcomes at the public
workload boundary.
