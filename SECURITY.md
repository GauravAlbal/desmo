# Security policy

Desmo is a cooperative lifecycle authority for a single worker or agent harness over a local
SQLite store. It is **not** a sandbox: a process that can open the store file, or run arbitrary
code in the same user account, is outside the protection model (see `docs/alpha-contract.md`).

Report suspected vulnerabilities privately through GitHub's private vulnerability reporting for
this repository (Security tab -> Report a vulnerability). Do not open a public issue for an
unpatched vulnerability.

Support during the alpha is best effort; only the latest released version is supported.

## Workstation acceptance is cooperative

Local coding agents on a developer workstation normally share an account with the engineering tools that accept their work. Role flags and hidden operator commands in an agent plugin are a convenience, not an authenticated security boundary: the same account can start an operator-mode client, run the ordinary command-line tools, or write the local receipt files. Treat workstation acceptance as cooperative. Do not claim that an agent cannot certify its own work unless the acceptance gate and its records run as a separate identity (for example a CI job or a locked-down service) that the agent cannot write. This is separate from the application lifecycle rules enforced by the Desmo runtime.
