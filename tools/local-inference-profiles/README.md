# Local inference profile evidence

`profiles-v1.json` is the machine-readable evidence boundary for the grouped
Ollama, llama.cpp, vLLM, and LocalAI workstream. Run the validator from this
directory:

```sh
python3 -m unittest -v
python3 profile.py profiles-v1.json
python3 gguf_tokenizer_digest.py /path/to/pinned-model.gguf
```

Every profile records immutable frontend provenance, optional backend
provenance, model and tokenizer identities, route claims, configuration,
loopback/resource/credential/teardown policy, and qualification evidence.

`gguf_tokenizer_digest.py` reads only bounded GGUF metadata and hashes the
canonical `tokenizer.ggml.*` keys; it never reads tensor payloads. The recorded
Ollama tokenizer digest was produced from the pinned model blob using this tool.

An `unqualified` profile is deliberately non-selectable and must carry an
explicit reason. It is not a fallback, a mock, or a compatibility claim. The
The Ollama profile is selectable only for the exact local-live evidence in the
manifest. The llama.cpp, vLLM, and LocalAI profiles remain non-selectable until
their own immutable executable/model/backend and repeated-trial evidence exists.
