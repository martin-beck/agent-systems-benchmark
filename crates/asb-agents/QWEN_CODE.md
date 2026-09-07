# Qwen Code adapter evidence

ASB targets the official Apache-2.0 Qwen Code `v0.23.0` release at unsigned
upstream commit `98a9c964158697dd5631d15a62174684ff7bbb53` and tree
`b1142a1c52088d500220a4bab5a6cfa37eaf2aa1`. The inspected source license has
SHA-256 `55367b61ccd2a016a0159ad886bd66a3ee6cb5e873d0c75c803c897dd245b075`.

The native Linux x86_64 boundary uses the official standalone archive:

- archive: `da20f7227e1daa0834c6dd0b46f23a72a2eeb083b9b21e73cb53e182674b05bd`
- launcher: `1461b5ef9149b86e58ea22ddb9cc0616679437f9ff0011cf7c3c102f94568abf`
- bundled Node.js 22.23.2: `3517c2df0b2f8cd7f422b4b8450ef81c6889f08eb03e281d6de9079b15e6a327`
- launcher-imported CLI entry: `68cb29eb7ccc936d78ece5564ef55cae41a55b630e6657dc417c1f2e561cf4c9`
- primary bundled CLI module: `c85176761861172aa41003fed2b80b99ee73d797a1c60fca9e160086e5390524`

The official Linux aarch64 archive was inspected at SHA-256
`f6f8f434f8531e90a5a1624181e4e937d97b7ddc72b3ba72b7963a387697dc8c`,
but it has not been exercised natively and is not a supported adapter artifact.
The npm package advertises integrity
`sha512-foznQtmptzM7DtYGPALZVtwUBVTsANYc6ytWOfGisXB7FgpzNmYT6xIHzh+26RJD9Kl+qzRW0gFp/AkqRouhnQ==`
and shasum `01a83323ddf99f2b9b0e76e87c0ea42fab1d24c2`; ASB does not execute that
package in this revision.

## Boundary

The prompt is supplied on an unlinked descriptor, never in argv. Qwen Code
runs headlessly with stream-JSON output, `--safe-mode`, chat recording disabled,
an explicit OpenAI-compatible endpoint/model, a credential-free sentinel,
`auto-edit`, and native turn, wall-clock and tool-call ceilings. Shell, subagent,
web and computer-use tools are excluded. ASB clears the environment, gives the
process private HOME/XDG/runtime directories, disables telemetry/update checks,
and points ambient proxies at a closed listener while allowing only the selected
provider host through `NO_PROXY`. The sandbox runtime remains the authoritative
network and filesystem containment boundary.

The adapter validates a bounded regular-file workspace and rejects symlinks,
special files, excessive entries, long paths, large files and excessive total
bytes. Stream parsing rejects duplicate JSON members, malformed ordering,
mixed upstream session IDs, orphan/duplicate tool results and oversized lines
or event counts. Only causal lifecycle, tool name/ID, success and token counts
leave the adapter; prompt, response, reasoning, arguments, results and raw
diagnostics do not.

Pinned Qwen Code can encode an HTTP provider rejection as an ordinary assistant
message followed by a successful zero-token result. ASB therefore requires
positive input and output token evidence before accepting a completed attempt;
missing or zero usage fails closed. A native loopback HTTP 400 fixture exercises
this boundary without retaining the provider diagnostic.

## Limits

Native evidence is Linux glibc x86_64 only. `auto-edit` permits Qwen Code's
write/edit tools; ASB deliberately excludes shell and remote-capable tools.
Internal provider retries are not independently counted by this adapter.
The archive and directly executed launcher, Node, CLI entry and primary module are pinned, but
this AR does not prove a reproducible content manifest for every transitive file
in the standalone runtime; AR-0316 owns that bundle-wide claim. Safe mode blocks
ambient QWEN.md, hooks, extensions, skills, MCP servers and custom subagents,
but built-in tools and built-in agents can still appear in the content-free init
frame. Cancellation may interrupt the final JSON frame; only prior complete
newline-delimited frames are retained.
