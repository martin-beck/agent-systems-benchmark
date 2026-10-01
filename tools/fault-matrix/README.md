# ASB deterministic fault matrix

`run.py` executes a bounded manifest of setup, recording, replay, benchmark,
and recovery cases without a shell, ambient credentials, or network access.
On Linux, each case is launched in an `unshare --net` network namespace; if
that isolation primitive is unavailable the case is reported as the typed
`runner_unavailable` result rather than falling back to the host network.
Human output is the default; pass `--json` for the versioned machine result.
Each case has a bounded timeout, capped output, bounded private workspace,
streamed output, an explicit expected exit, and
optional private-root cleanup assertions. Development-only missing auth,
signatures, and keys may be classified as warnings; production failures remain
fail-closed cases in the manifest.

Example:

```sh
python3 tools/fault-matrix/run.py \
  --manifest tools/fault-matrix/manifest-v1.json \
  --binary target/debug/asb --json
```
