# X08 independent extension exercise

One fresh agent context (GPT-5.6 Luna builder, shell available) received only the shipped starter adapter, the
analysis-adapter documentation, and an `ik` binary, with the task in `TASK.md`: change the question to
`unreachable_node_count`. Duration 5 min 13 s.

Observed by the orchestrator after the run, not taken from the agent's report:
* `ik analysis conformance <manifest> --fixtures <dir>` on the agent's `work/` exited with `passed: true`,
  `failure_count: 0` over all twelve fixture cases.
* The manifest `executable_sha256` equals the SHA-256 of the delivered `adapter.py`.
* The adapter computes reachability from `graph.initial` over committed edges (`adapter.py` `reachable_ids`).

What this shows: a developer-shaped agent can extend the starter to a new question without editing kernel code and
reach host-contract conformance. What it does not show: that the new checker is correct (conformance does not test
that), human usability, or that a second model would do the same. One attempt, one model family.

Agent-reported documentation observations: the starter README and `mapping.example.json` describe the old
question and had to be rewritten (expected for a changed question); `docs/analysis-adapters.md` still carries the
starter-example subsection for the old question (read-only for the exercise, so not changed).

`adapter-work.tar.gz` holds the delivered `work/` directory with local paths replaced.
