# IK-06: claim -> enforcement -> evidence

- `claims.json`: each supported claim, where it is enforced, the tests that evidence it, and the
  deleted-wall mutants that must be killed. Every `LawMutant` variant must be listed exactly once.
- `qualify.py --check`: rows exist, are not `#[ignore]`d, enforcement text is present, every
  mutant is covered. `--self-test`: broken manifests are rejected. `--run`: executes all rows.
- `results.json`: the executed run (41 rows, macOS arm64, rustc in the file). `provenance.dirty` is
  true because it was produced on the working tree that this change commits; rerun
  `python3 validation/ik06/qualify.py --run` on a checkout to regenerate against clean bytes.
- `SELF_REVIEW.md`: author review. It is NOT the independent review the roadmap requires.

Honesty rules enforced by the code: a row passes only if the runner exited 0 and at least one test
ran with none skipped/ignored; `with_law_mutant` panics if the deleted wall was never reached, so a
kill that did not fire cannot pass; metadata alone never satisfies a row.
