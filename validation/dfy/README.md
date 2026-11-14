# Cold qualification materials

`contract.md` states the protocol. `projects.tar.gz` holds, per family (N1, N2, N3): `brief.md`, `change.md`,
the fake service (`stub/`), the black-box evaluators (`evaluator/`), defect controls (`controls/`) and diagnose
materials (`diagnose/`), plus each family's `NOTES.md`. Extract with `tar -xzf projects.tar.gz`; run
`sh validate_controls.sh` from the extracted layout to repeat the control validation recorded in
`controls-validation.log`.

sha256 of projects.tar.gz: `a8bddb7cb5889b598b2f3b9e39546968a29ede720d7dc7c19fa76ec30f530529`

The evaluators are archived as a tarball (not unpacked in the tree) because they are experiment fixtures run by
their own harness, not part of the product's test suite.
