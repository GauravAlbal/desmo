# Batch 017: evaluator-kit archive reseal record

This record corrects the lineage of the retained capture-and-replay evaluator kit archive cited in `DECISION.md` (addendum 2) and `results.json`. The earlier record is preserved; nothing is overwritten.

## Identities

| Item | Digest (sha256) | Note |
|---|---|---|
| Archive cited in the earlier record | `cba44f3eb1c2b1075f1344b5f0023399c7a672da0824b894f908bb7755e06b27` | Sealed immediately after the four capture-and-replay rescorings. It contained the kit directory including the evaluator scratch directory (about 54 MB of generated run scratch). The archive bytes were later overwritten at the same path; only this digest is retained. |
| Archive after reseal | `ee502afc0d61bc635ac81e94e8eb79f568bdd473e2a88cdf5682615c4635491e` | Sealed without any `scratch` directory. Member listing contains no `scratch` entry; 328 entries. |
| Kit member manifest | `6b8552fc2af10346862ac833081d042f333cc092e7638b78e801e2d97990dcad` (file `MANIFEST.sha256`, 243 files) | Delivered and hashed by the kit builder before any workspace was removed. Verified with zero mismatches before the four rescorings, after them, and again after the reseal. `MANIFEST.sha256` lists no scratch file. |
| Scorer used by the four rescorings | kit `b17_score.py` `689dde29d1d25f71c5a22a49e00b14f2be339b4511863982412493cd6cc8ee00` | Invoked through a parallel driver. The experiment runner (`exp.py`) was not the scorer for those four runs; its first scoring use is batch 018. Its digest is recorded in the batch 018 manifest. |

## Cause

The kit directory had accumulated evaluator scratch output while the retained deliveries were scored; the first seal captured part of it. The reseal removed the scratch from the retained kit and archive so the retained artifact is the manifest-listed kit only.

## What this establishes and what it does not

Established: every file in the kit's member manifest was unchanged across the reseal (the manifest verified clean immediately before and after), and the new archive contains no scratch content. Not established: a byte-level comparison of the old archive's contents against the new one, because the old archive bytes no longer exist. The reseal is therefore a custody and packaging change with the manifest-listed evaluator semantics unchanged; it is not proven by diffing the old archive, and the manifest-based evidence above is the support for that statement.

The earlier results (first scoring, rescorings v2 to v4) and their archive identities remain as recorded. Any future reseal must keep the previous archive until the new archive's member manifest has been compared and recorded.
