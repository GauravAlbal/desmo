"""Stage acceptance contract for the DFY evaluators.

A stage passes only if the evaluator process terminated normally, every required
(rule, criterion) coordinate arrived exactly once with a strict boolean result, and
every result is true. Missing, duplicate, conflicting or malformed rows never become
passes. Twin-construction and routing gaps are inconclusive, never application failures.
"""

STATUSES = ("pass", "fail", "inconclusive", "incomplete", "invalid", "runner_failure")
INCONCLUSIVE_PREFIXES = ("twin_failed", "unroutable")


def required_coordinates(rules, criteria):
    """Contract: every rule needs every criterion unless the caller lists an explicit exemption."""
    return {(rule, crit) for rule in rules for crit in criteria}


def is_inconclusive(row):
    why = row.get("why")
    return isinstance(why, str) and why.startswith(INCONCLUSIVE_PREFIXES)


def _malformed(row):
    rule, crit, passed = row.get("rule"), row.get("criterion"), row.get("pass")
    return not (isinstance(rule, str) and type(crit) is int and type(passed) is bool)


def _index(rows):
    seen, problems = {}, []
    for row in rows:
        if _malformed(row):
            problems.append(("malformed", repr(row)[:120]))
            continue
        key = (row["rule"], row["criterion"])
        if key in seen:
            problems.append(("conflicting" if seen[key]["pass"] != row["pass"] else "duplicate", key))
            continue
        seen[key] = row
    return seen, problems


def _status(runner, problems, missing, conclusive, failed):
    gates = (
        ("runner_failure", bool(runner.get("timeout")) or runner.get("rc") != 0),
        ("invalid", bool(problems)),
        ("incomplete", bool(missing)),
        ("fail", bool(conclusive)),
        ("inconclusive", bool(failed)),
    )
    return next((name for name, hit in gates if hit), "pass")


def evaluate_stage(rows, required, runner):
    """rows: delivered rule-level rows; required: set of (rule, criterion); runner: {'rc': int, 'timeout': bool}."""
    seen, problems = _index(rows)
    missing = sorted(required - set(seen))
    failed = [k for k in sorted(required & set(seen)) if not seen[k]["pass"]]
    conclusive = [k for k in failed if not is_inconclusive(seen[k])]
    status = _status(runner, problems, missing, conclusive, failed)
    return {"status": status, "accepted": status == "pass", "missing": missing, "failed": failed,
            "conclusive_failures": conclusive, "problems": problems}
