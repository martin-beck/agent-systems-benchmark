# AR-1060 protected-history repair

This record documents the one-time authorized replacement of the invalid AR-1060
protected-main merge boundary.

- Reviewed feature tree: `36f21d9bfca9d3afde9514348c89d2ab884ebefc`
- Repaired signed merge: `3f1daeb4980b5dd8fae132b97dc61b83d6a3124b`
- Pre-repair main is retained under the repository backup reference
  `refs/backup/ar1060-pre-repair-main`.

The replacement preserves the reviewed product tree and does not alter policy,
verification, or application behavior. The final protected integration merge is
required to be GitHub Web Flow signed and DCO-bearing.
