module Recovery
abstract sig Phase {}
one sig Planned, Running, Terminal, Uncertain extends Phase {}
sig Attempt { epoch: one Int, phase: one Phase }
lone sig Lease { owner: one Attempt, epoch: one Int }
sig Event { attempt: one Attempt, epoch: one Int, sequence: one Int, completion: one Int }
one sig Projection { applied: set Event }
sig ReplaySession { cursor: one Int }
fact FiniteRecovery {
  all a: Attempt | a.epoch >= 0 and a.epoch <= 2
  all e: Event | e.epoch >= 0 and e.epoch <= 2
  all e: Event | e.sequence >= 0 and e.sequence < 4
  all e: Event | e.completion = 0 or e.completion = 1
  all disj left, right: Event | left.sequence != right.sequence
  all e: Projection.applied |
    all earlier: Event | earlier.sequence < e.sequence implies earlier in Projection.applied
  all a: Attempt | lone {e: Event | e.attempt = a and e.completion = 1}
  all a: Attempt | a.phase = Uncertain implies no l: Lease | l.owner = a
  all l: Lease | l.epoch = l.owner.epoch
  all s: ReplaySession | s.cursor >= 0 and s.cursor <= 2
}
pred Witness {
  some Lease
  some Projection.applied
  some a: Attempt | a.phase = Uncertain
  #ReplaySession = 2
}
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
run Witness for exactly 2 Attempt, exactly 4 Event, exactly 2 ReplaySession, 4 Int
check UniqueLease for exactly 2 Attempt, exactly 4 Event, exactly 2 ReplaySession, 4 Int
check StaleLeaseFenced for exactly 2 Attempt, exactly 4 Event, exactly 2 ReplaySession, 4 Int
check JournalProjectionPrefix for exactly 2 Attempt, exactly 4 Event, exactly 2 ReplaySession, 4 Int
check NoDuplicateCompletion for exactly 2 Attempt, exactly 4 Event, exactly 2 ReplaySession, 4 Int
check UncertainHasNoLease for exactly 2 Attempt, exactly 4 Event, exactly 2 ReplaySession, 4 Int
check ReplayCursorsBounded for exactly 2 Attempt, exactly 4 Event, exactly 2 ReplaySession, 4 Int
