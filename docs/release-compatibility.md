# Release compatibility

| Surface | Compatibility |
|---|---|
| Protocol major | Must match exactly. Unknown major → `IK_PROTOCOL_VERSION_UNKNOWN`. |
| Runtime version | Must match the speaking `ik`. |
| Machine identity | `(machine_id, version, spec_hash)` is frozen per aggregate. |
| Persisted store | Forward-only. Older `ik` must not open a newer schema. |
| Python / Go clients | Protocol and runtime must match; bump clients and Python worker with the kernel. |

A protocol-major bump is a SemVer major and a coordinated consumer
upgrade. Release notes must name any protocol, schema, or client break.

## Unreleased

- Guard form `text_in(VALUE, SET, MIN, MAX)` is additive syntax on schema 1
  and schema 2. Schema version is unchanged. Machines that do not use the
  form keep their identity. An older binary that does not know `text_in`
  reports a syntax error (`unknown predicate` or an unexpected-token form)
  and does not load the machine.
- Diagnostic code `IK_STORE_NOT_FOUND` is additive in the protocol schema
  (protocol stays `1.0`; no new envelope field). The code emitted when the
  store file is absent changed from `IK_STORE_CORRUPT` to `IK_STORE_NOT_FOUND`.
  `rule=store.missing` is unchanged. Consumers that matched only
  `IK_STORE_CORRUPT` must treat `IK_STORE_NOT_FOUND` as absence, not damaged
  data. `IK_STORE_CORRUPT` remains the code for a present but invalid store.
  See [error taxonomy](design/error-taxonomy.md).
- Persisted store schema `4` (additive `operations` table) with an in-place
  forward migration from `3` on first open; older binaries refuse it, so back up
  first. Protocol stays `1.0`: `operation_id` and the `lookup_operation` op are
  additive and optional. Released `0.4.1` remains schema `3`.
- Protocol stays `1.0` with one more additive op, `create_with_command`. It is
  optional: old clients keep using `create`. A binary that does not know the op
  rejects the envelope; it does not apply a partial create. Store schema is
  unchanged. `lookup_operation` of that op sends the command and no capability.

## 0.4.1

- Additive authoring schema `2`: bounded compound/parallel regions lower into the
  existing flat `Definition`. Schema-1 identities are preserved against the
  accepted `v0.2.1` artifact, not regenerated candidate expectations.
- Runtime, Rust package, Python package/worker, and Go adapter expectation are
  `0.4.1` and must match exactly. Wire protocol stays `1.0`; persisted store
  schema stays `3`. This is an additive `0.3.0` → `0.4.1` authoring upgrade,
  not a protocol or store migration.
- The `CORE_LAWS` freeze remains version `0.2.1`: this release changes authoring
  and evidence, not runtime authority or those laws.
- Historical persisted schema `2` remains refused. It is not authoring schema `2`.
- Clients retain names/arguments-only authority. No chart or projected path is
  an executable lifecycle authority.
- Observed integrated-source checks are source evidence, recorded separately
  in [region lowering](region-lowering.md). Earlier old-base green runs do not
  prove the integrated source; source checks are not an artifact pass.
- Before publication, the release owner MUST qualify the actual binary tarball
  and sdist-derived wheel from the exact committed source. Offline-installed
  Python and the real Go client MUST exercise the same extracted, checksummed
  binary, including region session and transaction behavior.
- The release owner MUST obtain the source acceptance receipt and record the exact
  source SHA, artifact digests, and receipt in the GitHub Release.
  `LATEST_RELEASE` MUST be updated separately only after qualified publication.
  These are release conditions, not claims of acceptance or publication.
- Existing consumer pins remain unchanged; consumer upgrades
  require separate artifact qualification.

## 0.3.0 (historical)

- Persistent sessions keep the machine/reducer and SQLite store open across
  canonical JSON request lines; one-shot transport remains available.
- Ordered command transactions are all-or-nothing under one capability, and
  only the final step may create an effect intent.
- Reducer workers use JSON-RPC 2.0, remain long-lived and stateless by contract,
  recycle after 5000 calls, and are replaced after failure. Optional
  `IK_REDUCER_PARANOID=1` compares calls with fresh workers.
- These contracts remain in `0.4.1`; strict runtime-version matching still
  requires the binary, client, and worker to move together.
