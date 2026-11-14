# Upgrading and pins

Consumers pin a **released artifact**, never `origin/main`.

The source SHA is provenance. The artifact is what you qualify.

## release-manifest.json (schema 3)

Shipped with every GitHub Release:

The example below is the historical `0.3.0` manifest shape, not a current-release
pointer or a qualification record for another version. Consult the qualified
GitHub Release and its separately updated `LATEST_RELEASE` manifest for actual
source and artifact identities.

```json
{
  "product": "Ikaros",
  "version": "0.3.0",
  "tag": "v0.3.0",
  "source_sha": "<immutable commit>",
  "protocol_major": 1,
  "runtime_version": "0.3.0",
  "minimum_supported_protocol": 1,
  "schema_version": 3,
  "python_client_version": "0.3.0",
  "go_client_source_revision": "<same commit>",
  "artifacts": [
    {
      "target": "aarch64-apple-darwin",
      "file": "ik-darwin-arm64.tar.gz",
      "sha256": "..."
    }
  ]
}
```

## Active consumer pin

Example of a separately qualified future consumer `RUNTIME_PIN`; do not
rewrite the sealed V1.0 view or treat this example as a pin-migration instruction:

```text
product=desmo
version=v0.3.0
source_revision=<sha>
artifact=ik-darwin-arm64.tar.gz
artifact_sha256=<sha256>
protocol_major=1
status=accepted
```

Historical `revision=` pins remain valid for sealed candidate files.
New claims should use the release identity above.

## Persisted schema 2

Runtime `0.2.0` intentionally refuses schema-2 stores. The release inventory
found no durable schema-2 authority requiring preservation, so no implicit
migration exists. Recreate disposable stores explicitly; qualify any future
export/reimport migration separately before using it on accepted state.

## Upgrade

```text
Desmo release
    ↓
consumer upgrade candidate
    ↓
parity / qualification against that artifact
    ↓
RUNTIME_PIN movement
```

Rollback is the previous pin. Do not rebuild `main`.

## Install a release binary

Download the matching `ik-*.tar.gz`, check `SHA256SUMS`, extract `ik`,
set `PATH` or `IK_BINARY`. Language packages do not ship the binary.

## 0.3.0 → 0.4.1 region-authoring upgrade

Observed integrated-source verification is source evidence, recorded in
[region lowering](region-lowering.md), not release-artifact qualification.
Before publication, the release owner must qualify the actual binary tarball,
sdist-derived wheel, offline-installed Python, and real Go client from the exact
committed source, including region session and transaction behavior, and obtain
the source acceptance receipt. The GitHub Release must record the exact source SHA,
artifact digests, and receipt. `LATEST_RELEASE` is updated separately only after
qualified publication; source checks alone do not satisfy these conditions.

After publication and consumer qualification, upgrade the binary and Python/Go
clients together. Runtime-version equality is strict: `ik`, the Python
client/worker, and the Go adapter expectation must all be `0.4.1`; protocol
compatibility alone does not permit mixed `0.3.0`/`0.4.1` versions.
Protocol stays `1.0`; persisted store schema stays `3`. Authoring schema `2`
is an additive region frontend, not the refused historical store schema.
The `0.3.0` persistent-worker, session, and all-or-nothing transaction APIs
remain available.

Existing schema-1 sources and aggregate identities do not need migration.
Schema-2 sources lower to the same flat runtime contract; `spec_hash` hashes that
lowered definition, not source text, region metadata, or charts. Reauthoring an
existing machine is not automatically identity-preserving: compare `ik identity`
before using it with an existing store.

Python `Kernel` and Go `Kernel` continue to submit command names and arguments,
never destination states. Their `check` methods invoke the source-aware CLI
checker. A reachable invariant-refused proposal makes a region check REFUSED
without committing that proposal. ACCEPT is bounded and corpus-relative.

Existing consumer pins remain unchanged.
Qualify a separate consumer upgrade against the immutable release before moving
any active pin. Rust consumers can keep schema-1 `Definition` APIs; authoring-aware
checks use `declarative::region::parse_source` and `check_source`, rather than
discarding region metadata and calling the legacy flat checker.

## Store schema 4 (unreleased)

Unreleased builds open a schema-3 store and migrate it in place to schema 4 on
first open (an empty `operations` table; no history is invented). Back up the
store file (with its `-wal`/`-shm`) first: schema-3 binaries refuse schema 4 and
there is no rollback. Released `0.4.1` is unchanged and stays schema `3`.
