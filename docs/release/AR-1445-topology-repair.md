# AR-1445 protected-main topology repair

AR-1440's reviewed product change is already present on `main` at commit
`84b587ec`. Its squash merge has one parent, while the protected-main policy
requires a reviewed merge commit with two parents. This note records the
topology repair boundary: the repair must preserve the current product tree,
run the ordinary required checks, and use the protected non-squash merge path.

This note does not authorize rewriting protected history, bypassing required
checks, or changing the OpenRouter provider/model selection contract.
