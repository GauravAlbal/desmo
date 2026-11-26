# Machine language

Declarative machines are `.ik.toml` files: facts, states, commands,
guards, effects, invariants. `ik identity` prints the kernel identity
`(machine_id, version, spec_hash)`. Clients must not compute the hash.

Commands are requested by **name and args**. Destination states are not
sent. The kernel selects the outcome.

## Guard and `then` grammar

Guards (`when`, invariant `assert`) are a bounded grammar, not a general
evaluator. Shared runtime, check, the library explain function, scenario, Hegel, and TLC-lowering
consumers use the same expression AST.

```text
expr        := or
or          := and ( '||' and )*
and         := atom ( '&&' atom )*
atom        := '(' expr ')' | cmp | 'exists' '(' IDENT ')' | 'not_exists' '(' IDENT ')' | text_in
text_in     := 'text_in' '(' value ',' STRING ',' INT ',' INT ')'
cmp         := operand ( '==' | '!=' | '<' | '<=' | '>' | '>=' ) operand
operand     := value ( ('+' | '-') (INT | value) | '*' POSINT | '/' POSINT )?
value       := '$' IDENT | IDENT | STRING | INT | 'true' | 'false' | 'state'
             | 'effect_status' | 'effect_kind' | 'effect_outcome'
POSINT      := INT greater than 0
```

Command guards (not `[invariants]`) may also read three reserved runtime effect operands:
- `effect_status`: `"none"`, `"pending"`, or `"recorded"`.
- `effect_kind`: the planned or recorded effect kind string (evaluation error when `effect_status == "none"`).
- `effect_outcome`: the recorded `outcome_digest` string (evaluation error unless `effect_status == "recorded"`).

Using these in a follow-up command (`when = "effect_status == 'recorded' && effect_outcome == 'succeeded'"`) ensures the kernel itself refuses completion unless a matching effect outcome was recorded. `effect_status`, `effect_kind`, and `effect_outcome` are reserved identifiers and cannot name facts or arguments; SMT leaves effect-guarded obligations `Unknown`.

String literals in guards use `'paid'` or `"paid"`. `exists(fact)` and
`not_exists(fact)` test presence. `text_in(VALUE, SET, MIN, MAX)` is a
bounded string predicate: VALUE is a `$arg` or a string fact (not a
literal and not `state`); SET is an ASCII character class (`a-z` ranges,
`-` literal when first or last, `*` as the whole set meaning any Unicode
scalar); MIN and MAX are Unicode-scalar length bounds in `0..=4096`.
Length is `str::chars` count. Membership is exact: no case folding,
Unicode normalization, locale, or trimming. One `holds()` implementation
is shared by runtime evaluation, the library explain function, explore, scenario replay, and
Hegel. SMT does not encode it. Feasibility treats it as opaque, so a
generated obligation is never discharged by `text_in` alone. TLC checks
the exported graph, not guards. An older binary that does not know the
form reports a syntax error and does not load the machine. Schema version
is unchanged. See `examples/shipped/tagged_item.ik.toml`.

Integer `+`/`-` may add or subtract a
non-negative constant (`attempt_count + 1`) or another integer operand
(fact or `$arg`, either side: `remaining_qty - $units`, `$now_ms + hold_ms`).
`*` and `/` take a positive integer constant only (`units * 175`,
`amount / 50`). Division is toward-zero `i64`. Overflow refuses with
`integer arithmetic overflow`; an unset fact is an evaluation error and
does not fall through. There is no `%`, chaining, `min`/`max`, floats, or
rounding modes.

`then` assigns a literal, `$arg`, `+N` (on a **counter** fact only), or a bounded checked `i64` arithmetic expression using the same operand grammar as guards (`remaining_qty = "remaining_qty - $units"`, `total = "$price * 3"`). Multi-key `then` updates evaluate simultaneously against pre-transition facts and args, and any arithmetic overflow refuses with `IK_INVARIANT_FAILED` (`integer arithmetic overflow`).

Editor autocomplete & validation: [schema/ik.schema.json](../schema/ik.schema.json) (JSON Schema Draft-07) covers both schema 1 and schema 2 `.ik.toml` files.

Then-literal quoting and native TOML values:
- String literals require inner double quotes inside a TOML string: `then = { status = '"paid"' }` or `then = { status = "\"paid\"" }` stores `paid`; `then = { deposit_id = '""' }` stores an empty string (clearing a string fact). Bare `then = { status = "paid" }` is rejected so typos fail loudly instead of storing unintended strings.
- Booleans and integers accept either native TOML literals (`then = { approved = true, retries_left = 3 }`) or quoted strings (`then = { approved = "true", retries_left = "3" }`) and hash to the exact same `spec_hash`.
- Counter increments use `"+N"` on `policy = "counter"` facts only.
- `policy = "once"` facts refuse overwriting once set (`IK_INVARIANT_FAILED`), and `policy = "kernel_only"` facts refuse assignment in `then`.
- `[invariants]` and command guards statically reject type mismatches, `$arg` references inside `[invariants]`, and unknown `state == '...'` literals.
- Command arguments use `"$arg"`, whose declared type must match the target fact type.

