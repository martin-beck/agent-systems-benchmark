# Deterministic LLM-double conformance spike

This is research tooling owned by AR-0888. It does not add an ASB provider,
modify replay, or enter any candidate into a support catalog. The standard-library
fixture exposes OpenAI Chat/Responses and Anthropic Messages buffered and SSE
routes, rate limits, truncation, unmatched routes, and deterministic repeated
responses. It binds each assessed candidate to an immutable revision and license.

Run it without credentials or network access beyond loopback:

```sh
python3 tools/llm-double-spike/conformance.py --output /tmp/asb-llm-double.json
python3 -m unittest discover -s tools/llm-double-spike -p 'test_*.py'
```

The report labels evidence as `synthetic`; candidate rows remain `untested`
until an independently pinned executable artifact is supplied. This fail-closed
boundary prevents fixture success from being misreported as live-provider or
candidate qualification. No private prompts, credentials, hosts, or unbounded
responses are accepted.
