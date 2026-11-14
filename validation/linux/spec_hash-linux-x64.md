# spec_hash parity: darwin-arm64 packed `ik` vs linux-x64 packed `ik`

Both binaries ran `ik identity` and `ik check --json --profile` on the two shipped examples. The compared linux-x64 binary is the release-profile ELF extracted from `ik-linux-x64.tar.gz` (sha256 `5396cb6c135e206144c0eb077ab533d3bdd7c8482d40b8d1da2b593d88170c6c`). The compared darwin-arm64 binary is the packed `ik` from the batch-017 kit.

| Example | Field | darwin-arm64 | linux-x64 | Match |
|---|---|---|---|---|
| `examples/shipped/loan_checkout.ik.toml` | `identity.spec_hash` | `6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8` | `6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8` | yes |
| `examples/shipped/loan_checkout.ik.toml` | `check` `report.spec_hash` | `6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8` | `6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8` | yes |
| `examples/shipped/loan_checkout.ik.toml` | `check` verdict | ACCEPT | ACCEPT | yes |
| `examples/shipped/tagged_item.ik.toml` | `identity.spec_hash` | `f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6` | `f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6` | yes |
| `examples/shipped/tagged_item.ik.toml` | `check` `report.spec_hash` | `f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6` | `f0bef35d059f09af32614c758cbfb5f66ac707bf92c03240dd96b729cb3758d6` | yes |
| `examples/shipped/tagged_item.ik.toml` | `check` verdict | ACCEPT | ACCEPT | yes |

Every shipped example's `spec_hash` is identical across the two packed binaries.
