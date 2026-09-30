# Protected-main merge procedure

Changes that land on `main` must preserve the reviewed topic and the
repository's authenticated merge boundary. Before merging a pull request:

1. Rebase or synchronize the topic locally against the current `main`.
2. Keep every topic commit DCO-bearing and SSH-signed with an allowed key.
3. Run the exact-head checks on the synchronized topic.
4. Use a two-parent, non-squash merge whose first parent is the protected
   base and whose second parent is the reviewed topic tip.
5. After the push, verify the exact `main` SHA with
   `tools/quality/repository_policy.py --mode protected-main` and wait for all
   required hosted checks to reach a terminal success state.

Do not use a hosting-service rebase or squash operation that regenerates an
unsigned topic commit. If a service operation does that, preserve the
immutable history and create a signed forward repair merge instead of waiving
or weakening the policy.
