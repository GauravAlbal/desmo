# Order lifecycle example

The same `order.ik.toml` definition is checked at build time and executed at runtime by `ik`:

- `cart → paid → delivered → returned → refunded`
- `cart` or `paid` → `cancelled`
- `refunded` and `cancelled` are terminal.

## Inspect, check & simulate with `ik`

```bash
ik diagram examples/orders/order.ik.toml
ik check examples/orders/order.ik.toml --corpus examples/orders/order.ik.cases.json
ik simulate examples/orders/order.ik.toml --out order-simulator.html
ik laws examples/orders/order.ik.toml
```

## Run the comparison

TypeScript / Node (from an installed `@desmo/runtime` project):

```bash
npx --no-install desmo demo orders
```

Python (with `invariant_kernel` installed and `ik` on `PATH` or `IK_BINARY`):

```bash
python3 examples/orders/demo.py
```

Both demos show an unpaid cart refusing `refund`, a cancelled order refusing `deliver`, a valid order completing `cart → paid → delivered → returned → refunded`, and a second `refund` refused with `IK_ILLEGAL_STATE`. Desmo governs durable lifecycle state; it does not move money or parcels.
