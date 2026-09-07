---- MODULE RecoveryStaleMutation ----
EXTENDS Recovery
MutationOwner == CHOOSE a \in Attempts : TRUE
MutationInit ==
  /\ leaseOwner = MutationOwner
  /\ leaseEpoch = 1
  /\ attemptEpoch = [a \in Attempts |-> 0]
  /\ phase = [a \in Attempts |-> IF a = MutationOwner THEN "running" ELSE "planned"]
  /\ journal = <<>>
  /\ projection = 0
  /\ completionCount = [a \in Attempts |-> 0]
  /\ cursors = [s \in Sessions |-> 0]
  /\ priorCursors = cursors
  /\ lastAdvanced = "none"
MutationNext == UNCHANGED vars
MutationSpec == MutationInit /\ [][MutationNext]_vars
====
