STATUS: prepared, not qualified; no real P run exists

# Prepared P route

This directory is a registration template, a mapping example, and parser
fixtures. It is not a qualified P adapter. No P checker has been executed, and
this package does not download one.

`adapter.py describe` looks up `p --version` on `PATH`. It does not spawn `p`
or `dotnet`. When `p` is absent the document's `execution_status` is
`tool_unavailable` and the process exits 1. `run` writes that same status and
does not start a checker.

`manifest.template.json` names engine `P` and version constraint `p --version`.
Pin `invocation.argv` and `executable_sha256` before any host run. The
placeholder digest is not a pin.

`mapping.example.md` maps one Desmo named command to a P event and states the
queue, send, and monitor semantics. They are not the same interleaving.

`fixtures/` holds P checker text for the prepared parser (`adapter.py parse
--fixture <file>`). A bounded "no bug found" line is `no_violation_found` with
coverage not complete. It is never `holds_in_scope`. A "Bug found" line is a
model `violation`. A monitor that remained hot on a finite trace is
`inconclusive`. These fixtures do not qualify the route.
