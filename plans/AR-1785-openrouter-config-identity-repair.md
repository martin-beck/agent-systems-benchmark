# AR-1785: Repair OpenRouter configuration identity for live benchmark qualification

## Objective

Repair the mismatch exposed by AR-1715: a secure configured OpenRouter setup can
make direct provider transport succeed, while `provider-plan --use-config` and
`benchmark-live` reject the resulting selection as absent or identity-incompatible.
The repair must preserve digest binding, explicit live mode, and no-fallback
semantics.

## Required path

1. Read the complete ASB development, architecture, quality, and provider-launch
   documentation and inspect AR-1715 evidence.
2. Reproduce the mismatch in a disposable clean product-bound worktree using a
   mode-0600 runtime key reference; never persist the key, prompt, response, or
   private path.
3. Trace setup/config persistence, provider-plan `--use-config`, selection
   serialization, and benchmark-live identity validation. Define one stable
   canonical identity and reject stale/ambiguous configurations explicitly.
4. Implement the smallest contract repair with positive and negative tests for
   selected and all-agent human/JSON benchmark-live paths and no-fallback errors.
5. Run focused and full local gates, independent exact-head review, hosted CI,
   signed protected merge, exact-main post-merge verification, then rerun AR-1715.

## Exit conditions

The AR is not complete until AR-1715 selected/all CLI qualification passes in
human and versioned JSON modes, typed negative cases are recorded, all required
CI is terminal green, and a redacted post-merge receipt is committed in state.
