# IK-13 cold run, performed by an agent (not a human)

Status: **executed by a separate agent; this is not the human cold run.** The agent was given only the
release kit (candidate artifacts, README, docs) in a scratch directory. Isolation was by instruction
(honor system), not by sandbox. The human cold run in `docs/cold-run-protocol.md` remains open.

## Outcome

All 7 protocol tasks were completed. No unsafe act was observed (no direct store edits, no capability
forging). The transcript showed no access to the author's working tree.

## Friction found, and disposition

| Finding | Disposition |
|---|---|
| npm tarballs absent from `SHA256SUMS` | fixed: `scripts/pack_npm.py` writes `SHA256SUMS` |
| No documented `ik --version` | fixed: README install block |
| Demo hid the diagnostic code | fixed: `ikaros` demo prints `err.code` |
| Schema 1 missing `[facts]` gives a misleading parse span | documented in `docs/machine-language.md` (span fix not done) |
| `show()` on another machine version hides recorded identity | documented in `docs/machine-evolution.md` |
| `--prior` message misleading when `--prior` was given | fixed in `src/declarative/release.rs` |
| Operation-identity ADR still said "not implemented" | fixed |
| No way to extend an existing aggregate without a migrator | open; recorded as a limit |

Fixes land after the run, so the candidate must be rebuilt and requalified; the agent did not see them.
