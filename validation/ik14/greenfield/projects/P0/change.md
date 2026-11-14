# Change: reprint a completed label

The spooler you are maintaining already implements the label printing brief. Requirements R1 through R9 stay in force. This change revises none of them. It adds one command and requirement R10.

The operator must be able to print a completed label again without starting a new state directory and without sending the original job to the printer a second time. Reprint creates a new job linked to the completed job. The new job carries the same captured label bytes. It is dispatched on its own, at most once. The recorded result of the original job does not change.

## Command

### `./run reprint <job_id>`

Allowed only when that job exists and its stored state is `printed`.

Create a new job. Do not contact the printer. Do not allocate a dispatch id. Do not change the source job.

The new job has:

- a new job id, different from the source
- the same printer id
- the same captured label bytes and digest as the source job, copied from the stored record, not read from a file
- state `queued`
- dispatch id null
- `printed_digest` null
- `reprint_of` set to the source job id

Success, exit 0:

```json
{"job_id":"...","printer_id":"...","digest":"...","state":"queued","reprint_of":"..."}
```

Refuse if the job does not exist or its state is not `printed`. A refusal must not create a job, must not change the named job, and must not contact the printer.

After this change, `status` includes `reprint_of`. It is the source job id when the job was created by `reprint`, and null otherwise. The source job keeps `reprint_of` null. Fields already required by the brief stay required.

The new job is an ordinary job. `hold`, `cancel`, and `dispatch` apply to it. `dispatch` of the new job allocates a new dispatch id. It must not reuse the source job dispatch id. A successful dispatch counts one new print effect, and that effect records the new job id. A second `dispatch` of the new job does not add another effect. The source job remains `printed` with the same dispatch id, `printed_digest`, digest, and label bytes. The source print effect is unchanged.

Each successful `reprint` of a printed job creates another new job. A printed reprint may itself be reprinted. That second reprint is specified and not separately scored.

## Requirements

R10. Reprint. After a job is `printed`, `reprint` exits 0, does not add a print effect, and creates a new queued job whose `reprint_of` is the source job id, whose digest and label bytes equal the source, and whose dispatch id is null. The source job stays `printed` with the same dispatch id, `printed_digest`, digest, and label bytes, and its `reprint_of` stays null. `dispatch` of the new job against a healthy service exits 0, records `printed` with a dispatch id different from the source, and causes the service to count exactly one new effect whose job id is the new job. The source effect is unchanged. A further `dispatch` of the new job does not add another effect, and the source job record stays as it was. `reprint` of a job that is not `printed` is refused, adds no print effect, and does not change that job.
