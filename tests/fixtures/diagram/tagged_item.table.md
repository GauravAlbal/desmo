| command | from | to     | guard                               | effect | class    |
| ------- | ---- | ------ | ----------------------------------- | ------ | -------- |
| tag     | idle | tagged | text_in($id, 'A-Za-z0-9._-', 1, 64) |        | declared |

Rules: given = supplied by the caller, stored = kept by the machine.

| name | rule                                    | verbatim                            |
| ---- | --------------------------------------- | ----------------------------------- |
| tag  | id (given) in A-Za-z0-9._- length 1..64 | text_in($id, 'A-Za-z0-9._-', 1, 64) |
