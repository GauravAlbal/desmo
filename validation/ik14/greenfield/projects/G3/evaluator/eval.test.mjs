import test from "node:test";
import fs from "node:fs/promises";
import path from "node:path";
import { withFixture, expectOk, expectErr, killed, assertStatus, waitUntil } from "./harness.mjs";

const includeChange = process.env.EVAL_INCLUDE_CHANGE === "1";

test("R1 accepted import survives a new process and stays in its state directory", async () => {
  await withFixture(async (fx) => {
    const submitted = expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    assertStatus(submitted, {
      import_id: "imp1",
      phase: "ready",
      dispatch: "none",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    });
    let snap = await fx.state();
    if (snap.requests.length !== 0) throw new Error("submit contacted the destination");
    const again = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(again, {
      import_id: "imp1",
      phase: "ready",
      dispatch: "none",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    });
    snap = await fx.state();
    if (snap.requests.length !== 0) throw new Error("status contacted the destination");
    const other = path.join(fx.dir, "other-state");
    await fs.mkdir(other);
    expectErr(await fx.cmd(["status", "imp1"], { STATE_DIR: other }), "not_found");
  });
});

test("R2 invalid input and unknown commands are refused without a destination call", async () => {
  await withFixture(async (fx) => {
    const badId = expectErr(await fx.cmd(["submit", "bad/id", "alpha"]), "invalid_input");
    if (badId.field !== "import_id") throw new Error(`field ${badId.field}`);
    const missing = expectErr(await fx.cmd(["submit", "imp1"]), "invalid_input");
    if (missing.field !== "record") throw new Error(`field ${missing.field}`);
    expectErr(await fx.cmd(["submit", "imp1", "x".repeat(257)]), "invalid_input");
    expectErr(await fx.cmd(["nope"]), "unknown_command");
    expectErr(await fx.cmd(["status", "missing"]), "not_found");
    const snap = await fx.state();
    if (snap.requests.length !== 0) throw new Error("validation contacted the destination");
    expectErr(await fx.cmd(["status", "missing"]), "not_found");
  });
});

test("R2 missing STATE_DIR or SERVICE_URL is refused", async () => {
  await withFixture(async (fx) => {
    expectErr(await fx.cmd(["submit", "imp1", "alpha"], { STATE_DIR: null }), "invalid_input");
    expectErr(await fx.cmd(["status", "imp1"], { SERVICE_URL: null }), "invalid_input");
    const snap = await fx.state();
    if (snap.requests.length !== 0) throw new Error("missing env contacted the destination");
  });
});

test("R3 a successful import creates one destination effect and a repeat does not create another", async () => {
  await withFixture(async (fx) => {
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const delivered = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(delivered, {
      import_id: "imp1",
      phase: "delivered",
      dispatch: "confirmed",
      attempts_failed: 0,
      destination_id: delivered.destination_id,
      last_error: null,
    });
    if (typeof delivered.destination_id !== "string" || delivered.destination_id.length === 0) {
      throw new Error("destination_id missing");
    }
    const looked = await fx.lookup("imp1");
    if (looked.status !== 200 || looked.body.found !== true || looked.body.record !== "alpha") {
      throw new Error(`lookup ${JSON.stringify(looked.body)}`);
    }
    if (looked.body.destination_id !== delivered.destination_id) {
      throw new Error("destination_id does not match lookup under the import id");
    }
    let snap = await fx.state();
    if (fx.effects(snap, "alpha").length !== 1) throw new Error(`effects ${JSON.stringify(snap.effects)}`);
    const again = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(again, {
      phase: "delivered",
      dispatch: "confirmed",
      attempts_failed: 0,
      destination_id: delivered.destination_id,
      last_error: null,
    });
    snap = await fx.state();
    if (fx.effects(snap, "alpha").length !== 1) throw new Error(`repeat created another effect ${JSON.stringify(snap.effects)}`);
    const resub = expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    assertStatus(resub, { phase: "delivered", destination_id: delivered.destination_id, attempts_failed: 0 });
    snap = await fx.state();
    if (fx.effects(snap, "alpha").length !== 1) throw new Error("resubmit created an effect");
  });
});

