# Signed merge integrity

ASB integrates a reviewed pull request by constructing a local two-parent merge whose tree is the
approved pull-request tree. GitHub web merge, squash, rebase, and auto-merge are not authorized
integration paths because they cannot create a commit signed by an ASB allowed signer.

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
author, committer, and DCO identity. It uses `git commit-tree -S` to create a signed, DCO-trailered
no-fast-forward merge with parents exactly `BASE_OID HEAD_OID` and tree exactly `TREE_OID`.
Publication reads the target and pull-request refs together immediately before and after the push
and uses an exact `--force-with-lease`. A transport error is not proof of failure: if the post-error
read shows the exact merge, it records acceptance; unchanged or divergent state fails closed and
requires a new invocation after reconciliation. Diagnostics are bounded and generic and never
copy Git, transport, user, machine, path, or credential text.

Repository administrators disable every GitHub web merge mode after reviewing the change:

```sh
python3 tools/integration/repository_settings.py --apply
python3 tools/integration/repository_settings.py
```

The second command is a fail-closed audit. The setting change is external and must be recorded with
its exact response; it does not retroactively repair an old merge. Historical merge
`b6d04a8305ce6d49cc327e4e6d2d6fa42a88050b` remains unsigned/non-DCO. A signed descendant can
restore a green current-main evidence range, but must not be described as changing that history.

That historical object has tree `579310d1a7d89418eaafb068a1c1369be5088fc4` and parents
`a4e1a9de985a4c9f22628c6d604a6e62f4f173e3` and
`6cf8384bc6cf05ada62abdc102c9f4994191afed`. Push workflow run
`34347816992` failed in `Policy, coverage, and supply chain`; later steps did not establish a green
quality result for that exact main tip. These identities are evidence, not targets for replacement.

After pushing, verify the remote merge object, exact tree and parents, run every exact-main hosted
workflow plus local policy gates, and reconcile the coordination state. Never delete or rewrite a
published historical merge to make it satisfy current policy.
