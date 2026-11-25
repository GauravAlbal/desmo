#!/usr/bin/env python3
"""Compare direct status writes against Desmo enforcing examples/orders/order.ik.toml."""

from __future__ import annotations

import sqlite3
import tempfile
from pathlib import Path

from invariant_kernel import IllegalStateError, Kernel

MACHINE = Path(__file__).resolve().parent / "order.ik.toml"


def run_naive(root: Path) -> None:
    db = sqlite3.connect(root / "naive.db")
    db.execute("create table orders (id text primary key, status text)")
    db.executemany(
        "insert into orders values (?, ?)",
        [("1042", "cart"), ("1043", "cancelled"), ("1044", "paid")],
    )
    # Direct status writes let every illegal state slip through:
    db.execute("update orders set status = 'refunded' where id = '1042'")
    db.execute("update orders set status = 'delivered' where id = '1043'")
    db.execute("update orders set status = 'PAID' where id = '1044'")
    db.execute("insert into orders values ('1045', 'incinerated')")
    rows = [r[0] for r in db.execute("select distinct status from orders order by status")]
    print("Without Desmo (select distinct status from orders):")
    for status in rows:
        print(f"  {status}")
    print()


def run_desmo(root: Path) -> None:
    orders = Kernel(store=root / "orders.db").machine(str(MACHINE))
    print("With Desmo (order.ik.toml):")

    unpaid = orders.create("1042")
    try:
        unpaid.refund()
    except IllegalStateError as err:
        print(f"  refund(cart)       -> refused ({err.code}) · stays {orders.show('1042').state}")

    cancelled = orders.create("1043").pay(payment_ref="ch_1043").cancel()
    try:
        cancelled.deliver()
    except IllegalStateError as err:
        print(f"  deliver(cancelled) -> refused ({err.code}) · stays {orders.show('1043').state}")

    refunded = (
        orders.create("1044")
        .pay(payment_ref="ch_1044")
        .deliver()
        .send_back()
        .refund()
    )
    print(f"  valid lifecycle    -> committed ({refunded.state}, gen={refunded.generation})")

    try:
        refunded.refund()
    except IllegalStateError as err:
        print(f"  refund(refunded)   -> refused ({err.code}) · terminal")


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="desmo-orders-") as tmp:
        root = Path(tmp)
        run_naive(root)
        run_desmo(root)


if __name__ == "__main__":
    main()
