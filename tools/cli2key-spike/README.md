# cli2key contract spike

This tool validates the development-only, unofficial cli2key boundary in
[`CLI2KEY_DEVELOPMENT.md`](../../docs/CLI2KEY_DEVELOPMENT.md). The normal test is
credential-free and uses an explicit `not-a-secret` authorization marker plus a
loopback fake:

```sh
python3 tools/cli2key-spike/cli2key_spike.py --fake
python3 -m unittest discover -s tools/cli2key-spike -p 'test_*.py'
```

Run the complete credential-free qualification (setup, discovery, one run,
bounded sweep, results/comparison, cancellation, reset, redaction, and
sidecar cleanup) with:

```sh
python3 tools/cli2key-spike/qualification.py
```

It uses one supervised fake-sidecar lifetime and at most two concurrent
attempts. The JSON is synthetic development evidence only and contains no
generated key, OAuth material, input, provider body, or private path.

The live spike is deliberately opt-in. Start the exact pinned bridge on an
ephemeral numeric loopback address against an existing user-approved Codex login,
then supply the freshly rotated invocation key and input through inherited
environment variables. Neither value is accepted on argv or written to the
report:

```sh
read -rs ASB_CLI2KEY_CLIENT_KEY
export ASB_CLI2KEY_CLIENT_KEY
read -rs ASB_CLI2KEY_SPIKE_INPUT
export ASB_CLI2KEY_SPIKE_INPUT
python3 tools/cli2key-spike/cli2key_spike.py \
  --confirm-live \
  --endpoint http://127.0.0.1:8317 \
  --model MODEL_ID \
  --output cli2key-development-receipt.json
unset ASB_CLI2KEY_CLIENT_KEY ASB_CLI2KEY_SPIKE_INPUT
```

The receipt contains only pinned bridge identity, route outcomes, model identity,
counts, and fixed warnings. It never contains OAuth material, the local client
key, authorization headers, input, provider bodies, private paths, or raw
exception text. A successful receipt is development-live evidence only; it is
not official OpenAI support, an OpenAI Platform API key, production authority,
or release qualification.
