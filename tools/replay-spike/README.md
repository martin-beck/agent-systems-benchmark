# Replay candidate spike fixture

This standard-library-only synthetic provider supplies the same public,
credential-free trajectories to each AR-0501 candidate: buffered JSON, paced
SSE text and tool events, causal response IDs, parallel session IDs, an HTTP
error, a deliberately truncated stream, malformed input, and an unmatched
route. It never contacts a model provider.

Run the independent fixture checks with:

```sh
python3 -m unittest discover -s tools/replay-spike -p 'test_*.py'
```

Candidate adapters must record from this origin, stop it, deny outbound
network, and then replay the same requests. A pass requires exact status/body
or ordered SSE-event fidelity, measured event offsets, session-local cursors,
preserved tool/causal IDs, an observable incomplete cancellation trajectory,
redacted persisted secrets, and a non-successful miss with no fallback.
Buffered replay alone is not an SSE pass. A configurable delay alone is not an
original-pacing pass. This fixture is research tooling and does not establish
provider-dialect or native-platform support.
