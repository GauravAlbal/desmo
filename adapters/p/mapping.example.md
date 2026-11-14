STATUS: prepared, not qualified; no real P run exists

# Mapping example: one Desmo command is not one P event

This file is a prepared mapping. No P checker has been run against it.

## Desmo command

`approve` on machine `publish-approval`, from state `pending` to state `approved`, is one atomic runtime transition. The command either commits or is refused. It is not a message sitting in another machine's queue.

## P event

`eApprove` is an event the P model may declare on machine `ApprovalHost`.

- Queue. Default P delivery is reliable FIFO per sender. `eApprove` waits in the target machine's input queue until that machine dequeues it. Other events already in that queue run first.
- Send. `send host, eApprove, payload` enqueues. It does not run the handler in the sender's turn, and it does not commit a Desmo aggregate.
- Monitor. `ApprovalMonitor` can observe `eApprove`. A monitor that is still hot at the end of a finite schedule is not a proved liveness failure. That conclusion needs the selected checking mode, and this route has no qualified mode.
- Schedule. A step bound or a schedule bound is `bounded_paths`. It is not an exhaustive proof of the Desmo machine, and it is not `holds_in_scope`.

## What this mapping does not say

Default P does not drop, duplicate, or reorder that queue. Mapping `approve` to `eApprove` does not reproduce interleavings of a lossy or reordering network unless the P model states that behavior itself. A Desmo refusal is not a P event that failed to be sent. Imported monitor results cover only the events the trace actually contained.

The version probe is `p --version`. When that command is absent, describe reports `tool_unavailable` and no checker runs.
