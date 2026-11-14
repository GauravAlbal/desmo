| command  | from    | to       | guard                                       | effect | class    |
| -------- | ------- | -------- | ------------------------------------------- | ------ | -------- |
| checkout | idle    | on_loan  | $loan_until_ms == $checkout_ms + 1209600000 |        | declared |
| checkin  | on_loan | returned | $overdue == 0 \|\| recall_placed == 1       |        | declared |
| recall   | on_loan | on_loan  |                                             |        | declared |

Rules: given = supplied by the caller, stored = kept by the machine.

| name     | rule                                                | verbatim                                    |
| -------- | --------------------------------------------------- | ------------------------------------------- |
| checkout | loan_until_ms (given) is checkout_ms (given) + 14d  | $loan_until_ms == $checkout_ms + 1209600000 |
| checkin  | overdue (given) is 0 or recall_placed (stored) is 1 | $overdue == 0 \|\| recall_placed == 1       |
