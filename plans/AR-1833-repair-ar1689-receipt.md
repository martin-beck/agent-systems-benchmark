# AR-1833 repair plan

1. Read the AR-1689 evidence and development/state documentation.
2. Rewrite only the malformed receipt bytes, preserving its reviewed JSON content.
3. Recompute the receipt digest, record the repair through handoffctl, and obtain independent review.
4. Reconcile generated state, verify privacy and exact heads, then release this repair AR.

No product code or hosted release is changed by this repair.
