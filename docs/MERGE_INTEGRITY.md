# Signed merge integrity

ASB integrates a reviewed pull request by constructing a local two-parent merge whose tree is the
approved pull-request tree. GitHub requires one pull-request merge method to remain enabled, so the
repository exposes merge-commit only. An active protected-main ruleset requires the portable
provenance check, which rejects GitHub Web Flow and noreply identities and validates the exact
allowed-SSH, DCO, parent, reviewed-head, and reviewed-tree publication. The metadata email rule is
retained only when bounded owner-plan evidence proves that capability is supported. Squash, rebase,
auto-merge, and every web-created merge remain unauthorized.

Before integration, record full object IDs for current remote `main`, the reviewed pull-request
head, and its tree. Use a clean isolated integration worktree at the exact base and run:

```sh
python3 tools/integration/merge_pr.py \
  --pr-ref refs/pull/NUMBER/head \
  --expected-base BASE_OID \
  --expected-head HEAD_OID \
  --expected-tree TREE_OID \
  --subject 'Merge reviewed PR #NUMBER' \
  --push
```

The command refuses a dirty tree, stale target or pull-request ref, non-descendant head, wrong
tree, missing DCO, untrusted signature, or a signer principal that differs from the matching
author, committer, and DCO identity. It requalifies the protected target and pull-request refs
after object refresh and immediately before publication; any target advancement requires a fresh
exact-main qualification. Before creating the commit, it previews the exact two-parent merge with
`git merge-tree` and requires the preview tree to equal the reviewed tree. It uses
`git commit-tree -S` to create a signed, DCO-trailered
no-fast-forward merge with parents exactly `BASE_OID HEAD_OID` and tree exactly `TREE_OID`.
Immediately before publication, the integration command rechecks the target and pull-request refs,
the reviewed topic tree and ancestry, and the constructed merge's exact parents, tree, signature
and DCO. It then reads the target and pull-request refs together again and uses an exact
`--force-with-lease` for the protected target. A transport error is not proof of failure: if the
post-error read shows the exact merge, it records acceptance; unchanged or divergent state fails
closed and requires a new invocation after reconciliation. Diagnostics are bounded and generic and
never copy Git, transport, user, machine, path, or credential text.

Every required post-merge workflow keeps its normal cancellation behavior for pull-request and
manual runs, but never cancels a `push` run on protected `main`. The workflow concurrency groups
therefore queue concurrent main pushes and preserve each immutable merge SHA's exact evidence;
later main activity cannot silently cancel the earlier merge's assurance run.

Repository administrators apply the GitHub-compatible merge-only settings and exact protected-main
ruleset after reviewing the change:

```sh
python3 tools/integration/repository_settings.py \
  --apply --expected-ruleset-id 24750310
```

Ruleset `24750310` is the immutable owned identity. The tool updates that ID in
place with PUT and never creates, deletes, recreates, or selects mutation ownership
by name. Every partial or ambiguous path after GitHub returns the positive ID also
prints that ID and classifies its ownership as either `response-only` or
`readback-verified`. Record the ID and classification from success **or failure**
in the owning AR's durable external-settings receipt; never publish raw
authenticated API output. Every apply and audit binds the recorded identity:

```sh
python3 tools/integration/repository_settings.py --apply --expected-ruleset-id 24750310
python3 tools/integration/repository_settings.py --expected-ruleset-id 24750310
```

A circular legacy approval rule may be recovered only with the separately
reviewed, exact-ID, ruleset-only boundary:

```sh
python3 tools/integration/repository_settings.py \
  --apply --ruleset-only --expected-ruleset-id 24750310
python3 tools/integration/repository_settings.py \
  --ruleset-only --expected-ruleset-id 24750310
```

The first command performs the same identity, protected-head, capability,
inventory, strict-settings-tuple, exact-ID, response-normalization, and fresh
readback barriers as normal apply, but returns immediately after the verified
ruleset readback and can never PATCH repository settings. The second command is
read-only: it admits only the exact documented pre/final settings tuple and
requires the desired ruleset. This is a recovery-only bootstrap, not a general
administrative interface. Offline fixtures are incompatible, and no live use is
permitted until its exact candidate has independent review, all hosted checks,
and an explicit execution gate.

