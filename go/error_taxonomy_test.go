package invariantkernel_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	invariantkernel "github.com/GauravAlbal/desmo/go"
)

func TestMissingStoreIsNotFoundAndReadsCreateNothing(t *testing.T) {
	ctx := context.Background()
	parent := filepath.Join(t.TempDir(), "absent-parent")
	store := filepath.Join(parent, "store.db")
	if _, err := os.Stat(parent); !os.IsNotExist(err) {
		t.Fatalf("parent must be absent before the read, stat=%v", err)
	}
	kernel, err := invariantkernel.Open(store, invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	machine, err := kernel.Machine(ctx, filepath.Join(repoRoot(t), "machines", "verification_attempt.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = machine.Show(ctx, "missing")
	assertStoreNotFound(t, err)
	_, err = machine.Replay(ctx, "missing")
	assertStoreNotFound(t, err)
	_, err = machine.Verify(ctx, "missing")
	assertStoreNotFound(t, err)
	_, err = machine.LookupOperation(ctx, "missing", map[string]any{"generation": 0, "nonce": "n"}, "op-1", "start", nil)
	assertStoreNotFound(t, err)
	if _, statErr := os.Stat(store); !os.IsNotExist(statErr) {
		t.Fatalf("read created a store file: %v", statErr)
	}
	if _, statErr := os.Stat(parent); !os.IsNotExist(statErr) {
		t.Fatalf("read created a parent directory: %v", statErr)
	}
}

func assertStoreNotFound(t *testing.T, err error) {
	t.Helper()
	var missing *invariantkernel.StoreNotFoundError
	if !errors.As(err, &missing) {
		t.Fatalf("want StoreNotFoundError, got %v", err)
	}
	if missing.Code != "IK_STORE_NOT_FOUND" || missing.Rule != "store.missing" {
		t.Fatalf("code=%s rule=%s", missing.Code, missing.Rule)
	}
	if missing.Diagnostic["rule"] != "store.missing" || missing.Diagnostic["code"] != "IK_STORE_NOT_FOUND" {
		t.Fatalf("diagnostic lost rule or code: %#v", missing.Diagnostic)
	}
	class, classErr := invariantkernel.Classify(err, nil)
	if classErr != nil || class != invariantkernel.ClassNotFound {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
	if class == invariantkernel.ClassCorrupt {
		t.Fatal("a missing store must not classify as corrupt")
	}
}

func TestCreateCreatesTheStoreFile(t *testing.T) {
	ctx := context.Background()
	store := filepath.Join(t.TempDir(), "created.db")
	kernel, err := invariantkernel.Open(store, invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	machine, err := kernel.Machine(ctx, filepath.Join(repoRoot(t), "machines", "verification_attempt.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(store); !os.IsNotExist(err) {
		t.Fatalf("store existed before create: %v", err)
	}
	if _, err := machine.Create(ctx, "agg-1"); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(store)
	if err != nil || info.IsDir() {
		t.Fatalf("create did not create a store file: %v", err)
	}
}

func TestGarbageStoreIsCorruptNotMissing(t *testing.T) {
	ctx := context.Background()
	store := filepath.Join(t.TempDir(), "garbage.db")
	if err := os.WriteFile(store, []byte("this is not a sqlite database"), 0o644); err != nil {
		t.Fatal(err)
	}
	kernel, err := invariantkernel.Open(store, invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	machine, err := kernel.Machine(ctx, filepath.Join(repoRoot(t), "machines", "verification_attempt.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = machine.Show(ctx, "agg-1")
	assertCorrupt(t, err)
	if _, statErr := os.Stat(store); statErr != nil {
		t.Fatal(statErr)
	}
	_, err = machine.Show(ctx, "agg-1")
	assertCorrupt(t, err)
}

func assertCorrupt(t *testing.T, err error) {
	t.Helper()
	var missing *invariantkernel.StoreNotFoundError
	if errors.As(err, &missing) {
		t.Fatalf("garbage store classified as missing: %v", err)
	}
	var kernel *invariantkernel.KernelError
	if !errors.As(err, &kernel) || kernel.Code != "IK_STORE_CORRUPT" || kernel.Rule == "store.missing" {
		t.Fatalf("want IK_STORE_CORRUPT, got %v", err)
	}
	class, classErr := invariantkernel.Classify(err, nil)
	if classErr != nil || class != invariantkernel.ClassCorrupt {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
}

func TestUnknownAggregateIsNotFoundNotCorrupt(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	if _, err := machine.Create(ctx, "present"); err != nil {
		t.Fatal(err)
	}
	_, err := machine.Show(ctx, "absent")
	var kernel *invariantkernel.KernelError
	if !errors.As(err, &kernel) || kernel.Code != "IK_ILLEGAL_STATE" || kernel.Rule != "aggregate.not_found" {
		t.Fatalf("want aggregate.not_found, got %v", err)
	}
	if kernel.Diagnostic["rule"] != "aggregate.not_found" {
		t.Fatalf("diagnostic lost the rule: %#v", kernel.Diagnostic)
	}
	class, classErr := invariantkernel.Classify(err, nil)
	if classErr != nil || class != invariantkernel.ClassNotFound {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
}

func TestStaleCapabilityIsRefused(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	started, _, err := created.CommandWithOperation(ctx, "first-op", "start", nil)
	if err != nil {
		t.Fatal(err)
	}
	_, _, err = created.CommandWithOperation(ctx, "other-op", "cancel", nil)
	var stale *invariantkernel.StaleCapabilityError
	if !errors.As(err, &stale) || stale.Code != "IK_CAPABILITY_STALE" {
		t.Fatalf("want stale capability, got %v", err)
	}
	class, classErr := invariantkernel.Classify(err, nil)
	if classErr != nil || class != invariantkernel.ClassRefused {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
	if started == nil {
		t.Fatal("start did not commit")
	}
}

func TestExactRepeatIsHistoricalNotAnError(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := created.CommandWithOperation(ctx, "same-op", "start", nil); err != nil {
		t.Fatal(err)
	}
	next, historical, err := created.CommandWithOperation(ctx, "same-op", "start", nil)
	if err != nil || next != nil || historical == nil {
		t.Fatalf("repeat next=%v historical=%v err=%v", next, historical, err)
	}
	class, classErr := invariantkernel.Classify(nil, historical)
	if classErr != nil || class != invariantkernel.ClassHistorical {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
}

func TestMissingBinaryIsNotSent(t *testing.T) {
	ctx := context.Background()
	kernel, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	kernel.Binary = filepath.Join(t.TempDir(), "no-such-ik")
	_, _, err = created.CommandWithOperation(ctx, "never-left", "start", nil)
	if !invariantkernel.IsUnsent(err) {
		t.Fatalf("want unsent, got %v", err)
	}
	class, classErr := invariantkernel.Classify(err, nil)
	if classErr != nil || class != invariantkernel.ClassNotSent {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
	if got := invariantkernel.OperationIDOf(err); got != "never-left" {
		t.Fatalf("operation id = %q", got)
	}
}

func TestTimeoutAfterDispatchIsUncertain(t *testing.T) {
	ctx := context.Background()
	kernel, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	log := filepath.Join(dir, "dispatches.log")
	stub := filepath.Join(dir, "slow-ik")
	script := "#!/bin/sh\necho dispatched >> \"" + log + "\"\nsleep 5\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	kernel.Binary = stub
	kernel.Timeout = 300 * time.Millisecond
	_, _, err = created.CommandWithOperation(ctx, "maybe-committed", "start", nil)
	var transport *invariantkernel.TransportError
	if !errors.As(err, &transport) || transport.Unsent {
		t.Fatalf("want uncertain transport, got %v", err)
	}
	class, classErr := invariantkernel.Classify(err, nil)
	if classErr != nil || class != invariantkernel.ClassUncertain {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
	logged, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Fields(string(logged)); len(got) != 1 {
		t.Fatalf("dispatches = %q", logged)
	}
}

func TestLegacyMissingStoreCodeIsStillNotFound(t *testing.T) {
	legacy := &invariantkernel.KernelError{
		Code: "IK_STORE_CORRUPT",
		Rule: "store.missing",
		Diagnostic: map[string]any{
			"code": "IK_STORE_CORRUPT",
			"rule": "store.missing",
		},
	}
	class, err := invariantkernel.Classify(legacy, nil)
	if err != nil || class != invariantkernel.ClassNotFound {
		t.Fatalf("classify=%s err=%v", class, err)
	}
}

func TestMachineMismatchIsContractMismatch(t *testing.T) {
	ctx := context.Background()
	kernel, home := open(t)
	created, err := home.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	foreign, err := kernel.Machine(ctx, filepath.Join(repoRoot(t), "machines", "effect_probe.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = kernel.Invoke(ctx, "command", foreign, map[string]any{
		"aggregate_id": created.ID,
		"capability":   created.WireCapability(),
		"command":      map[string]any{"name": "start"},
		"operation_id": "wrong-machine",
	})
	var kernelErr *invariantkernel.KernelError
	if !errors.As(err, &kernelErr) || !strings.HasPrefix(kernelErr.Code, "IK_MACHINE_") {
		t.Fatalf("want machine mismatch, got %v", err)
	}
	class, classErr := invariantkernel.Classify(err, nil)
	if classErr != nil || class != invariantkernel.ClassContractMismatch {
		t.Fatalf("classify=%s err=%v", class, classErr)
	}
}

func TestSameOperationIDOnTwoAggregatesDoesNotCollide(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	first, err := machine.Create(ctx, "agg-a")
	if err != nil {
		t.Fatal(err)
	}
	second, err := machine.Create(ctx, "agg-b")
	if err != nil {
		t.Fatal(err)
	}
	if _, historical, err := first.CommandWithOperation(ctx, "shared-op", "start", nil); err != nil || historical != nil {
		t.Fatalf("agg-a start historical=%v err=%v", historical, err)
	}
	if _, historical, err := second.CommandWithOperation(ctx, "shared-op", "start", nil); err != nil || historical != nil {
		t.Fatalf("agg-b start must not collide, historical=%v err=%v", historical, err)
	}
	lookupA, err := machine.LookupOperation(ctx, "agg-a", first.WireCapability(), "shared-op", "start", nil)
	if err != nil || lookupA.Outcome != "committed_exact" {
		t.Fatalf("agg-a lookup=%+v err=%v", lookupA, err)
	}
	lookupB, err := machine.LookupOperation(ctx, "agg-b", second.WireCapability(), "shared-op", "start", nil)
	if err != nil || lookupB.Outcome != "committed_exact" {
		t.Fatalf("agg-b lookup=%+v err=%v", lookupB, err)
	}
	_, _, err = first.CommandWithOperation(ctx, "shared-op", "cancel", nil)
	var conflict *invariantkernel.OperationConflictError
	if !errors.As(err, &conflict) {
		t.Fatalf("changed semantics on agg-a must conflict, got %v", err)
	}
	stillB, err := machine.LookupOperation(ctx, "agg-b", second.WireCapability(), "shared-op", "start", nil)
	if err != nil || stillB.Outcome != "committed_exact" {
		t.Fatalf("agg-b must be unaffected, lookup=%+v err=%v", stillB, err)
	}
}
