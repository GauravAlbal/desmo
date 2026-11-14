# Brownfield

Move **one** lifecycle boundary first. Delete the host branch when the
kernel owns it. Do not keep both.

## Before

The host mutates status and decides retry/terminal:

```python
def reject(job):
    job.attempts += 1
    job.status = "exhausted" if job.attempts >= job.max_attempts else "repairable"
```

## After — one command

```python
job = machines.show(job_id)
job = job.submit(artifact_digest=artifact, baseline_digest=baseline)
```

Leave `reject` in the host until the next cut. When it moves, remove the
Python arithmetic.

A host written in Go is the Go form of this: the host already observes/records; it
must consume Desmo through the generic client plus a thin
`Advance()` wrapper, not a second protocol stack. Pin a release, not a
source SHA.
