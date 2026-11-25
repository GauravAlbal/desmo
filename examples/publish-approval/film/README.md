# Exact-artifact approval film

This local, synthetic demo is driven by the pinned observed trace in `source.trace.json`. It shows approval, an edit that invalidates that approval, refused publication authorization, a canonical read, renewed approval, and successful committed progress. A read or historical receipt replay is not a commit. The refusal is not represented as reaching `publishing`.

## Reproduce

Use the matching public release, not a source-built runtime. Install the released binary and Python wheel from the GitHub Release first (Apple silicon example):

```sh
gh release download v0.5.0 -R GauravAlbal/desmo \
  -p 'ik-darwin-arm64.tar.gz' \
  -p 'invariant_kernel-0.5.0-py3-none-any.whl' -p SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing
tar -xzf ik-darwin-arm64.tar.gz
python3 -m pip install --no-deps invariant_kernel-0.5.0-py3-none-any.whl
export IK_BINARY="$PWD/ik"
```

Both the trace generator and film builder fail closed unless the installed `invariant-kernel` Python package version and `IK_BINARY --version` identity match the pinned trace runtime version `0.5.0`; the binary must report protocol `1.0`.

The build requires Python 3, headless Google Chrome, FFmpeg, and FFprobe. The scripts first search `PATH`, then check the macOS defaults (`/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`, `/opt/homebrew/bin`, and `/usr/local/bin`). Set `IK_CHROME`, `IK_FFMPEG`, or `IK_FFPROBE` to override a tool path. Install the matching release assets and set `IK_BINARY` as shown above.

From the repository root, regenerate the report, storyboard, poster, six stills, muted MP4, overflow/frame-state checks, and negative-control log:

```sh
python3 examples/publish-approval/film/build.py
```

The build runs the standalone negative controls and appends render checks to `output/validation.log`; no script writes a `validation.log` beside this README.

The pinned source trace was produced by `flow.py` with the matching release binary. Regenerate it in place from a fresh run:
```sh
IK_BINARY="$PWD/ik" python3 examples/publish-approval/film/flow.py \
  --out examples/publish-approval/film/source.trace.json
```

The trace bundle keeps the product identity and `ikaros.trace.v2` event fields unchanged. On the refusal event, `filmFlowAnnotation` is an additive film-flow field containing JSON with `kind`, `approvedDigest`, and `proposedDigest`; those digest values are computed from the real approved aggregate and current draft bytes at the refusal. This annotation is not a product trace field. The film shows committed lifecycle state separately from the refused attempt: during the refusal and canonical-read scenes, the Authorized tracker node remains unlit. Headless Chrome checks text bounds in every scene; ffmpeg decodes and checks the Authorized-node pixels in every encoded refusal/read frame before renewed approval. The pinned trace records runtime version, machine id, and evidence events; film build uses it for event facts.

Run automated negative controls with:

```sh
python3 examples/publish-approval/film/checks.py
```

One-time mutation check: I disabled each of the four corresponding guards in a temporary copy of the product files. In every case, the matching negative control failed with `invalid input accepted`; the checked-in guards remain enabled.

## Outputs

`output/report.html`, `output/approval-film.mp4`, `output/poster.png`, six `output/still-N.png` images, `output/facts.json` (on-screen fact to event mapping), and `output/manifest.json` (size, SHA-256, duration, identities and output list). `output/validation.log` records the negative-control rejections and render checks.

The comprehension questionnaire, answerable from the muted film alone:

1. What rule is protected?
2. What proposal failed?
3. What is the correct next action?

Human run: open owner item; no human evidence is claimed. Any model-reader result must be labelled as a model-reader result, not human evidence.

This demonstrates publication authorization within the example boundary. It does not prevent arbitrary external API calls, does not attest to external execution, and uses no uploaded or user-provided artifact. Declared behavior is not demonstrated human comprehension.