An existing same-name ruleset without that exact recorded ID is foreign and fails before mutation.
The request sends an explicit empty `required_reviewers`, zero GitHub approvals,
and no last-push approval. GitHub's response may add exactly
`require_extra_approval_for_unattributed_changes=true`; the canonicalizer removes
only that response normalization and then requires exact request equality, including
the empty reviewers. False or non-boolean normalization, nonempty or malformed
reviewers, or any unknown response key fails closed. After PUT, the tool performs a
fresh complete ID-bound ruleset/inventory readback and requires stable repository
identity, protected-main head, owner plan/capability, and foreign inventory before
it attempts the repository-settings PATCH. The admission check accepts only the
exact normalized owned policy recorded at the fixed pre-apply boundary or the
already-updated policy; it validates repository identity, shape, and provenance
without incorrectly requiring the later PATCH result. The five merge/signoff
settings must also equal one complete strict-Boolean tuple: either documented
prestate `true,true,true,false,false` or final state
`true,false,false,false,true`, in merge/squash/rebase/auto/signoff order. Missing,
non-Boolean, or mixed tuples fail before PUT. Full desired settings and policy
validation remains mandatory after PATCH.
The second command is the required read-only audit: it binds the index summary, fetched detail, and
durable expected ID and emits the same `ruleset-id=N; ownership=readback-verified` receipt.

The ruleset requires a pull request, resolved review threads, strict exact-head
checks including Portable protected-main provenance, verified signatures, and
merge commits only. Development review is instead durable independent-agent
Coordinator evidence; the same GitHub account may publish after that separate
technical review, so the GitHub approving count is zero and last-push approval is
false. This does not relax the independent review, exact-head CI, signed local
merge, DCO, portable provenance, merge-only admission, or resolved-thread gates.
The portable check rejects Web Flow/noreply identities; a supported Enterprise
organization also retains the metadata rule. Deletion and non-fast-forward updates are prevented.
The repository setting
keeps merge-commit enabled solely because GitHub rejects disabling all three pull-request merge
methods; it disables squash, rebase, and auto-merge and requires web signoff. The second command is
a fail-closed audit of both settings and ruleset. The external change must be recorded with its exact
response; failure means protected-main admission is not ready and must be escalated to a repository
administrator. It does not retroactively repair an old merge. Historical merge
`b6d04a8305ce6d49cc327e4e6d2d6fa42a88050b` remains unsigned/non-DCO. A signed descendant can
restore a green current-main evidence range, but must not be described as changing that history.

The ruleset payload is structurally checked against the repository projection of GitHub's
immutable REST OpenAPI 2022-11-28 description. Its upstream commit, Git blob, full-file SHA-256,
and relevant rule-type enumeration are pinned in
[github-ruleset-openapi.json](../config/github-ruleset-openapi.json). Structural availability does
not establish account capability; owner type and plan are admitted separately before mutation.

The later OpenJiuwen merge `1c07e907a6fdf270264a94bef4af6b8ac4e5cbaf` is likewise preserved as published
history after its post-merge policy check rejected the missing DCO trailer. Recovery must add a new signed,
DCO-bearing descendant through this local integration path and rerun exact-main policy; it must not rewrite
or relabel the historical merge.

PR #186 was later published as merge `a7a64bcc86e9fa625547ed00a0be1c1e6dde3d73`; its missing DCO
trailer is another preserved historical failure. A signed descendant recovery is required before
the exact-main policy can be green again.

That historical object has tree `579310d1a7d89418eaafb068a1c1369be5088fc4` and parents
`a4e1a9de985a4c9f22628c6d604a6e62f4f173e3` and
`6cf8384bc6cf05ada62abdc102c9f4994191afed`. Push workflow run
`34347816992` failed in `Policy, coverage, and supply chain`; later steps did not establish a green
quality result for that exact main tip. These identities are evidence, not targets for replacement.

After pushing, verify the remote merge object, exact tree and parents, run every exact-main hosted
workflow plus local policy gates, and reconcile the coordination state. Never delete or rewrite a
published historical merge to make it satisfy current policy.
