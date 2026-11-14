import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const CHECKS = `profile_version = 1

[[goals]]
id = "verified"
kind = "reach_terminal"
terminal = "verified"
required = true

[[goals]]
id = "not-stuck"
kind = "no_stuck_states"
required = true
`;

const CASES = `{
  "commands": {
    "submit": [
      {
        "name": "ok",
        "args": { "artifact_digest": "a", "baseline_digest": "b" }
      }
    ],
    "verify": [
      {
        "name": "ok",
        "args": {
          "receipt_digest": "r",
          "receipt_artifact_digest": "a",
          "receipt_baseline_digest": "b"
        }
      }
    ]
  }
}
`;

const NORMAL = `{
  "scenario_version": 1,
  "machine": "../job.ik.toml",
  "aggregate": "job-1",
  "steps": [
    {
      "kind": "command",
      "label": "start",
      "name": "start",
      "expect": "committed"
    },
    {
      "kind": "command",
      "label": "submit",
      "name": "submit",
      "args": { "artifact_digest": "a", "baseline_digest": "b" },
      "expect": "committed"
    },
    {
      "kind": "command",
      "label": "verify",
      "name": "verify",
      "args": {
        "receipt_digest": "r",
        "receipt_artifact_digest": "a",
        "receipt_baseline_digest": "b"
      },
      "expect": "committed"
    }
  ]
}
`;

const STALE = `{
  "scenario_version": 1,
  "machine": "../job.ik.toml",
  "aggregate": "job-1",
  "steps": [
    {
      "kind": "command",
      "label": "start",
      "name": "start",
      "handle": "cap-start",
      "expect": "committed"
    },
    {
      "kind": "command",
      "label": "submit",
      "name": "submit",
      "args": { "artifact_digest": "a", "baseline_digest": "b" },
      "expect": "committed"
    },
    {
      "kind": "command",
      "label": "stale",
      "name": "cancel",
      "use_handle": "cap-start",
      "expect": "refused:IK_CAPABILITY_STALE"
    }
  ]
}
`;

export const WORKBENCH_README = `
## Authority and executor

\`job.ik.toml\` is the lifecycle definition. The \`ik\` runtime is the authority:
it commits or refuses each command. A refused command is not a committed state,
and a read is not a mutation. \`job.mjs\` is a client. It may propose commands.
It does not decide that a stale handle is current.

Scenarios record synthetic outcomes only. They do not send mail, publish an
artifact, or call an external executor. \`ik check\` does not authorize a
production commit. A missing witness is not proof of unreachability unless the
search completed.

\`ik check\` without \`--corpus\` loads \`job.ik.cases.json\` beside the machine.
That file is input coverage for the goals, not a second definition.

    ik check job.ik.toml --profile .ik/checks.toml
    ik scenario run job.ik.toml scenarios/normal.json --out .ik/runs/normal

The adverse schedule is \`scenarios/stale-handle.json\`. Its last step expects
\`refused:IK_CAPABILITY_STALE\`. Meeting that expectation means the runtime
refused the stale handle and committed nothing.
`;

export function writeJobWorkbench(target: string): void {
  mkdirSync(join(target, ".ik"), { recursive: true });
  mkdirSync(join(target, "scenarios"), { recursive: true });
  writeFileSync(join(target, ".ik", "checks.toml"), CHECKS);
  writeFileSync(join(target, "job.ik.cases.json"), CASES);
  writeFileSync(join(target, "scenarios", "normal.json"), NORMAL);
  writeFileSync(join(target, "scenarios", "stale-handle.json"), STALE);
}
