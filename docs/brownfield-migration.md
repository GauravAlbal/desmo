# Brownfield Integration: Read-Model Projections & Anti-Bypass Guard

When adopting Desmo in an existing TypeScript, Python, or Go service, **do not dual-write lifecycle state**. Move **one** lifecycle boundary into Desmo, delete the host's direct status branches, and project Desmo's committed state read-only into your existing Postgres or SQLite read tables.

---

## 1. Architecture: Write Authority vs. Read-Model Projection

```text
  API / Worker ──► Desmo Kernel (order.ik.toml + SQLite journal)
                         │
       ik events --since N --json (ordered by monotonic cursor)
                         │
                         ▼
               Your App DB Read Model
     (generation-fenced UPSERT: WHERE desmo_gen < excluded.desmo_gen)
```

1. **All writes go through commands (`order.pay()`, `order.refund()`).** Desmo returns `(generation, state, terminal)`.
2. **Projections are read-only & generation-fenced:** `ik events <store.sqlite> --since <cursor> --json` streams committed events in global commit order and includes each aggregate's `current_projection` (`state`, `sequence`, `generation`, `terminal`) plus `is_latest_for_aggregate`.
3. **CI stops bypass drift (`ik guard check`):** Declare forbidden raw status writes in `.ik/guard.toml` so future PRs or coding agents cannot re-introduce direct `UPDATE orders SET status = ...` mutations outside your authorized projection module.

---

## 2. Copy-Paste Brownfield HTTP + Projection Recipes

### TypeScript (Express + Desmo + Read-Model Projector)

```ts
import express from "express";
import { Kernel, IllegalStateError } from "@desmo/runtime";

const app = express();
app.use(express.json());
const kernel = new Kernel(".state/orders.db");
const orders = await kernel.machine("order.ik.toml");

app.post("/orders/:id/pay", async (req, res) => {
  try {
    const cur = await orders.show(req.params.id);
    const next = await cur.command("pay", { payment_ref: req.body.paymentRef }, { operationId: req.header("Idempotency-Key") });
    await projectRow(next.id, next.namedState(), Number(next.generation));
    res.json({ id: next.id, state: next.namedState(), generation: String(next.generation) });
  } catch (err) {
    if (err instanceof IllegalStateError) return res.status(409).json({ error: err.code, message: err.message });
    throw err;
  }
});

// Background catch-up projector using first-class `kernel.events()` (no manual shelling out):
export async function syncReadModel(lastCursor: number | bigint): Promise<number | bigint> {
  const feed = await kernel.events({ since: lastCursor });
  for (const ev of feed.events) {
    if (ev.isLatestForAggregate) {
      const stateName = String((ev.currentProjection.state as { state?: string }).state ?? "");
      await projectRow(ev.aggregateId, stateName, Number(ev.currentProjection.generation));
    }
  }
  return feed.nextCursor;
}
```

### Python (FastAPI + Desmo + Read-Model Projector)

```python
from fastapi import FastAPI, HTTPException, Header
from invariant_kernel import Kernel, IllegalStateError

app = FastAPI()
kernel = Kernel(".state/orders.db")
orders = kernel.machine("order.ik.toml")

@app.post("/orders/{order_id}/pay")
def pay_order(order_id: str, payment_ref: str, idempotency_key: str | None = Header(default=None)):
    try:
        cur = orders.show(order_id)
        nxt = cur.pay(payment_ref=payment_ref, operation_id=idempotency_key)
        upsert_read_model(nxt.id, nxt.state, nxt.generation)  # WHERE desmo_generation < excluded.desmo_generation
        return {"id": nxt.id, "state": nxt.state, "generation": nxt.generation}
    except IllegalStateError as err:
        raise HTTPException(status_code=409, detail={"code": err.code, "message": err.message})

def sync_read_model(last_cursor: int) -> int:
    feed = kernel.events(since=last_cursor)
    for ev in feed.events:
        if ev.is_latest_for_aggregate:
            proj = ev.current_projection
            state_name = proj.state["state"] if isinstance(proj.state, dict) else ""
            upsert_read_model(ev.aggregate_id, state_name, proj.generation)
    return feed.next_cursor
```

### Go (`net/http` + Desmo + `database/sql` Projector)

```go
package main

import (
	"context"
	"net/http"

	ik "github.com/GauravAlbal/desmo/go"
)

func payHandler(m *ik.Machine, upsert func(id, state string, gen uint64) error) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		id := r.PathValue("id")
		cur, err := m.Show(r.Context(), id)
		if err != nil {
			http.Error(w, err.Error(), http.StatusNotFound)
			return
		}
		next, hist, err := cur.CommandWithOperation(r.Context(), r.Header.Get("Idempotency-Key"), "pay", map[string]any{"payment_ref": r.URL.Query().Get("ref")})
		if err != nil {
			http.Error(w, err.Error(), http.StatusConflict)
			return
		}
		if next != nil {
			_ = upsert(next.ID, next.StateName(), next.Generation)
		} else if hist != nil {
			_ = hist
		}
	}
}

func syncReadModel(ctx context.Context, k *ik.Kernel, lastCursor uint64, upsert func(id, state string, gen uint64) error) (uint64, error) {
	feed, err := k.Events(ctx, ik.EventsOptions{Since: lastCursor})
	if err != nil {
		return lastCursor, err
	}
	for _, ev := range feed.Events {
		if ev.IsLatestForAggregate {
			if st, ok := ev.CurrentProjection.State["state"].(string); ok {
				_ = upsert(ev.AggregateID, st, ev.CurrentProjection.Generation)
			}
		}
	}
	return feed.NextCursor, nil
}
```

> **Deployment Topology Note:** Desmo stores are local single-node SQLite databases. In brownfield web backends, point a single dedicated writer/worker service (or single-instance deployment) at `.state/orders.db` rather than letting multiple stateless containers write to unshared local SQLite files.

---

## 3. Locking Down Bypass Writes in CI (`ik guard`)

After moving status transitions into Desmo, prevent accidental regressions or coding-agent shortcuts by adding `.ik/guard.toml` (or deriving rules directly from your `.ik.toml`):

```sh
ik guard init --machine order.ik.toml --table orders
ik guard check --json
```

`ik guard check` is case-insensitive, strips prose comments (so developer comments never false-positive, while code lines with trailing inline-ignore pragmas still fail), and scans across wrapped multi-line SQL and ORM statements. Exemptions belong explicitly in `allow_paths` inside `.ik/guard.toml`.

---

## 4. Incremental Replacement Checklist

Before:

```python
def reject(job):
    job.attempts += 1
    job.status = "exhausted" if job.attempts >= job.max_attempts else "repairable"
```

After moving that boundary to Desmo:

```python
job = jobs.show(job_id)
job = job.reject()
```

Leave unmigrated boundaries in the host until their cut; as soon as Desmo owns a command, delete the host arithmetic and add its legacy write patterns to `.ik/guard.toml`.
