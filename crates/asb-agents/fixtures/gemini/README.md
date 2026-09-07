# Gemini CLI adapter evidence

This adapter targets the official Apache-2.0
`google-gemini/gemini-cli` release `v0.58.0`:

- tag commit: `ac9431c9e2290d68af31a77614ff2fddb2391ca3`
- Git tree: `5dbf74073ce1f63693abdf02d88136d7f018eeb6`
- license SHA-256:
  `58d1e17ffe5109a7ae296caafcadfdbe6a7d176f0bc4ab01e12a689b0499d8bd`
- npm package: `@google/gemini-cli@0.58.0`
- npm shasum: `31fdcb4e46f2185519fa2e131a3901f8ffa1989a`
- npm integrity:
  `sha512-++LtUYMcLE8dVxMcuwv6kIp8+h6z+std/7iVE+vSunkrwNDaWMFkWw/psv2RSySWjr2A1SsEEIGCK0xULWY2sA==`
- downloaded tarball SHA-256:
  `8ffeb9e7edddffb054764d00749f39e8cc9804ca9b38b9093f906dd2157322ae`
- exercised entry bundle SHA-256:
  `25f087a42f4484891aa73e6e36cc2790e69a3c023cc44fd244adf44a091eebd7`
- canonical digest of all 446 extracted bundle files:
  `3e6332e8d8117f0bb41bffd67df0f216e305c745d6ab63c5e27e98769da492e1`
- exercised Node.js 26.3.0 Linux x86_64 SHA-256:
  `5325ac9da58541494afcc136f0880279a2a853609bf4dae7755e04fb682b6926`

The native fixture uses the pinned npm entry and Node runtime against a
credential-free loopback SSE provider. It proves a real file edit, structural
stream-JSON lifecycle/tool/usage evidence, cancellation, process-group cleanup,
private state cleanup, and proactive action exhaustion before action max+1 can
modify the workspace. Prompt, response, tool arguments/results and hook input
are never retained as evidence.

## Boundary and limitations

`GOOGLE_GEMINI_BASE_URL` alone makes v0.58.0 select its `GATEWAY` auth type,
which this release then rejects. The isolated fixture therefore selects
`gemini-api-key` and supplies a fixed public sentinel key; no credential or
hosted service is used.

The supported system settings/defaults environment variables are redirected to
private empty files. Because v0.58.0 has a fixed Linux system-policy location,
the adapter also fails before workspace/state mutation if any recognized
`/etc/gemini-cli` settings, defaults, or policy path exists.

The adapter permits only `write_file` and `replace`. A private user-tier
policy rejects every other tool, and an official `BeforeTool` command hook
atomically enforces the action budget. Startup returns only after a bounded
private `SessionStart` marker proves that the hook system is active. Session
turns are bounded by the pinned
CLI setting. Stream lines, total events, process output, prompt bytes and
identifiers are independently bounded and malformed output fails closed.

Only Linux x86_64 native npm execution is currently evidenced. Linux aarch64,
macOS, Windows, consumer OAuth, Vertex AI, direct Gateway authentication,
hosted Gemini service tiers, MCP, shell/web tools, replay compatibility and
non-loopback HTTP endpoints are unsupported. HTTPS custom endpoints are a
validated configuration route but have no live-service evidence. The adapter
verifies the complete extracted bundle tree before each run. The inspected npm
artifact still does not establish a reproducible source build. Process-group
cancellation is not a cgroup containment claim for daemonized descendants.
The extracted package directory is an immutable, coordinator-owned input;
concurrent same-user replacement after verification is outside this adapter's
process-boundary guarantee.