test("R4 a confirmed rejection consumes one retry and stays retryable", async () => {
  await withFixture(async (fx) => {
    await fx.config({ reject: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const rejected = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(rejected, {
      phase: "ready",
      dispatch: "none",
      attempts_failed: 1,
      destination_id: null,
      last_error: "rejected",
    });
    const snap = await fx.state();
    if (snap.effects.length !== 0) throw new Error("rejection stored an effect");
    const looked = await fx.lookup("imp1");
    if (looked.status !== 404) throw new Error("lookup should miss after rejection");
  });
});

test("R5 three confirmed rejections exhaust the limit and the limit survives restart", async () => {
  await withFixture(async (fx) => {
    await fx.config({ reject: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    let last;
    for (let i = 1; i <= 3; i += 1) {
      last = expectOk(await fx.cmd(["advance", "imp1"]));
      assertStatus(last, {
        attempts_failed: i,
        destination_id: null,
        last_error: "rejected",
        phase: i === 3 ? "failed" : "ready",
        dispatch: "none",
      });
    }
    const before = await fx.state();
    const restarted = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(restarted, {
      phase: "failed",
      dispatch: "none",
      attempts_failed: 3,
      destination_id: null,
      last_error: "rejected",
    });
    const advanced = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(advanced, { phase: "failed", attempts_failed: 3, dispatch: "none" });
    const after = await fx.state();
    if (after.requests.length !== before.requests.length) throw new Error("exhausted advance contacted the destination");
    const resub = expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    assertStatus(resub, { phase: "failed", attempts_failed: 3, last_error: "rejected" });
  });
});

test("R5 a confirmed rejection count does not decrease across restart or a later delivery", async () => {
  await withFixture(async (fx) => {
    await fx.config({ reject: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    expectOk(await fx.cmd(["advance", "imp1"]));
    const restarted = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(restarted, { phase: "ready", attempts_failed: 1, last_error: "rejected", dispatch: "none" });
    await fx.config({ reject: false });
    const delivered = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(delivered, {
      phase: "delivered",
      dispatch: "confirmed",
      attempts_failed: 1,
      last_error: null,
    });
    if (typeof delivered.destination_id !== "string") throw new Error("missing destination_id");
  });
});

test("R6 crash before dispatch does not consume a retry and later advance still delivers", async () => {
  await withFixture(async (fx) => {
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const crashed = await fx.cmd(["advance", "imp1"], { CRASH_AT: "after-record-before-dispatch" });
    if (!killed(crashed)) throw new Error(`expected SIGKILL, got code=${crashed.code} signal=${crashed.signal} stderr=${crashed.stderr}`);
    const snap = await fx.state();
    if (snap.requests.length !== 0) throw new Error("dispatch happened before the crash boundary");
    const status = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(status, {
      phase: "ready",
      dispatch: "none",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    });
    const delivered = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(delivered, { phase: "delivered", dispatch: "confirmed", attempts_failed: 0, last_error: null });
    const after = await fx.state();
    if (fx.effects(after, "alpha").length !== 1) throw new Error("recovery did not deliver exactly once");
  });
});

test("R7 crash after dispatch and before the result is applied stays uncertain and recovers once", async () => {
  await withFixture(async (fx) => {
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const crashed = await fx.cmd(["advance", "imp1"], { CRASH_AT: "after-dispatch-before-result" });
    if (!killed(crashed)) throw new Error(`expected SIGKILL, got code=${crashed.code} signal=${crashed.signal} stderr=${crashed.stderr}`);
    const mid = await fx.state();
    if (fx.effects(mid, "alpha").length !== 1) throw new Error(`expected one stored effect, got ${JSON.stringify(mid.effects)}`);
    const status = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(status, {
      phase: "uncertain",
      dispatch: "in_flight",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    });
    const recovered = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(recovered, {
      phase: "delivered",
      dispatch: "confirmed",
      attempts_failed: 0,
      last_error: null,
    });
    const looked = await fx.lookup("imp1");
    if (looked.body?.destination_id !== recovered.destination_id) {
      throw new Error(`recovered id ${recovered.destination_id} != lookup ${looked.body?.destination_id}`);
    }
    const after = await fx.state();
    if (fx.effects(after, "alpha").length !== 1) throw new Error(`recovery duplicated the effect ${JSON.stringify(after.effects)}`);
  });
});

test("R8 a dropped reply is attributed by lookup and is not reported as not sent", async () => {
  await withFixture(async (fx) => {
    await fx.config({ drop_reply: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const result = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(result, {
      phase: "delivered",
      dispatch: "confirmed",
      attempts_failed: 0,
      last_error: null,
    });
    if (result.phase === "ready" || result.dispatch === "none") throw new Error("reported as not sent");
    const looked = await fx.lookup("imp1");
    if (looked.status !== 200 || looked.body.destination_id !== result.destination_id) {
      throw new Error(`lookup did not attribute the submission: ${JSON.stringify(looked.body)}`);
    }
    const snap = await fx.state();
    if (fx.effects(snap, "alpha").length !== 1) throw new Error(`effect count ${JSON.stringify(snap.effects)}`);
    const posts = fx.posts(snap, "alpha");
    if (posts.some((p) => p.idempotency_key !== "imp1")) throw new Error(`retried under another key ${JSON.stringify(posts)}`);
  });
});

test("R9 a conflicting rejection is not a confirmed failure when lookup shows the record", async () => {
  await withFixture(async (fx) => {
    await fx.config({ late_conflict: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const result = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(result, {
      phase: "delivered",
      dispatch: "confirmed",
      attempts_failed: 0,
      last_error: null,
    });
    const snap = await fx.state();
    if (fx.effects(snap, "alpha").length !== 1) throw new Error("missing or duplicated effect");
  });
});

test("R9 a rejection is unfinished when lookup is unavailable and a bare 500 is not a confirmed failure", async () => {
  await withFixture(async (fx) => {
    await fx.config({ reject: true, fail_lookup: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const unfinished = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(unfinished, {
      phase: "uncertain",
      dispatch: "in_flight",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    });
    await fx.config({ force_status: 500 });
    expectOk(await fx.cmd(["submit", "imp2", "beta"]));
    const errored = expectOk(await fx.cmd(["advance", "imp2"]));
    assertStatus(errored, {
      phase: "uncertain",
      dispatch: "in_flight",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    });
    const snap = await fx.state();
    if (fx.effects(snap, "beta").length !== 0) throw new Error("500 stored an effect");
  });
});

test("R10 a late conflicting reply does not overwrite a newer delivered outcome", async () => {
  await withFixture(async (fx) => {
    await fx.config({ hold: true, hold_key: "imp1", late_conflict: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const first = fx.cmd(["advance", "imp1"]);
    await waitUntil(async () => {
      const snap = await fx.state();
      return fx.posts(snap, "alpha").length >= 1 && fx.effects(snap, "alpha").length === 1;
    });
    const newer = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(newer, { phase: "delivered", dispatch: "confirmed", attempts_failed: 0, last_error: null });
    await fx.release();
    const finished = expectOk(await first);
    if (finished.phase !== "delivered" || finished.attempts_failed !== 0) {
      throw new Error(`late reply overwrote newer state: ${JSON.stringify(finished)}`);
    }
    const status = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(status, {
      phase: "delivered",
      dispatch: "confirmed",
      attempts_failed: 0,
      destination_id: newer.destination_id,
      last_error: null,
    });
    const looked = await fx.lookup("imp1");
    if (looked.status !== 200 || looked.body.found !== true || looked.body.record !== "alpha") {
      throw new Error(`original delivery was not kept: ${JSON.stringify(looked.body)}`);
    }
  });
});

test("R11 legitimate work is not refused and an earlier rejection does not block delivery", async () => {
  await withFixture(async (fx) => {
    await fx.config({ reject: true });
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const rejected = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(rejected, { phase: "ready", attempts_failed: 1, last_error: "rejected" });
    await fx.config({ reject: false });
    const delivered = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(delivered, { phase: "delivered", dispatch: "confirmed", last_error: null });
    expectOk(await fx.cmd(["submit", "imp2", "beta"]));
    const second = expectOk(await fx.cmd(["advance", "imp2"]));
    assertStatus(second, { phase: "delivered", dispatch: "confirmed", attempts_failed: 0, last_error: null });
    const snap = await fx.state();
    if (fx.effects(snap, "alpha").length !== 1 || fx.effects(snap, "beta").length !== 1) {
      throw new Error(`progress effects ${JSON.stringify(snap.effects)}`);
    }
  });
});

test("R12 a second process sees committed state and can read status during an in-flight call", async () => {
  await withFixture(async (fx) => {
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const other = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(other, { phase: "ready", attempts_failed: 0, dispatch: "none" });
    await fx.config({ reject: true });
    expectOk(await fx.cmd(["advance", "imp1"]));
    const seen = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(seen, { attempts_failed: 1, phase: "ready", last_error: "rejected", dispatch: "none" });
    await fx.config({ hold: true, hold_key: "imp2", reject: false });
    expectOk(await fx.cmd(["submit", "imp2", "beta"]));
    const inflight = fx.cmd(["advance", "imp2"]);
    await waitUntil(async () => fx.posts(await fx.state(), "beta").length >= 1);
    const during = expectOk(await fx.cmd(["status", "imp2"]));
    assertStatus(during, { phase: "uncertain", dispatch: "in_flight", attempts_failed: 0 });
    await fx.release();
    expectOk(await inflight);
  });
});

test("R13 resubmit keeps the original record and a destination conflict does not create another effect", async () => {
  await withFixture(async (fx) => {
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const beforeConflict = await fx.state();
    expectErr(await fx.cmd(["submit", "imp1", "beta"]), "conflict");
    const afterConflict = await fx.state();
    if (afterConflict.requests.length !== beforeConflict.requests.length) {
      throw new Error("conflicting resubmit contacted the destination");
    }
    const unchanged = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(unchanged, { phase: "ready", attempts_failed: 0, dispatch: "none" });
    await fx.seed("imp2", "seeded");
    expectOk(await fx.cmd(["submit", "imp2", "other"]));
    const conflicted = expectOk(await fx.cmd(["advance", "imp2"]));
    assertStatus(conflicted, {
      phase: "failed",
      dispatch: "none",
      attempts_failed: 0,
      destination_id: null,
      last_error: "conflict",
    });
    const before = await fx.state();
    if (before.effects.length !== 1) throw new Error(`unexpected effects ${JSON.stringify(before.effects)}`);
    if (fx.effects(before, "other").length !== 0) throw new Error("conflict stored the import record");
    const again = expectOk(await fx.cmd(["advance", "imp2"]));
    assertStatus(again, { phase: "failed", attempts_failed: 0, last_error: "conflict" });
    const after = await fx.state();
    if (after.effects.length !== 1) throw new Error("later advance created an effect");
    if (fx.posts(after, "other").length > 1) throw new Error("conflict was retried with the import record");
  });
});

test("R14 crash after the result is read and before it is committed does not lose the single effect", async () => {
  await withFixture(async (fx) => {
    expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
    const crashed = await fx.cmd(["advance", "imp1"], { CRASH_AT: "after-result-before-commit" });
    if (!killed(crashed)) throw new Error(`expected SIGKILL, got code=${crashed.code} signal=${crashed.signal} stderr=${crashed.stderr}`);
    const status = expectOk(await fx.cmd(["status", "imp1"]));
    assertStatus(status, {
      phase: "uncertain",
      dispatch: "in_flight",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    });
    const mid = await fx.state();
    if (fx.effects(mid, "alpha").length !== 1) throw new Error("effect missing after result was read");
    const recovered = expectOk(await fx.cmd(["advance", "imp1"]));
    assertStatus(recovered, { phase: "delivered", dispatch: "confirmed", attempts_failed: 0, last_error: null });
    const after = await fx.state();
    if (fx.effects(after, "alpha").length !== 1) throw new Error("recovery created a second effect");
    const looked = await fx.lookup("imp1");
    if (looked.body?.destination_id !== recovered.destination_id) throw new Error("lookup id mismatch");
  });
});

if (includeChange) {
  test("R15 pause blocks a new dispatch and resume continues with the same retry count", async () => {
    await withFixture(async (fx) => {
      expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
      const paused = expectOk(await fx.cmd(["pause", "imp1"]));
      assertStatus(paused, { phase: "paused", dispatch: "none", attempts_failed: 0, destination_id: null, last_error: null });
      const again = expectOk(await fx.cmd(["pause", "imp1"]));
      assertStatus(again, { phase: "paused", dispatch: "none", attempts_failed: 0 });
      const blocked = expectOk(await fx.cmd(["advance", "imp1"]));
      assertStatus(blocked, { phase: "paused", dispatch: "none", attempts_failed: 0 });
      const snap = await fx.state();
      if (snap.requests.length !== 0) throw new Error("paused advance contacted the destination");
      expectErr(await fx.cmd(["resume", "missing"]), "not_found");
      expectErr(await fx.cmd(["resume", "imp2"]), "not_found");
      expectOk(await fx.cmd(["submit", "imp2", "beta"]));
      expectErr(await fx.cmd(["resume", "imp2"]), "not_paused");
      const resumed = expectOk(await fx.cmd(["resume", "imp1"]));
      assertStatus(resumed, { phase: "ready", dispatch: "none", attempts_failed: 0 });
      const delivered = expectOk(await fx.cmd(["advance", "imp1"]));
      assertStatus(delivered, { phase: "delivered", dispatch: "confirmed", attempts_failed: 0, last_error: null });
    });
  });

  test("R16 pause does not undo an in-flight dispatch and advance reconciles it once", async () => {
    await withFixture(async (fx) => {
      expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
      const crashed = await fx.cmd(["advance", "imp1"], { CRASH_AT: "after-dispatch-before-result" });
      if (!killed(crashed)) throw new Error(`expected SIGKILL, got ${crashed.signal} ${crashed.code}`);
      const before = await fx.state();
      const paused = expectOk(await fx.cmd(["pause", "imp1"]));
      assertStatus(paused, {
        phase: "paused",
        dispatch: "in_flight",
        attempts_failed: 0,
        destination_id: null,
        last_error: null,
      });
      if (paused.dispatch === "none" || paused.phase === "ready") throw new Error("in-flight work reported as not sent");
      const mid = await fx.state();
      if (fx.posts(mid, "alpha").length !== fx.posts(before, "alpha").length) throw new Error("pause contacted the destination");
      const resumed = expectOk(await fx.cmd(["resume", "imp1"]));
      assertStatus(resumed, { phase: "uncertain", dispatch: "in_flight", attempts_failed: 0 });
      expectOk(await fx.cmd(["pause", "imp1"]));
      const reconciled = expectOk(await fx.cmd(["advance", "imp1"]));
      assertStatus(reconciled, { phase: "delivered", dispatch: "confirmed", attempts_failed: 0, last_error: null });
      const after = await fx.state();
      if (fx.effects(after, "alpha").length !== 1) throw new Error("in-flight pause duplicated the effect");
      if (fx.posts(after, "alpha").length !== 1) throw new Error("reconciliation sent another delivery");
      const looked = await fx.lookup("imp1");
      if (looked.body?.destination_id !== reconciled.destination_id) throw new Error("lookup mismatch");
    });
  });

  test("R17 pause and resume do not restore exhausted retries", async () => {
    await withFixture(async (fx) => {
      await fx.config({ reject: true });
      expectOk(await fx.cmd(["submit", "imp1", "alpha"]));
      expectOk(await fx.cmd(["advance", "imp1"]));
      expectOk(await fx.cmd(["advance", "imp1"]));
      const paused = expectOk(await fx.cmd(["pause", "imp1"]));
      assertStatus(paused, { phase: "paused", attempts_failed: 2, last_error: "rejected", dispatch: "none" });
      const resumed = expectOk(await fx.cmd(["resume", "imp1"]));
      assertStatus(resumed, { phase: "ready", attempts_failed: 2, last_error: "rejected", dispatch: "none" });
      const failed = expectOk(await fx.cmd(["advance", "imp1"]));
      assertStatus(failed, { phase: "failed", attempts_failed: 3, last_error: "rejected", dispatch: "none" });
      expectErr(await fx.cmd(["pause", "imp1"]), "not_pausable");
      const still = expectOk(await fx.cmd(["status", "imp1"]));
      assertStatus(still, { phase: "failed", attempts_failed: 3, last_error: "rejected" });
      const restarted = expectOk(await fx.cmd(["status", "imp1"]));
      assertStatus(restarted, { phase: "failed", attempts_failed: 3 });
      const before = await fx.state();
      const advanced = expectOk(await fx.cmd(["advance", "imp1"]));
      assertStatus(advanced, { phase: "failed", attempts_failed: 3 });
      const after = await fx.state();
      if (after.requests.length !== before.requests.length) throw new Error("exhausted import was dispatched after pause refusal");
    });
  });
}
