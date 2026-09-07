module RecoveryMutants
abstract sig Phase {}
one sig Planned, Running, Terminal, Uncertain extends Phase {}
sig Attempt { epoch: one Int, phase: one Phase }
sig Lease { owner: one Attempt, epoch: one Int }
sig Event { attempt: one Attempt, sequence: one Int, completion: one Int }
one sig Projection { applied: set Event }
sig ReplaySession { cursor: one Int }
assert UniqueLease { lone Lease }
assert StaleLeaseFenced { all l: Lease | l.epoch = l.owner.epoch }
assert JournalProjectionPrefix {
  all e: Projection.applied |
    all earlier: Event | earlier.sequence < e.sequence implies earlier in Projection.applied
}
assert NoDuplicateCompletion {
  all a: Attempt | lone {e: Event | e.attempt = a and e.completion = 1}
}
assert UncertainHasNoLease {
  all a: Attempt | a.phase = Uncertain implies no l: Lease | l.owner = a
}
assert ReplayCursorsBounded {
  all s: ReplaySession | s.cursor >= 0 and s.cursor <= 2
}
check UniqueLease for exactly 2 Attempt, exactly 2 Lease, exactly 2 Event, exactly 2 ReplaySession, 4 Int
check StaleLeaseFenced for exactly 2 Attempt, exactly 1 Lease, exactly 2 Event, exactly 2 ReplaySession, 4 Int
check JournalProjectionPrefix for exactly 2 Attempt, exactly 1 Lease, exactly 2 Event, exactly 2 ReplaySession, 4 Int
check NoDuplicateCompletion for exactly 2 Attempt, exactly 1 Lease, exactly 2 Event, exactly 2 ReplaySession, 4 Int
check UncertainHasNoLease for exactly 2 Attempt, exactly 1 Lease, exactly 2 Event, exactly 2 ReplaySession, 4 Int
check ReplayCursorsBounded for exactly 2 Attempt, exactly 1 Lease, exactly 2 Event, exactly 2 ReplaySession, 4 Int
