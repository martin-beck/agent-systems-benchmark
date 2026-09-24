# Offline benchmark readiness

Use the readiness tutorial to check the local ASB contract without launching an
agent, contacting a provider, or reading a credential. It checks the installed
command and workload inventories, validates a provider/model configuration,
creates a digest-only provider selection, and validates a benchmark plan.

The selection uses only a digest of a logical credential reference. It is not a
credential, does not prove provider reachability or authorization, and does not
run a benchmark. A successful syntax/readiness check is therefore not live
qualification.

The checked-in contract is [`benchmark-readiness-v1.json`](../../tools/tutorials/benchmark-readiness-v1.json).
Validate it offline with:

```sh
python3 tools/tutorials/validate.py tools/tutorials/benchmark-readiness-v1.json
```

The `benchmark-experiment.toml` and `benchmark-selection.json` references are
bounded repository-relative inputs/outputs for the tutorial contract; they do
not authorize network access or agent execution.
