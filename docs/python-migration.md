# Brownfield: move one lifecycle boundary into the kernel

Python keeps orchestration and IO. The Rust kernel owns transition
authority. Do not port the whole application. Move one boundary first.

## BEFORE

Python owns status mutation and retry/terminal decisions:

```python
def submit(job, artifact, baseline):
    if job.status != "running":
        raise RuntimeError("not running")
    job.status = "submitted"
    job.artifact = artifact
    job.baseline = baseline
    db.save(job)

def reject(job):
    job.attempts += 1
    if job.attempts >= job.max_attempts:
        job.status = "exhausted"
    else:
        job.status = "repairable"
    db.save(job)
```

Those branches are the semantic boundary. They drift.

## AFTER — one boundary first

Leave retry/exhaustion in Python. Move **submit** only.

```python
from invariant_kernel import Kernel, IllegalStateError

kernel = Kernel(store="jobs.db")
machines = kernel.machine("machines/verification_attempt.ik.toml")

def submit(job_id, artifact, baseline):
    job = machines.show(job_id)
    try:
        job = job.submit(artifact_digest=artifact, baseline_digest=baseline)
    except IllegalStateError:
        raise RuntimeError(f"submit refused in state {job.state}")
    return job
```

`reject` / `retry` stay in the old Python until the next cut.

```python
def reject(job):
    # still application-owned, not yet a kernel command
    job.attempts += 1
    job.status = "exhausted" if job.attempts >= job.max_attempts else "repairable"
    db.save(job)
```

The incremental rule: one command family, one store, one machine file.
When `reject` moves, delete the Python attempt-count arithmetic. Do not
keep both.

## What Python still owns

- fetching artifacts
- calling verifiers
- scheduling
- HTTP / CLI / UI
- mapping `StaleCapabilityError` to "reload the job"

## What Python must not own after a cut

- legal destination states
- guard arithmetic
- capability generation / nonces
- effect identity
- "retry vs exhausted"
