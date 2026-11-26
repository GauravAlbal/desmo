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
import { execFileSync } from "node:child_process";
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

// Background catch-up projector using monotonic `ik events` cursor:
export async function syncReadModel(lastCursor: number): Promise<number> {
  const raw = execFileSync("ik", ["events", ".state/orders.db", "--since", String(lastCursor), "--json"], { encoding: "utf8" });
  const feed = JSON.parse(raw) as { next_cursor: number; events: Array<{ aggregate_id: string; is_latest_for_aggregate: boolean; current_projection: { state: { state: string }; generation: number } }> };
  for (const ev of feed.events) {
    if (ev.is_latest_for_aggregate) {
      await projectRow(ev.aggregate_id, ev.current_projection.state.state, ev.current_projection.generation);
    }
  }
  return feed.next_cursor;
}
```

### Python (FastAPI + Desmo + Read-Model Projector)

```python
import json, subprocess
from fastapi import FastAPI, HTTPException, Header
from invariant_kernel import Kernel, IllegalStateError

app = FastAPI()
orders = Kernel(".state/orders.db").machine("order.ik.toml")

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
    out = subprocess.check_output(["ik", "events", ".state/orders.db", "--since", str(last_cursor), "--json"], text=True)
    feed = json.loads(out)
    for ev in feed["events"]:
        if ev["is_latest_for_aggregate"]:
            proj = ev["current_projection"]
            upsert_read_model(ev["aggregate_id"], proj["state"]["state"], proj["generation"])
    return int(feed["next_cursor"])
```

### Go (`net/http` + Desmo + `database/sql` Projector)

```go
package main

import (
	"context"
	"encoding/json"
	"net/http"
	"os/exec"
	"strconv"

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

func syncReadModel(ctx context.Context, lastCursor uint64, upsert func(id, state string, gen uint64) error) (uint64, error) {
	out, err := exec.CommandContext(ctx, "ik", "events", ".state/orders.db", "--since", strconv.FormatUint(lastCursor, 10), "--json").Output()
	if err != nil {
		return lastCursor, err
	}
	var feed struct {
		NextCursor uint64 `json:"next_cursor"`
		Events     []struct {
			AggregateID       string `json:"aggregate_id"`
			IsLatest          bool   `json:"is_latest_for_aggregate"`
			CurrentProjection struct {
				Generation uint64         `json:"generation"`
				State      map[string]any `json:"state"`
			} `json:"current_projection"`
		} `json:"events"`
	}
	if err := json.Unmarshal(out, &feed); err != nil {
		return lastCursor, err
	}
	for _, ev := range feed.Events {
		if ev.IsLatest {
			if st, ok := ev.CurrentProjection.State["state"].(string); ok {
				_ = upsert(ev.AggregateID, st, ev.CurrentProjection.Generation)
			}
		}
	}
	return feed.NextCursor, nil
}
```

---

## 3. Locking Down Bypass Writes in CI (`ik guard`)

After moving status transitions into Desmo, prevent accidental regressions or coding-agent shortcuts by adding `.ik/guard.toml`:

```sh
ik guard init
ik guard check --json
```

Add `ik guard check` to your CI workflow. Exemptions are explicit in `allow_paths` inside `.ik/guard.toml` (inline comment ignores inside source files are not honored).

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
