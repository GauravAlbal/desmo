#!/usr/bin/env python3
"""Read SMT-LIB2 from stdin and print cvc5 check-sat / get-value output.

Used when the cvc5 package ships the Python module and not a CLI binary.
No network. The interpreter is the pinned subprocess, not a crate dependency.
Pin the shebang (or PATH) to the same interpreter that can `import cvc5`.
"""

import signal
import sys

import cvc5


def is_null(cmd):
    text = str(cmd).strip()
    return text == "null" or not text


def run(smt2):
    solver = cvc5.Solver()
    solver.setOption("produce-models", "true")
    parser = cvc5.InputParser(solver)
    parser.setStringInput(cvc5.InputLanguage.SMT_LIB_2_6, smt2, "<stdin>")
    while True:
        cmd = parser.nextCommand()
        if is_null(cmd):
            break
        result = cmd.invoke(solver, parser.getSymbolManager())
        if result:
            sys.stdout.write(str(result))
            if not str(result).endswith("\n"):
                sys.stdout.write("\n")
        if parser.done():
            break
    sys.stdout.flush()


def parse_cli(argv):
    version = False
    limit_ms = None
    files = []
    for arg in argv:
        if arg in ("--version", "-V"):
            version = True
        elif arg.startswith("--tlimit="):
            limit_ms = int(arg.split("=", 1)[1])
        elif arg.startswith("-"):
            continue
        else:
            files.append(arg)
    return version, limit_ms, files


def arm_timer(limit_ms):
    if limit_ms is None:
        return

    def on_timeout(_signum, _frame):
        sys.stdout.write("unknown\n")
        sys.exit(0)

    signal.signal(signal.SIGALRM, on_timeout)
    signal.setitimer(signal.ITIMER_REAL, max(limit_ms / 1000.0, 0.05))


def print_version():
    solver = cvc5.Solver()
    version = solver.getVersion()
    if isinstance(version, bytes):
        version = version.decode()
    sys.stdout.write("cvc5 %s\n" % version)


def load_smt2(files):
    if files:
        with open(files[-1]) as handle:
            return handle.read()
    return sys.stdin.read()


def main():
    version, limit_ms, files = parse_cli(sys.argv[1:])
    if version:
        print_version()
        return
    arm_timer(limit_ms)
    run(load_smt2(files))


if __name__ == "__main__":
    main()
