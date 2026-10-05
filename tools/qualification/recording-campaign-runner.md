# Deterministic recording acceptance runner

`recording-campaign-runner.py` qualifies the AR-1711/1712 recording handoff
without provider traffic. It creates a selected-agent × selected-workload
matrix from the checked-in buffered cassette fixture, invokes only
`record-campaign --local-mock`, and verifies that every sealed cassette is
published and the result is `offline_ready`.

The default selection covers every catalog agent and every currently available
fixture workload. Narrow checks can pass comma-separated `--agents` or
`--workloads`. The runner never calls `benchmark-live`, never reads an API key,
and must not be used as live-provider evidence.

Example:

```sh
python3 tools/qualification/recording-campaign-runner.py \
  --asb target/debug/asb \
  --fixture crates/asb-replay/fixtures/v1/buffered.json \
  --output-dir /tmp/asb-recording-qualification
```
