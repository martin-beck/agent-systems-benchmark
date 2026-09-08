---- MODULE Recovery ----
\* Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
\* SPDX-License-Identifier: MIT
EXTENDS Integers, Naturals, Sequences, FiniteSets
CONSTANTS Attempts, Sessions, MaxEpoch, MaxJournal, MaxCursor
Phases == {"planned", "running", "terminal", "uncertain"}
VARIABLES leaseOwner, leaseEpoch, attemptEpoch, phase, journal, projection,
          completionCount, cursors, priorCursors, lastAdvanced
vars == <<leaseOwner, leaseEpoch, attemptEpoch, phase, journal, projection,
          completionCount, cursors, priorCursors, lastAdvanced>>
Init ==
  /\ leaseOwner = "none"
  /\ leaseEpoch = 0
  /\ attemptEpoch = [a \in Attempts |-> 0]
  /\ phase = [a \in Attempts |-> "planned"]
  /\ journal = <<>>
  /\ projection = 0
  /\ completionCount = [a \in Attempts |-> 0]
  /\ cursors = [s \in Sessions |-> 0]
  /\ priorCursors = cursors
  /\ lastAdvanced = "none"
NoReplayChange ==
  /\ UNCHANGED cursors
  /\ priorCursors' = cursors
  /\ lastAdvanced' = "none"
Acquire(a) ==
  /\ leaseOwner = "none"
  /\ leaseEpoch < MaxEpoch
  /\ phase[a] = "planned"
  /\ leaseOwner' = a
  /\ leaseEpoch' = leaseEpoch + 1
  /\ attemptEpoch' = [attemptEpoch EXCEPT ![a] = leaseEpoch + 1]
  /\ UNCHANGED phase
  /\ UNCHANGED <<journal, projection, completionCount>>
  /\ NoReplayChange
Start(a, e) ==
  /\ leaseOwner = a /\ e = leaseEpoch /\ attemptEpoch[a] = e
  /\ phase[a] = "planned" /\ Len(journal) < MaxJournal
  /\ phase' = [phase EXCEPT ![a] = "running"]
  /\ journal' = Append(journal, <<a, e, "started">>)
  /\ UNCHANGED <<leaseOwner, leaseEpoch, attemptEpoch, projection, completionCount>>
  /\ NoReplayChange
Complete(a, e) ==
  /\ leaseOwner = a /\ e = leaseEpoch /\ attemptEpoch[a] = e
  /\ phase[a] = "running" /\ completionCount[a] = 0
  /\ Len(journal) < MaxJournal
  /\ phase' = [phase EXCEPT ![a] = "terminal"]
  /\ completionCount' = [completionCount EXCEPT ![a] = 1]
  /\ journal' = Append(journal, <<a, e, "completed">>)
  /\ leaseOwner' = "none"
  /\ UNCHANGED <<leaseEpoch, attemptEpoch, projection>>
  /\ NoReplayChange
Crash(a) ==
  /\ leaseOwner = a /\ phase[a] = "running"
  /\ phase' = [phase EXCEPT ![a] = "uncertain"]
  /\ leaseOwner' = "none"
  /\ UNCHANGED <<leaseEpoch, attemptEpoch, journal, projection, completionCount>>
  /\ NoReplayChange
ReconcileUncertain(a) ==
  /\ phase[a] = "uncertain" /\ leaseOwner = "none"
  /\ completionCount[a] = 0 /\ Len(journal) < MaxJournal
  /\ journal' = Append(journal, <<a, attemptEpoch[a], "uncertain">>)
  /\ completionCount' = [completionCount EXCEPT ![a] = 1]
  /\ UNCHANGED <<leaseOwner, leaseEpoch, attemptEpoch, phase, projection>>
  /\ NoReplayChange
ProjectNext ==
  /\ projection < Len(journal)
  /\ projection' = projection + 1
  /\ UNCHANGED <<leaseOwner, leaseEpoch, attemptEpoch, phase, journal, completionCount>>
  /\ NoReplayChange
AdvanceReplay(s) ==
  /\ cursors[s] < MaxCursor
  /\ priorCursors' = cursors
  /\ cursors' = [cursors EXCEPT ![s] = @ + 1]
  /\ lastAdvanced' = s
  /\ UNCHANGED <<leaseOwner, leaseEpoch, attemptEpoch, phase, journal, projection, completionCount>>
Next ==
  \/ \E a \in Attempts : Acquire(a)
  \/ \E a \in Attempts, e \in 0..MaxEpoch : Start(a, e)
  \/ \E a \in Attempts, e \in 0..MaxEpoch : Complete(a, e)
  \/ \E a \in Attempts : Crash(a)
  \/ \E a \in Attempts : ReconcileUncertain(a)
  \/ ProjectNext
  \/ \E s \in Sessions : AdvanceReplay(s)
TypeOK ==
  /\ leaseOwner \in Attempts \cup {"none"}
  /\ leaseEpoch \in 0..MaxEpoch
  /\ attemptEpoch \in [Attempts -> 0..MaxEpoch]
  /\ phase \in [Attempts -> Phases]
  /\ projection \in 0..Len(journal)
  /\ completionCount \in [Attempts -> 0..1]
  /\ cursors \in [Sessions -> 0..MaxCursor]
UniqueLease == leaseOwner = "none" \/ Cardinality({a \in Attempts : leaseOwner = a}) = 1
StaleEventsFenced == \A a \in Attempts : phase[a] = "running" => attemptEpoch[a] = leaseEpoch
JournalProjectionMonotonic == projection <= Len(journal)
NoDuplicateCompletion == \A a \in Attempts : completionCount[a] <= 1
UncertainIsTerminal == \A a \in Attempts : phase[a] = "uncertain" => leaseOwner # a
ReplaySessionIsolation ==
  lastAdvanced = "none" \/
    /\ cursors[lastAdvanced] = priorCursors[lastAdvanced] + 1
    /\ \A s \in Sessions \ {lastAdvanced} : cursors[s] = priorCursors[s]
Spec == Init /\ [][Next]_vars
====
