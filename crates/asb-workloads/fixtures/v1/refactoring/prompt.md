Move slug normalization behind a private `normalize` module while preserving
the public `label` behavior. `src/lib.rs` must delegate to the new module and
must no longer define the normalization function itself.
