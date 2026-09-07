# CSB execution provenance

ASB's optional CSB boundary is based on the public
[`martin-beck/CSB`](https://github.com/martin-beck/CSB) repository. This document records source
and narrowly qualified execution identity. It does not claim that the complete upstream Python
environment is reproducible.

## Pinned source graph

The inspected release commit is `d577c5249501b29e33a87524a677a101477d5579` (`[CSB] Release
v0.4.0. (#265)`) with Git tree `97d08b39026f7c7d3e6f748b26a20e819a92184f`. The repository did
not publish a `v0.4.0` tag at inspection time, so the full commit and tree, rather than the release
message, are the identity boundary.

| Component | Public repository | Commit | Tree | Intended boundary |
| --- | --- | --- | --- | --- |
| CSB | `martin-beck/CSB` | `d577c5249501b29e33a87524a677a101477d5579` | `97d08b39026f7c7d3e6f748b26a20e819a92184f` | `bm-runner` inspection and a future bounded adapter |
| benchkit | `open-s4c/benchkit` | `cebe6d1548170f63d65bd23b4f83e47b52131a9a` | `d82f2d7cbd1691621d3f5aae220a08810349e8d9` | Transitive execution dependency requiring further reduction |
| FlameGraph | `brendangregg/FlameGraph` | `41fee1f99f9276008b7cd112fca19dc3ea84ac32` | `85d36fd0ea17a61bec8edb1bea8d76bfaa3bb280` | Excluded from the initial adapter |
| inferno | `open-s4c/inferno` | `497418432f46b290c20ba14b35eedcfb80f149c2` | `3e271fe67becae0d9a9292016896511cd4d2407e` | Excluded from the initial adapter |
| inferno FlameGraph | `jonhoo/FlameGraph` | `57207afbe114e3d50c6c7aef93d3206ef76cf6e2` | `32a15a836a23bb8c27eb760abc201f3f33069464` | Excluded nested submodule |
| syzkaller fork | `open-s4c/syzkaller` | `44df4dbcbd159bf6ad0ee97ee7f9fc021d0e40f9` | `9586b54e9fbafef3f8c5458c60ac2fc8010474b7` | Excluded generator dependency |

The top-level repository is MIT licensed. Benchkit is MIT, inferno declares CDDL-1.0, and the
syzkaller fork contains Apache-2.0 and additional embedded notices. The FlameGraph trees have no
single root license file and contain per-file CDDL, GPL, Apache, and other notices. Consequently,
ASB must not redistribute or execute the excluded FlameGraph, inferno, generator, or syzkaller
trees under a blanket CSB MIT assertion.

## Build and execution limits

The top-level `requirements.txt` pins only `pytest==9.0.3` and `pytest-mock==3.15.1`; pandas,
tabulate, seaborn, matplotlib, dominate, docker, jsonpath-ng, and psutil float. Benchkit constrains
but does not lock `docopt<=0.6.2` and `rich<=14.2.0`. Therefore an offline byte-reproducible
`bm-runner` environment has not been established. The initial conformance fixture must be
credential-free, offline, and independently pinned rather than importing this floating environment.

CSB's current `bm-runner/main.py` is an argparse program that writes files and human-oriented
output; it is not a version-negotiated structured extension protocol. ASB will put a separate
bounded JSON-RPC stdio process in front of any accepted CSB execution. ASB owns deadlines,
process-tree and cgroup containment, durable pre-effect intent, artifact validation, cancellation,
and cleanup. CSB generators, arbitrary plugins, network access, privileged monitors, and nested
schedulers remain unsupported.

The first eligible native fixture is the pinned
`bm-external/bwrap/bm-bwrap.py` file at the source identity above, SHA-256
`e1228516fe0648db35cfb7b6669b09f25dc32f62691c8fc8cca4ee1e935eee88`. Its `baseline` scenario
uses only the Python standard library and can invoke `/usr/bin/true` once without credentials or
network access. A direct source-checkout probe produced `success_count=1`; elapsed values are
deliberately not retained as benchmark evidence. Native ASB tests separately execute the exact
script through the reviewed `asb-runtime` sandbox and durable store three times, and exercise
explicit cancellation and deadline escalation with zero residual CPU leases. The qualified host
interpreter is `/usr/bin/python3.12` 3.12.3 with SHA-256
`1643dacd9feaedc58f3cc581e4d22577dfe25c09b10282936186ccf0f2e61118`; other interpreter bytes are
not implied. The script is executed through its verified open file descriptor, closing the
pathname replacement window. Workspace and state roots reject symlink ancestry but require a
trusted private parent against hostile same-UID ancestor replacement.

No Linux aarch64, non-glibc, Windows, or macOS support follows from this source inspection. The only
eligible first claim is a native credential-free offline Linux x86_64 fixture after its exact
executable bytes, isolation behavior, and repeated cleanup are independently verified.
