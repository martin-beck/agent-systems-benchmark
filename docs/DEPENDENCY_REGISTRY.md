# Command dependency registry

`asb preflight` is the bounded, side-effect-free readiness view for command
dependencies. It reports each dependency as an installable ASB tool, a project
or workload dependency, or an honest host capability. Missing installable IDs
include the exact `asb tool install` remediation; Linux, cgroup v2, user
systemd delegation, Bubblewrap, BTF, and PMU conditions are never represented
as downloadable tools.

The typed registry is generated from the checked-in command and acquisition
catalog declarations. A new executable or workload must add its catalog entry,
dependency record, preflight behavior, selection metadata, and a bounded
development-run test in the same change.

Provider APIs and procfs/cgroup collection are not executable tool
dependencies. Provider access remains an explicit runtime authorization, while
procfs/cgroup collection is provided by the ASB metrics boundary.
