# Tranche 2 dispatch instructions

Dispatch only after the validation gates in `contract.md` pass and an independent reviewer has cleared the staging, control and parity findings. Frozen candidate: the released v0.5.0 npm tarballs for arm B (staged by `stage.py`, checksummed against the release `SHA256SUMS`). Every builder and maintainer is a fresh agent with a fresh context and the same model configuration; record the configuration and timestamps.

## Commands (all exist in this directory)

```sh
# Stage a builder workspace outside the repository (default root /tmp/ik14-runs, override with IK14_WORK)
python3 validation/ik14/greenfield-2/stage.py <G4|G5|G6> <A|B> <r1|r2> initial
# After the builder has finished and the initial project is frozen (copy it aside for the record), stage maintenance
python3 validation/ik14/greenfield-2/stage.py <G4|G5|G6> <A|B> <r1|r2> change
# Score a staged project with the protected evaluator (writes <root>/<project>/<arm>/<rep>/results/<stage>.json)
sh validation/ik14/greenfield-2/evaluate.sh <G4|G5|G6> <A|B> <r1|r2> <initial|change>
```

`stage.py` prints the project directory. The workspace for one run is `<root>/<project>/<arm>/<rep>/<stage>/` containing `PROMPT.md` and `project/`. A builder sees only `project/` (BRIEF.md, CHANGE.md in the change stage, `stub/CONTRACT.md`, `stub/server.mjs`, and `KIT/` for arm B only). The evaluator, sidecars, controls and references are never staged, and `stage.py` refuses to finish if its leak scan finds evaluator or control material in the staged tree.

## Prompts

`prompts/common.md` (initial) or `prompts/maintainer.md` (change), plus `prompts/armA.md` or `prompts/armB.md`; `stage.py` concatenates them into `PROMPT.md`. Both arms are told the same competence baseline (atomic commit of state, history and effect intent, currentness check, idempotent operations, re-derivable history, correct recovery at the named crash boundaries, no mutation on refusal). The maintainer prompt is an implementation assignment, not a review.

## Dispatch procedure

1. Run `stage.py ... initial`. Start a fresh builder agent whose working directory is `project/`, giving it `PROMPT.md` verbatim as the task. Do not add hints.
2. When it finishes, copy `project/` to `<run>/initial-frozen/`, run `evaluate.sh ... initial`. Keep the builder's final reply verbatim.
3. Run `stage.py ... change` (it copies the frozen initial project), start a fresh maintainer, then `evaluate.sh ... change`. A maintainer receives the project as the builder left it even if the initial stage failed (rescue-and-change); the initial result is not rewritten.
4. Append every attempt (including invalid ones) to `results.jsonl` in this directory with the evaluator JSON, the agent's final reply, wall time and the configuration. Report all runs in the decision; never drop an attempt.
5. Evaluators allocate their own ephemeral ports, so runs may execute concurrently on one machine. Run at most 6 builders at once to keep the machine from timing out the evaluator's fault schedules; do not evaluate while another evaluation of the same project is starting up if timeouts appear, re-run it and record both.

## Counterbalancing

Arm order within each project and replica: G4-r1 A then B; G4-r2 B then A; G5-r1 B then A; G5-r2 A then B; G6-r1 A then B; G6-r2 B then A. "Then" means the first-listed arm's builder is dispatched first; maintenance order reverses each pair's initial order. Project order is randomized independently.

## Invalidity

Invalidate a run only for unequal inputs or budget, evaluator leakage, a broken common fixture, or protected-material access, and record why. A weak result is a result.
