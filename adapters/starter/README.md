# Starter analysis adapter

A small checker for one property family on an exported `ik.graph.v1` graph.
Copy it, change the property function, and register the manifest. No kernel
edit is required.

Passing `ik analysis conformance` proves the adapter meets the host contract.
It does not prove the checker, the mapping, or the engine is correct.

## Four steps

1. Copy this directory to the place you will register the adapter.
2. Edit `manifest.template.json`. Set `invocation.argv[0]` to the absolute path
   of your `adapter.py`. Set `invocation.executable_sha256` to the lowercase
   SHA-256 of that file. A relative path, a symlink, or a digest mismatch is
   refused before the process starts. Replace `adapter.id` with your adapter's
   name. The template digest is a placeholder, not a pin.
3. Implement the property function. `dead_end_count` in `adapter.py` is the
   working example: non-terminal nodes that are not on the export frontier and
   have no committed outgoing edge. Keep `finish_run` and the
   `controls.conformance_fault` hook. Replace `QUESTION` and `classify` if you
   answer a different family, and say `unsupported` for everything else.
4. Run conformance:

```text
ik analysis conformance /absolute/path/to/manifest.json --fixtures /absolute/path/to/fixtures
```

Exit 0 means every fixture matched the host outcome. Exit 1 means a fixture
failed. Exit 2 means the manifest or the fixture directory was refused before
the battery ran.

## What the battery checks

Each case is `fixtures/cases/*.json`. The host builds a request, spawns the
pinned argv through the same path as `ik analysis run`, and compares the host
report to `expect`.

| Case | Expected host outcome |
|---|---|
| `correct_result` | Accepted `holds_in_scope` on a closed graph with no dead end. |
| `real_violation` | Accepted `violation` when a dead end is in the export. |
| `incomplete_search` | Accepted `inconclusive` with `bounded_paths` and coverage not complete. Not unreachable. |
| `unsupported_property` | Accepted `execution_status` `unsupported`. |
| `preserves_native_logs` | The checker log is inside the adapter out directory, and stdout is kept on the host report. |
| `honors_timeout` | The host kills the run and does not accept a result. |
| `honors_cancellation` | The host cancels the run and does not accept a result. |
| `wrong_request_id` | Rejected `request_id_mismatch`. |
| `stale_subject_hash` | Rejected `stale_subject_hash`. |
| `malformed_output` | Rejected `malformed_json`. |
| `path_traversal` | Rejected `path_traversal`. |
| `exit_without_result` | Rejected `exit_without_result`. |

The adapter cannot naturally emit a wrong request id, a stale subject hash,
malformed JSON, a traversal artifact, or exit 0 with no result while also
passing the positive cases. For those five fixtures the host runs the adapter,
then injects the fault into the written `result.json` and judges that file
with the same result checker `ik analysis run` uses. The fixture field is
`fault`. This teaches the rejection contract. It is not a claim that your
checker produced the bad file.

`timeout` and `cancel` are different. The host sets
`controls.conformance_fault` to that name and enforces the limit. The starter
writes its result, then waits until killed, so a killed run still had a
result file on disk. The host must reject it. Keep that hook.

Shipped graphs under `fixtures/graphs/` are `ik graph export` output for the
profile machines. Replace them with a fresh export when you change the
question. A depth cut is incomplete. A frontier node is not a dead end.

`mapping.example.json` states how a graph node and a committed edge relate to
`dead_end_count`. A source label is not a node id. A refusal is not a
continuation.
