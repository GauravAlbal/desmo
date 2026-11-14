# batch-001 frozen inputs

Candidate: Ikaros 0.5.0 host-target (macOS arm64) rebuilt from private commit adf071afc11bee405ee376a6426051d64bc55229
via the public-source export (270 files), stand-in commit b362013b01372a9ec26b7f35ac3a7e541666999d.

The previously recorded candidate files (stand-in 3ca83dd8...) were removed from a temporary directory and could not
be re-verified. This batch uses a rebuild from the same source revision. Digests below are authoritative for batch-001.
The two npm tarballs are byte-identical to the previously recorded ones; the release archive, wheel and sdist differ
(not byte-reproducible). Installed-artifact qualification of this rebuild passed (batch-001/qualify.log, qualify.json).

| File | sha256 |
|---|---|
| artifacts/ik-darwin-arm64.tar.gz | 904078a528f1b2d1a911ad2638dc802267fc4221a23182485be7ec5fc9cf1bd8 |
| artifacts/invariant_kernel-0.5.0-py3-none-any.whl | ab2a3ecc2203cf88cf39c9161bf40eaad71d8bad977f15c33b2f5b40d12302e3 |
| artifacts/invariant_kernel-0.5.0.tar.gz | e515378f2291165bd58db7624d78e25032afa2c14531353ffec4d39ca5b312fa |
| kit-B/npm/ikaros-runtime-0.5.0.tgz | ef37a1f52a1db8e5e1e8c8b97cf9ccfb0f77f215a1c761b0b6e054ad6515e710 |
| kit-B/npm/ikaros-darwin-arm64-0.5.0.tgz | df9acb510321f16d231de8ce9a1332663845cf60b7d5a6c88592716677a8571e |

Arm B kit: the two npm tarballs, the shipped README, docs/ and examples/ from that export. No other assistance.
Builder/maintainer agent: agent type `reviewer`, one fresh context per run; the underlying model and reasoning settings
are resolved by the runtime and are not exposed to the orchestrator (recorded as unknown). Wall-clock cap per
builder run: 45 minutes; per maintainer run: 30 minutes (declared before dispatch; token budgets not enforceable).
