# Scoring

One unfamiliar reader. At most 20 minutes. Score the **readout**, not the person.

For each numbered question, mark exactly one:

- **matches key** — the answer agrees with `ANSWER-KEY.md` on the named facts
- **partially** — some of the key is present; a named fact is missing or reversed
- **misses** — the answer is absent, opposite, or not grounded in the readout

A miss, or a partial that drops a named key fact, is a **readout finding**. Record it in `FINDINGS-HUMAN.md`. It is not a reader failure. Jargon, JSON, or status words the product printed that a non-technical reader cannot use count as readout defects when they cause the miss.

Unanswered items at 20 minutes are misses (readout findings).

## BUILD

1. What will the application do? — matches key / partially / misses
2. What is not yet established? — matches key / partially / misses
3. Does anything require your decision? — matches key / partially / misses

## CHANGE

1. What changes for new instances? — matches key / partially / misses
2. What happens to existing ones? — matches key / partially / misses
3. Is a migration or permission needed? — matches key / partially / misses

## DIAGNOSE / RECOVER

1. Did the operation commit? — matches key / partially / misses
2. What is still uncertain? — matches key / partially / misses
3. What action is safe next? — matches key / partially / misses
