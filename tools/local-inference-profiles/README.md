# Local inference profile evidence

`profiles-v1.json` is the machine-readable evidence boundary for the grouped
Ollama, llama.cpp, vLLM, and LocalAI workstream. Run the validator from this
directory:

```sh
python3 -m unittest -v
python3 profile.py profiles-v1.json
```

Every profile records immutable frontend provenance, optional backend
provenance, model and tokenizer identities, route claims, configuration,
loopback/resource/credential/teardown policy, and qualification evidence.

An `unqualified` profile is deliberately non-selectable and must carry an
explicit reason. It is not a fallback, a mock, or a compatibility claim. The
current manifest therefore does not expand the CLI/TUI catalog: the existing
Ollama runtime path remains governed by its Rust probe and model digest until
an independent tokenizer digest and repeated trial evidence are recorded.
