# AR-1834 plan: AR-1775 protected-main local identity recovery

1. Preserve merge `356a669e0c6ebbb6cd3675a119cba16fea486283`, base
   `74b391348fc5c1361034a14742f6afef747d72db`, reviewed topic `67ddb52211144bc79a3d8dca36ba9030b3798085`, and exact failure evidence.
2. Reproduce the three failures locally. Confirm the existing exact identity
   contract rejects only the GitHub-generated committer and do not modify it.
3. From a clean isolated product worktree, create the smallest signed-DCO
   metadata-only recovery topic from current protected main. Preserve the exact
   product tree and historical AR-1775 evidence.
4. Obtain independent review, run focused and full applicable product gates,
   verify privacy/signature/DCO/topology/tree identity, and await exact-head CI.
5. Publish only with `tools/integration/merge_pr.py`; verify the local
   integration committer, all exact-main post-merge workflows, and durable state
   before acceptance. No web merge, force update, policy exception, or unrelated
   AR mutation is permitted.
