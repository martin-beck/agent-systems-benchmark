# Compare agents on one benchmark definition

Comparison is meaningful only when every run uses the same benchmark/workload
revision, measurement definition and configuration digest. Report each terminal
run before comparing it; a failed, cancelled, unavailable or otherwise
incomplete run remains in the attempted denominator and is never converted to
zero or silently discarded.

The offline tutorial uses synthetic run IDs and does not launch an agent,
provider or benchmark:

```text
asb report results/runs/codex-run
asb report results/runs/gemini-run
asb compare results/runs/codex-run results/runs/gemini-run
```

Only a structured result with `comparable: true` supports a difference claim.
If identities differ, or any terminal run is failed or incomplete, preserve the
reason and report the comparison as non-comparable. The comparison fixtures
include both mismatched-definition and dropped-failure negative cases.

Validate the syntax contract offline:

```sh
python3 tools/tutorials/validate.py tools/tutorials/result-comparison-v1.json
```

The command validates syntax only; it does not run benchmark or analysis
commands.
