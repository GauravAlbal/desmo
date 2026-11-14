package invariantkernel_test

import (
	"context"
	"strings"
	"testing"

	invariantkernel "github.com/GauravAlbal/desmo/go"
)

func prepare(t *testing.T, a *invariantkernel.Aggregate, name string, args map[string]any) *invariantkernel.Invocation {
	t.Helper()
	inv, err := a.Prepare("", name, args)
	if err != nil {
		t.Fatal(err)
	}
	return inv
}

func TestInvocationCrashAfterCommitIsReconciledFromPersistedForm(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "job-1")
	if err != nil {
		t.Fatal(err)
	}
	inv := prepare(t, created, "start", nil)
	persisted := inv.JSON()
	started, historical, err := machine.Dispatch(ctx, inv)
	if err != nil || started == nil || historical != nil {
		t.Fatalf("dispatch: %v %v %v", started, historical, err)
	}
	if _, _, err := started.CommandWithOperation(ctx, "", "cancel", nil); err != nil {
		t.Fatal(err)
	}
	restored, err := invariantkernel.InvocationFromJSON(persisted)
	if err != nil {
		t.Fatal(err)
	}
	lookup, err := machine.Reconcile(ctx, restored)
	if err != nil || lookup.Outcome != "committed_exact" || lookup.History == nil {
		t.Fatalf("lookup = %+v err=%v", lookup, err)
	}
	if lookup.History.Generation != 1 || lookup.History.CurrentGeneration != 2 {
		t.Fatalf("history = %+v", lookup.History)
	}
	again, historical, err := machine.Dispatch(ctx, restored)
	if err != nil || again != nil || historical == nil || historical.OperationID != inv.OperationID() {
		t.Fatalf("repeat must be history: %v %v %v", again, historical, err)
	}
}

func TestInvocationNeverSentIsNotRecorded(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "job-2")
	if err != nil {
		t.Fatal(err)
	}
	lookup, err := machine.Reconcile(ctx, prepare(t, created, "start", nil))
	if err != nil || lookup.Outcome != "not_recorded" {
		t.Fatalf("lookup = %+v err=%v", lookup, err)
	}
}

func TestInvocationIsASnapshotOfWhatWasPersisted(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "job-4")
	if err != nil {
		t.Fatal(err)
	}
	started, _, err := created.CommandWithOperation(ctx, "", "start", nil)
	if err != nil {
		t.Fatal(err)
	}
	args := map[string]any{"artifact_digest": "AMBER", "baseline_digest": "b"}
	inv := prepare(t, started, "submit", args)
	persisted := string(inv.JSON())

	args["artifact_digest"] = "BLUE" // ordinary reuse of a caller-owned map
	inv.Command()["args"].(map[string]any)["artifact_digest"] = "GREEN"
	inv.JSON()[0] = 'X'
	if string(inv.JSON()) != persisted {
		t.Fatalf("snapshot changed: %s", inv.JSON())
	}

	sent, _, err := machine.Dispatch(ctx, inv)
	if err != nil || sent == nil {
		t.Fatalf("dispatch: %v %v", sent, err)
	}
	if _, _, err := sent.CommandWithOperation(ctx, "", "cancel", nil); err != nil {
		t.Fatal(err)
	}
	restored, err := invariantkernel.InvocationFromJSON([]byte(persisted))
	if err != nil {
		t.Fatal(err)
	}
	lookup, err := machine.Reconcile(ctx, restored)
	if err != nil || lookup.Outcome != "committed_exact" {
		t.Fatalf("lookup = %+v err=%v", lookup, err)
	}
}

func TestPrepareRefusesAnUnserializableRequest(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "job-5")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := created.Prepare("", "start", map[string]any{"f": func() {}}); err == nil {
		t.Fatal("expected an error")
	}
}

func TestInvocationForAnotherMachineIsRefused(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "job-3")
	if err != nil {
		t.Fatal(err)
	}
	text := strings.Replace(string(prepare(t, created, "start", nil).JSON()), `"machine_id":"`, `"machine_id":"other-`, 1)
	foreign, err := invariantkernel.InvocationFromJSON([]byte(text))
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := machine.Dispatch(ctx, foreign); err == nil || !strings.Contains(err.Error(), "different machine identity") {
		t.Fatalf("dispatch err = %v", err)
	}
	if _, err := machine.Reconcile(ctx, foreign); err == nil {
		t.Fatal("reconcile must refuse a foreign identity")
	}
}

const goodInvocation = `{"aggregate_id":"a","capability":{"generation":0,"nonce":"n"},"command":{"name":"start"},` +
	`"invocation_version":1,"kind":"command","machine":{"machine_id":"m","spec_hash":"h","version":1},"operation_id":"op"}`

func TestInvocationWireRefusesBadInput(t *testing.T) {
	if _, err := invariantkernel.InvocationFromJSON([]byte(goodInvocation)); err != nil {
		t.Fatal(err)
	}
	for name, text := range map[string]string{
		"version":  strings.Replace(goodInvocation, `"invocation_version":1`, `"invocation_version":2`, 1),
		"unknown":  strings.Replace(goodInvocation, `"kind":"command"`, `"kind":"command","surprise":1`, 1),
		"empty id": strings.Replace(goodInvocation, `"operation_id":"op"`, `"operation_id":""`, 1),
		"kind":     strings.Replace(goodInvocation, `"kind":"command"`, `"kind":"transaction"`, 1),
	} {
		if _, err := invariantkernel.InvocationFromJSON([]byte(text)); err == nil {
			t.Errorf("%s: expected an error", name)
		}
	}
}

// The same corpus is restored by the Python and TypeScript clients: non-ASCII text,
// HTML-sensitive characters, nesting and an integer beyond 2^53 must survive exactly.
const corpus = `{"aggregate_id":"a","capability":{"generation":0,"nonce":"n"},"command":{"args":{"label":"\u00e9<&\u2603","n":9007199254740993,"nested":{"a":[1,{"b":"x"}]}},"name":"start"},"invocation_version":1,"kind":"command","machine":{"machine_id":"m","spec_hash":"h","version":1},"operation_id":"op"}`

func TestSharedCorpusRestoresExactlyAndSurvivesASecondRoundTrip(t *testing.T) {
	first, err := invariantkernel.InvocationFromJSON([]byte(corpus))
	if err != nil {
		t.Fatal(err)
	}
	args := first.Command()["args"].(map[string]any)
	if args["label"] != "é<&☃" || args["n"].(interface{ String() string }).String() != "9007199254740993" {
		t.Fatalf("args = %v", args)
	}
	again, err := invariantkernel.InvocationFromJSON(first.JSON())
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(again.JSON()), "9007199254740993") || again.OperationID() != "op" {
		t.Fatalf("second round trip = %s", again.JSON())
	}
}
