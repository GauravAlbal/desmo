package invariantkernel_test

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	invariantkernel "github.com/GauravAlbal/desmo/go"
)

func repoRoot(t *testing.T) string {
	t.Helper()
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	root, err := invariantkernel.RepoRoot(wd)
	if err != nil {
		t.Fatal(err)
	}
	return root
}

func ikBin(t *testing.T) string {
	t.Helper()
	if env := os.Getenv("IK_BINARY"); env != "" {
		return env
	}
	bin := filepath.Join(repoRoot(t), "target", "debug", "ik")
	if _, err := os.Stat(bin); err == nil {
		return bin
	}
	cmd := exec.Command("cargo", "build", "--bin", "ik", "--offline")
	cmd.Dir = repoRoot(t)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("cargo build ik: %v\n%s", err, out)
	}
	return bin
}

func open(t *testing.T) (*invariantkernel.Kernel, *invariantkernel.Machine) {
	t.Helper()
	store := filepath.Join(t.TempDir(), "kernel.db")
	k, err := invariantkernel.Open(store, invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	m, err := k.Machine(context.Background(), filepath.Join(repoRoot(t), "machines", "verification_attempt.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	return k, m
}

func TestCreateAndHappyLifecycle(t *testing.T) {
	_, m := open(t)
	ctx := context.Background()
	job, err := m.Create(ctx, "attempt-123")
	if err != nil {
		t.Fatal(err)
	}
	if job.State() != "created" || job.Generation != 0 {
		t.Fatalf("create: state=%v gen=%d", job.State(), job.Generation)
	}
	if job, err = job.Command(ctx, "start", nil); err != nil {
		t.Fatal(err)
	}
	if job, err = job.Command(ctx, "submit", map[string]any{
		"artifact_digest": "abc", "baseline_digest": "def",
	}); err != nil {
		t.Fatal(err)
	}
	if job, err = job.Command(ctx, "verify", map[string]any{
		"receipt_digest": "r1", "receipt_artifact_digest": "abc", "receipt_baseline_digest": "def",
	}); err != nil {
		t.Fatal(err)
	}
	if job.State() != "verified" || !job.Terminal || job.Generation != 3 {
		t.Fatalf("verify: state=%v terminal=%v gen=%d", job.State(), job.Terminal, job.Generation)
	}
}

func TestStaleCopiedHandleRefuses(t *testing.T) {
	_, m := open(t)
	ctx := context.Background()
	old, err := m.Create(ctx, "job")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := old.Command(ctx, "start", nil); err != nil {
		t.Fatal(err)
	}
	_, err = old.Command(ctx, "submit", map[string]any{"artifact_digest": "a", "baseline_digest": "b"})
	var stale *invariantkernel.StaleCapabilityError
	if !errors.As(err, &stale) || stale.Code != "IK_CAPABILITY_STALE" || stale.Rule == "" {
		t.Fatalf("want StaleCapabilityError, got %v", err)
	}
}

func TestMalformedCommandRefuses(t *testing.T) {
	_, m := open(t)
	ctx := context.Background()
	job, err := m.Create(ctx, "job")
	if err != nil {
		t.Fatal(err)
	}
	if job, err = job.Command(ctx, "start", nil); err != nil {
		t.Fatal(err)
	}
	_, err = job.Command(ctx, "submit", nil)
	var invalid *invariantkernel.CommandInvalidError
	if !errors.As(err, &invalid) || invalid.Code != "IK_COMMAND_INVALID" {
		t.Fatalf("want CommandInvalidError, got %v", err)
	}
}

func TestTerminalMutationRefuses(t *testing.T) {
	_, m := open(t)
	ctx := context.Background()
	job, err := m.Create(ctx, "job")
	if err != nil {
		t.Fatal(err)
	}
	if job, err = job.Command(ctx, "start", nil); err != nil {
		t.Fatal(err)
	}
	if job, err = job.Command(ctx, "submit", map[string]any{"artifact_digest": "a", "baseline_digest": "b"}); err != nil {
		t.Fatal(err)
	}
	if job, err = job.Command(ctx, "verify", map[string]any{
		"receipt_digest": "r", "receipt_artifact_digest": "a", "receipt_baseline_digest": "b",
	}); err != nil {
		t.Fatal(err)
	}
	_, err = job.Command(ctx, "cancel", nil)
	var illegal *invariantkernel.IllegalStateError
	if !errors.As(err, &illegal) || illegal.Code != "IK_ILLEGAL_STATE" {
		t.Fatalf("want IllegalStateError, got %v", err)
	}
}

func TestEffectPlanRecordDuplicateConflict(t *testing.T) {
	k, err := invariantkernel.Open(filepath.Join(t.TempDir(), "kernel.db"), invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	m, err := k.Machine(ctx, filepath.Join(repoRoot(t), "machines", "effect_probe.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	agg, err := m.Create(ctx, "fx")
	if err != nil {
		t.Fatal(err)
	}
	if agg, err = agg.Command(ctx, "plan", map[string]any{
		"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1",
	}); err != nil {
		t.Fatal(err)
	}
	if agg.Effect.Status != "pending" {
		t.Fatalf("plan status %q", agg.Effect.Status)
	}
	recorded, err := agg.Record(ctx, "o1", nil)
	if err != nil {
		t.Fatal(err)
	}
	again, err := recorded.Record(ctx, "o1", nil)
	if err != nil {
		t.Fatal(err)
	}
	if again.Mutation == nil || *again.Mutation {
		t.Fatalf("duplicate should not mutate: %+v", again.Mutation)
	}
	_, err = again.Record(ctx, "other", nil)
	var conflict *invariantkernel.EffectConflictError
	if !errors.As(err, &conflict) || conflict.Code != "IK_EFFECT_OUTCOME_CONFLICT" {
		t.Fatalf("want EffectConflictError, got %v", err)
	}
}

func TestShowReplayVerify(t *testing.T) {
	_, m := open(t)
	ctx := context.Background()
	created, err := m.Create(ctx, "shown")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := created.Command(ctx, "start", nil); err != nil {
		t.Fatal(err)
	}
	shown, err := m.Show(ctx, "shown")
	if err != nil {
		t.Fatal(err)
	}
	if shown.State() != "running" || shown.Generation != 1 {
		t.Fatalf("show: state=%v gen=%d", shown.State(), shown.Generation)
	}
	replayed, err := m.Replay(ctx, "shown")
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := replayed["aggregates"]; !ok {
		t.Fatalf("replay missing aggregates: %v", replayed)
	}
	verified, err := m.Verify(ctx, "shown")
	if err != nil {
		t.Fatal(err)
	}
	if verified["verified"] != true {
		t.Fatalf("verify: %v", verified)
	}
}

func TestProtocolMismatch(t *testing.T) {
	k, m := open(t)
	k.SetProtocolForTest(2, 0)
	_, err := m.Create(context.Background(), "job")
	var proto *invariantkernel.ProtocolError
	if !errors.As(err, &proto) || proto.Code != "IK_PROTOCOL_VERSION_UNKNOWN" {
		t.Fatalf("want ProtocolError, got %v", err)
	}
}
func TestTransportFailureIsNotDomainDiagnostic(t *testing.T) {
	k, err := invariantkernel.Open(filepath.Join(t.TempDir(), "s.db"), invariantkernel.Options{
		Binary: "/no/such/ik-binary",
	})
	if err != nil {
		var te *invariantkernel.TransportError
		if !errors.As(err, &te) {
			t.Fatalf("want TransportError, got %v", err)
		}
		return
	}
	_, err = k.Machine(context.Background(), "machines/verification_attempt.ik.toml")
	var te *invariantkernel.TransportError
	if !errors.As(err, &te) {
		t.Fatalf("want TransportError, got %v", err)
	}
}

func TestGoMatchesDirectCLI(t *testing.T) {
	root := repoRoot(t)
	bin := ikBin(t)
	va := filepath.Join(root, "machines", "verification_attempt.ik.toml")
	k, err := invariantkernel.Open(filepath.Join(t.TempDir(), "go.db"), invariantkernel.Options{Binary: bin})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	m, err := k.Machine(ctx, va)
	if err != nil {
		t.Fatal(err)
	}
	job, err := m.Create(ctx, "agg-1")
	if err != nil {
		t.Fatal(err)
	}
	if job, err = job.Command(ctx, "start", nil); err != nil {
		t.Fatal(err)
	}

	identOut, err := exec.Command(bin, "identity", va).Output()
	if err != nil {
		t.Fatal(err)
	}
	var identity map[string]any
	if err := json.Unmarshal(identOut, &identity); err != nil {
		t.Fatal(err)
	}
	cliStore := filepath.Join(t.TempDir(), "cli.db")
	created := runCLI(t, bin, cliStore, va, "create", map[string]any{
		"protocol": map[string]any{"major": 1, "minor": 0}, "runtime_version": "0.7.0",
		"request_id": "c", "op": "create", "aggregate_id": "agg-1", "machine": identity,
	})
	started := runCLI(t, bin, cliStore, va, "command", map[string]any{
		"protocol": map[string]any{"major": 1, "minor": 0}, "runtime_version": "0.7.0",
		"request_id": "s", "op": "command", "aggregate_id": "agg-1", "machine": identity,
		"capability": created["result"].(map[string]any)["capability"],
		"command":    map[string]any{"name": "start"},
	})
	cli := started["result"].(map[string]any)
	if job.State() != cli["state"].(map[string]any)["state"] || job.Generation != asUint(cli["generation"]) {
		t.Fatalf("parity mismatch go=%v/%d cli=%v", job.State(), job.Generation, cli)
	}
}

func TestCheckWrapper(t *testing.T) {
	k, _ := open(t)
	root := repoRoot(t)
	res, err := k.Check(
		context.Background(),
		filepath.Join(root, "machines", "verification_attempt.ik.toml"),
		"", "",
		filepath.Join(root, "validation", "verification_baseline.json"),
	)
	if err != nil {
		t.Fatal(err)
	}
	if res.Verdict != "accept" && res.Verdict != "refused" {
		t.Fatalf("verdict %q", res.Verdict)
	}
}

func runCLI(t *testing.T, bin, store, machine, op string, req map[string]any) map[string]any {
	t.Helper()
	payload, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(bin, "--store", store, "--machine", machine, op)
	cmd.Stdin = bytes.NewReader(payload)
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("cli %s: %v\n%s", op, err, out)
	}
	var env map[string]any
	if err := json.Unmarshal(out, &env); err != nil {
		t.Fatal(err)
	}
	return env
}

func asUint(v any) uint64 {
	switch n := v.(type) {
	case json.Number:
		if i, err := n.Int64(); err == nil && i >= 0 {
			return uint64(i)
		}
		return 0
	case float64:
		return uint64(n)
	default:
		return 0
	}
}

// TestInvokeNilOmitsMachine is the Bind+Invoke escape hatch: a bound Machine
// injects `machine` only when passed; nil is an unbound extension envelope.
func TestInvokeNilOmitsMachine(t *testing.T) {
	dir := t.TempDir()
	capture := filepath.Join(dir, "req.json")
	bin := writeCaptureBin(t, capture)
	k, err := invariantkernel.Open(filepath.Join(dir, "store.db"), invariantkernel.Options{Binary: bin})
	if err != nil {
		t.Fatal(err)
	}
	m := k.Bind(invariantkernel.NewIdentity("demo.machine", 1, "abc"))
	ctx := context.Background()
	if _, err := k.Invoke(ctx, "extension", m, map[string]any{"aggregate_id": "agg"}); err != nil {
		t.Fatalf("bound Invoke: %v", err)
	}
	bound := readCaptured(t, capture)
	if bound["machine"] == nil {
		t.Fatalf("Invoke(op, machine, fields) must carry machine: %v", bound)
	}
	if _, err := k.Invoke(ctx, "extension", nil, map[string]any{"aggregate_id": "agg"}); err != nil {
		t.Fatalf("nil Invoke: %v", err)
	}
	unbound := readCaptured(t, capture)
	if _, ok := unbound["machine"]; ok {
		t.Fatalf("Invoke(op, nil, fields) must omit machine: %v", unbound)
	}
	if unbound["op"] != "extension" || unbound["aggregate_id"] != "agg" {
		t.Fatalf("unbound envelope = %v", unbound)
	}
}

func writeCaptureBin(t *testing.T, capture string) string {
	t.Helper()
	bin := filepath.Join(t.TempDir(), "ik-capture")
	script := "#!/bin/sh\n" +
		"if [ \"$1\" = \"--version\" ]; then echo 'ik 0.7.0 protocol 1.0'; exit 0; fi\n" +
		"cat > '" + capture + "'\n" +
		"echo '{\"ok\":true,\"result\":{\"recorded\":true}}'\n"
	if err := os.WriteFile(bin, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return bin
}

func readCaptured(t *testing.T, path string) map[string]any {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var env map[string]any
	if err := json.Unmarshal(raw, &env); err != nil {
		t.Fatalf("captured envelope is not JSON: %v\n%s", err, raw)
	}
	return env
}

func TestOwnerNamesTheDefinitionThatOwnsAnOldAggregate(t *testing.T) {
	ctx := context.Background()
	store := filepath.Join(t.TempDir(), "kernel.db")
	k, err := invariantkernel.Open(store, invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	machines := filepath.Join(repoRoot(t), "machines")
	v1, err := k.Machine(ctx, filepath.Join(machines, "publish_approval.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	v2, err := k.Machine(ctx, filepath.Join(machines, "publish_approval_v2.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := v1.Create(ctx, "legacy-doc"); err != nil {
		t.Fatal(err)
	}
	if _, err := v2.Create(ctx, "fresh-doc"); err != nil {
		t.Fatal(err)
	}
	legacy, err := v2.Show(ctx, "legacy-doc")
	if err != nil {
		t.Fatal(err)
	}
	if legacy.Owner() != v1.Identity || legacy.Owner() == v2.Identity {
		t.Fatalf("legacy owner %+v want v1 %+v not v2 %+v", legacy.Owner(), v1.Identity, v2.Identity)
	}
	fresh, err := v2.Show(ctx, "fresh-doc")
	if err != nil {
		t.Fatal(err)
	}
	if fresh.Owner() != v2.Identity {
		t.Fatalf("fresh owner %+v want %+v", fresh.Owner(), v2.Identity)
	}
}

func TestTextInIdentifierGuard(t *testing.T) {
	k, err := invariantkernel.Open(filepath.Join(t.TempDir(), "kernel.db"), invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	m, err := k.Machine(ctx, filepath.Join(repoRoot(t), "examples", "shipped", "tagged_item.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	item, err := m.Create(ctx, "item-1")
	if err != nil {
		t.Fatal(err)
	}
	tagged, err := item.Command(ctx, "tag", map[string]any{"id": "item_1", "label": "AMBER"})
	if err != nil {
		t.Fatal(err)
	}
	if tagged.State() != "tagged" || !tagged.Terminal {
		t.Fatalf("tag: state=%v terminal=%v", tagged.State(), tagged.Terminal)
	}
	empty, err := m.Create(ctx, "item-empty")
	if err != nil {
		t.Fatal(err)
	}
	_, err = empty.Command(ctx, "tag", map[string]any{"id": "", "label": "AMBER"})
	var failed *invariantkernel.InvariantFailedError
	if !errors.As(err, &failed) || failed.Code != "IK_INVARIANT_FAILED" || failed.Rule != "tag.when" {
		t.Fatalf("want InvariantFailedError tag.when, got %v", err)
	}
	shown, err := m.Show(ctx, "item-empty")
	if err != nil {
		t.Fatal(err)
	}
	if shown.State() != "idle" {
		t.Fatalf("empty id mutated state to %v", shown.State())
	}
}

func TestSessionTransportAndTransaction(t *testing.T) {
	dir := t.TempDir()
	k, err := invariantkernel.Open(filepath.Join(dir, "session.db"), invariantkernel.Options{
		Binary:  ikBin(t),
		Session: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer k.Close()
	ctx := context.Background()
	m, err := k.Machine(ctx, filepath.Join(repoRoot(t), "machines", "verification_attempt.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	agg, err := m.Create(ctx, "sess-1")
	if err != nil {
		t.Fatal(err)
	}
	if agg.StateName() != "created" {
		t.Fatalf("expected created, got %q", agg.StateName())
	}
	next, hist, err := agg.Transaction(ctx, "tx-1", []map[string]any{
		{"name": "start", "args": map[string]any{}},
		{"name": "submit", "args": map[string]any{"artifact_digest": "a1", "baseline_digest": "b1"}},
	})
	if err != nil || hist != nil {
		t.Fatalf("transaction failed: err=%v hist=%v", err, hist)
	}
	if next.StateName() != "submitted" || next.Generation != 2 {
		t.Fatalf("unexpected post-tx state=%q gen=%d", next.StateName(), next.Generation)
	}
	// Exact replay of the same operation id returns Historical, never mutating.
	again, replayHist, err := agg.Transaction(ctx, "tx-1", []map[string]any{
		{"name": "start", "args": map[string]any{}},
		{"name": "submit", "args": map[string]any{"artifact_digest": "a1", "baseline_digest": "b1"}},
	})
	if err != nil || again != nil || replayHist == nil {
		t.Fatalf("expected historical replay, got next=%v hist=%v err=%v", again, replayHist, err)
	}
}

func TestRunEffectHelper(t *testing.T) {
	dir := t.TempDir()
	k, err := invariantkernel.Open(filepath.Join(dir, "fx.db"), invariantkernel.Options{
		Binary:  ikBin(t),
		Session: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer k.Close()
	ctx := context.Background()
	m, err := k.Machine(ctx, filepath.Join(repoRoot(t), "machines", "effect_probe.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	agg, err := m.Create(ctx, "fx-go-1")
	if err != nil {
		t.Fatal(err)
	}
	var seenKey string
	res, err := agg.RunEffect(
		ctx,
		"plan",
		map[string]any{"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"},
		func(_ context.Context, intent invariantkernel.EffectIntent) (string, error) {
			seenKey = intent.IdempotencyKey
			return "succeeded", nil
		},
		"plan-go-1",
		"rec-go-1",
	)
	if err != nil {
		t.Fatalf("RunEffect failed: %v", err)
	}
	if seenKey != "k1" || res.Recorded == nil || res.Recorded.Effect.Status != "recorded" || res.Recorded.Effect.OutcomeDigest != "succeeded" {
		t.Fatalf("unexpected RunEffect result: key=%q res=%+v", seenKey, res)
	}
	// Duplicate planOpID must refuse rather than re-running executor.
	ran := false
	if _, err := agg.RunEffect(
		ctx,
		"plan",
		map[string]any{"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"},
		func(_ context.Context, _ invariantkernel.EffectIntent) (string, error) {
			ran = true
			return "succeeded", nil
		},
		"plan-go-1",
		"rec-go-2",
	); err == nil || ran {
		t.Fatalf("expected duplicate plan refusal without running executor, got err=%v ran=%v", err, ran)
	}
}

func TestLargeInt64PrecisionPreserved(t *testing.T) {
	dir := t.TempDir()
	specPath := filepath.Join(dir, "bignum.ik.toml")
	spec := `schema_version = 1
machine_id = "bignum"
machine_version = 1

[facts]
amount = { type = "int", policy = "replace" }

[states]
idle = { initial = true }
set = { terminal = true }

[commands.store]
from = ["idle"]
to = "set"
args = { amount = "int" }
then = { amount = "$amount" }
`
	if err := os.WriteFile(specPath, []byte(spec), 0o600); err != nil {
		t.Fatal(err)
	}
	k, err := invariantkernel.Open(filepath.Join(dir, "bignum.db"), invariantkernel.Options{
		Binary:  ikBin(t),
		Session: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer k.Close()
	ctx := context.Background()
	m, err := k.Machine(ctx, specPath)
	if err != nil {
		t.Fatal(err)
	}
	agg, err := m.Create(ctx, "b-1")
	if err != nil {
		t.Fatal(err)
	}
	const want int64 = 9007199254740993 // 2^53 + 1: loses precision if decoded as float64
	next, err := agg.Command(ctx, "store", map[string]any{"amount": want})
	if err != nil {
		t.Fatal(err)
	}
	facts := next.Facts()
	num, ok := facts["amount"].(json.Number)
	if !ok {
		t.Fatalf("expected json.Number for fact amount, got %T (%v)", facts["amount"], facts["amount"])
	}
	if num.String() != "9007199254740993" {
		t.Fatalf("expected exact int64 9007199254740993, got %s", num.String())
	}
}

func TestKernelEventsCursor(t *testing.T) {
	dir := t.TempDir()
	k, err := invariantkernel.Open(filepath.Join(dir, "events.db"), invariantkernel.Options{
		Binary:  ikBin(t),
		Session: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer k.Close()
	ctx := context.Background()
	m, err := k.Machine(ctx, filepath.Join(repoRoot(t), "examples", "orders", "order.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	ord1, err := m.Create(ctx, "ord-1")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ord1.Command(ctx, "pay", map[string]any{"payment_ref": "ch_1"}); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Create(ctx, "ord-2"); err != nil {
		t.Fatal(err)
	}

	page1, err := k.Events(ctx, invariantkernel.EventsOptions{Since: 0, Limit: 2})
	if err != nil {
		t.Fatal(err)
	}
	if page1.Schema != "ik.events.v1" || !page1.HasMore || len(page1.Events) != 2 {
		t.Fatalf("unexpected page1: %+v", page1)
	}
	if page1.Events[0].IsLatestForAggregate || !page1.Events[1].IsLatestForAggregate || page1.Events[1].EventType != "pay" {
		t.Fatalf("unexpected latest flags: %+v", page1.Events)
	}
	if st, _ := page1.Events[1].CurrentProjection.State["state"].(string); st != "paid" {
		t.Fatalf("expected projection state=paid, got %v", page1.Events[1].CurrentProjection.State)
	}

	page2, err := k.Events(ctx, invariantkernel.EventsOptions{Since: page1.NextCursor})
	if err != nil {
		t.Fatal(err)
	}
	if page2.HasMore || len(page2.Events) != 1 || page2.Events[0].AggregateID != "ord-2" || !page2.Events[0].IsLatestForAggregate {
		t.Fatalf("unexpected page2: %+v", page2)
	}
}

func TestMachineAndKernelList(t *testing.T) {
	dir := t.TempDir()
	k, err := invariantkernel.Open(filepath.Join(dir, ".state", "list.db"), invariantkernel.Options{
		Binary:  ikBin(t),
		Session: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer k.Close()
	ctx := context.Background()
	orders, err := k.Machine(ctx, filepath.Join(repoRoot(t), "examples", "orders", "order.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	fx, err := k.Machine(ctx, filepath.Join(repoRoot(t), "machines", "effect_probe.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}

	o1, err := orders.Create(ctx, "ord-1")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := o1.Command(ctx, "pay", map[string]any{"payment_ref": "ch_1"}); err != nil {
		t.Fatal(err)
	}
	o2, err := orders.Create(ctx, "ord-2")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := o2.Command(ctx, "cancel", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := orders.Create(ctx, "ord-3"); err != nil {
		t.Fatal(err)
	}

	f1, err := fx.Create(ctx, "fx-1")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f1.Command(ctx, "plan", map[string]any{
		"effect_id":       "e1",
		"payload_digest":  "p1",
		"idempotency_key": "k1",
	}); err != nil {
		t.Fatal(err)
	}

	p1, err := orders.List(ctx, invariantkernel.ListOptions{Limit: 2})
	if err != nil {
		t.Fatal(err)
	}
	if p1.Schema != "ik.list.v1" || !p1.HasMore || p1.NextAfter != "ord-2" || len(p1.Aggregates) != 2 {
		t.Fatalf("unexpected page1: %+v", p1)
	}

	p2, err := orders.List(ctx, invariantkernel.ListOptions{After: p1.NextAfter, Limit: 2})
	if err != nil {
		t.Fatal(err)
	}
	if p2.HasMore || len(p2.Aggregates) != 1 || p2.Aggregates[0].AggregateID != "ord-3" {
		t.Fatalf("unexpected page2: %+v", p2)
	}

	paid, err := orders.List(ctx, invariantkernel.ListOptions{State: "paid"})
	if err != nil {
		t.Fatal(err)
	}
	if len(paid.Aggregates) != 1 || paid.Aggregates[0].AggregateID != "ord-1" {
		t.Fatalf("unexpected paid: %+v", paid)
	}

	pend, err := k.List(ctx, invariantkernel.ListOptions{Effect: "pending"})
	if err != nil {
		t.Fatal(err)
	}
	if len(pend.Aggregates) != 1 || pend.Aggregates[0].AggregateID != "fx-1" || pend.Aggregates[0].Effect.Status != "pending" {
		t.Fatalf("unexpected pending: %+v", pend)
	}

	termFlag := true
	term, err := orders.List(ctx, invariantkernel.ListOptions{Terminal: &termFlag})
	if err != nil {
		t.Fatal(err)
	}
	if len(term.Aggregates) != 1 || term.Aggregates[0].AggregateID != "ord-2" {
		t.Fatalf("unexpected terminal: %+v", term)
	}
}
