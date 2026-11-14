# READOUT

Entry: you have run `ik handoff` and must tell a human what was done. This is the summary you write around the handoff. Desmo owns state transitions. It does not own whether the criteria are valid or whether execution did what was intended, and it states no readiness. Your summary must not say more than the handoff does.

## Steps

1. Save the handoff text: `ik handoff <behavior.json> --evidence <path>... --kind <kind> > handoff.txt` (text output, not `--json`).
2. Write `summary.md`. Put the handoff's own lines in `>` quotes and your words outside them.
3. Run `ik readout summary.md --handoff handoff.txt`. Fix every defect it prints and run it again. Each defect says which vector fired, on which line, what is wrong and how to fix it.
4. When it is clean, run `ik readout summary.md --handoff handoff.txt --stamp`. The stamp is written only if the lint is clean. Edit the summary or the handoff afterwards and the stamp goes stale: lint again, then stamp again.

## What the summary must contain

- The handoff's `Status:` line, quoted verbatim, near the top (delivery and change handoffs; a recovery handoff has no Status line).
- The handoff's `Not checked:` paragraph, quoted verbatim.
- The handoff's `Claim:` paragraph, quoted verbatim. It is the one sentence Desmo stands behind; your own words may follow it but must not say more than it does.
- Every material gap in `What is missing`, named or quoted, each with what you did about it: fixed, accepted by the owner, or left for the owner to decide.
- What you ran, with the evidence ids the handoff cites.

## Rules

- Quote, do not paraphrase. A `>` block must be verbatim handoff text. To skip lines, put a `>` line holding only `…` between the parts.
- Your own words go outside `>` quotes and are labeled as yours.
- Do not write readiness or completeness words: ready, complete, verified, safe, correct, approved, nothing missing, no gaps, all cases. The owner decides readiness. A list that says `none found` is a count over a stated scope, not a statement that nothing exists.
- Do not write proof words: prove, guarantee, certain, ensure. Evidence is observed or explored within a stated scope, and declared is never demonstrated.
- Do not drop a gap because it is awkward. A listed gap you leave out is a defect.
- If the summary is too long for the reader, say what you cut and where the full handoff is. Do not summarize silently.
- Atomize the requirement as the handoff asks: verbatim spans that cover every word of every sentence, each traced to a real element or marked not applicable with a reason. Do not use one atom for the whole requirement.

## What the lint cannot do

It checks wording and quoting. It cannot tell whether your account of the work is true. A clean lint and a stamp mean the wording rules held; the hash binds the summary and handoff text, not who wrote it. The reviewer still reads the summary against the handoff.

## Defect vectors

| vector | what fired |
|---|---|
| `readiness_word` | prose uses a readiness word (ready, complete, verified, safe, correct, approved) |
| `completeness_claim` | prose says nothing is missing or all cases are covered |
| `proof_word` | prose says prove, guarantee, certain or ensure |
| `quote_not_in_handoff` | a `>` block is not verbatim handoff text |
| `status_not_quoted` | the handoff has a `Status:` line and it is not quoted |
| `not_checked_not_quoted` | the `Not checked:` paragraph is not quoted |
| `claim_not_quoted` | the `Claim:` paragraph is not quoted |
| `gap_not_mentioned` | a material gap in `What is missing` is not named or quoted (a table row is matched by its subject, for example `tag $label`; a machine-level gap such as no invariants by the plain word `invariant`) |
| `stamp_stale` | the summary or handoff changed after it was stamped |
| `stamp_malformed` | a stamp line is present but was not written by `--stamp` |
