---
"release": minor
---

# Check graph artifacts before consuming them

`lk verify graph --bundle <receipt> --sha256 <independent-digest>` checks a graph build against its exact manifest bytes and the current project declarations. It verifies file contents, modes, membership, unit placement, recipes, readiness dependencies and declared immutable toolchain pins without executing the supplied programs.

Use repeated `--unit` options with `--out <existing-parent>` to copy only selected units into a new private staging directory. The original manifest remains pinned; inputs are rechecked when consumed, and a failed check leaves no successful staging child. Source reproduction, approval, executable target compatibility and deployed runtime remain explicitly unchecked.

Build capture now uses the same bounded consumer before publishing its receipt. Both capture and staging request owner-only directory permissions at creation. Duplicate JSON keys, unexpected files, conflicting ownership, overstated verification evidence and aggregate metadata/artifact limits are rejected.