Native machines (a native delegation lifecycle today) stay in a thin product package
above the generic client. They are not a second protocol.

## Schema versions

Schema 1 declares flat states and commands. Its deserialization, runtime
semantics, and identity hash are unchanged by the schema-2 frontend.

Schema 1 requires an explicit `[facts]` table even when it has no entries; schema 2 defaults it to empty.

Schema 2 declares regions in the same `.ik.toml` file, then lowers them to
the existing schema-1 runtime definition. `schema_version = 2`, `machine_id`,
and `machine_version` are required; `name` is optional, and authored facts
and invariants default to empty maps. For example:

```toml
schema_version = 2
machine_id = "player"
machine_version = 1

[regions.playback.states.playing]
initial = true
[regions.playback.states.paused]
[regions.playback.commands.pause]
from = ["playing"]
to = "paused"

[regions.volume.states.unmuted]
initial = true
[regions.volume.states.muted]
[regions.volume.commands.mute]
from = ["unmuted"]
to = "muted"

[commands.pause_and_mute]
from = { playback = ["playing"], volume = ["unmuted"] }
to = { playback = "paused", volume = "muted" }
```

Local `pause` preserves volume; the explicit joint `pause_and_mute` changes
both regions atomically. Joint source regions omitted from `from` are
wildcards; target regions omitted from `to` remain unchanged. Region-local
commands retain schema-1 `from`, `to`/`when`/`then` or ordered `outcomes`,
plus `args` and `effect`. Joint commands use region maps for sources and
targets. This is deterministic named command-scoped execution, **not SCXML
broadcast or run-to-completion**. Command names have one region/joint owner,
except for a child/parent pair within one region.

A region may declare one nonrecursive parent with `name`, `children`, and
`commands`. Child outcomes precede parent fallback. All child guards must
evaluate false before fallback; an evaluation error refuses, and an
unconditional final child outcome suppresses fallback at that source.
Shared child/parent commands require exactly equal serialized `args` and
`effect` declarations.

Use generated string facts such as `region_playback == "playing"` in
cross-region guards and invariants. The authored `state` operand is refused
structurally in schema 2; unknown leaf strings in region comparisons are
also refused. Authored fact/invariant names starting `region_` and
assignments to generated region facts are forbidden. Generated hard
invariants enforce agreement with the flat canonical state.
Lowered outcomes guard source agreement too, and decoding refuses inconsistent
state/fact pairs. Schema-2 initial invariant violations are refused at parse
time using the runtime's shared invariant evaluator; schema 1 is unchanged.

Region, leaf, and parent names are ASCII identifiers without `__`.
Lexically ordered regions produce flat leaf names joined with `__`. Each
region has one nonterminal initial leaf; the aggregate is terminal only
when all leaves are terminal. Local commands cannot source terminal leaves.
Joints cannot explicitly source terminal leaves or target currently terminal
components through wildcard sources, even with same-leaf assignments;
omitted terminal siblings remain unchanged.
The current frontend limit is 64 product states, checked with overflow
detection before Cartesian-product allocation; it is not a CORE_LAW.

Save the example as `player.ik.toml`, then use:

```sh
ik identity player.ik.toml
ik check player.ik.toml
ik check player.ik.toml --corpus cases.json
```

These commands are examples, not validation evidence. Identity is the hash
of the lowered runtime definition; authoring metadata is not a second
machine identity. Check output prints the overall `VERDICT` before derived
region paths and Mermaid charts. A reachable invariant-refusal witness
means the kernel blocked an unsafe proposal, not that it committed an
unsafe state. Such a witness makes the region check REFUSED. ACCEPT remains
bounded and corpus-relative, not a proof over all inputs and unbounded paths.
“Complete within the supplied finite corpus” is not exhaustive. Invariant
refusals are included in the shared pipeline's flat judgment as well as the
overall verdict and text, not applied as a CLI-only override.
Charts are lossy structural views: guards, invariants, facts, and joint
atomicity are not shown.
The schema-1 `machines/invariant_probe.ik.toml` intentionally violates an
invariant and is expected to check as REFUSED; its hash remains unchanged.

See [Region lowering](region-lowering.md) for complete parent and joint
examples, resource limits, witness projection, and Rust loader/check APIs.
