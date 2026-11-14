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

func TestDroppedReplyThenOtherWriterIsRecoveredByLookup(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	persisted := created.WireCapability()
	operationID := invariantkernel.NewOperationID()
	started, historical, err := created.CommandWithOperation(ctx, operationID, "start", nil)
	if err != nil || started == nil || historical != nil {
		t.Fatalf("start: started=%v historical=%v err=%v", started, historical, err)
	}
	// Another writer advances the aggregate past the original capability.
	if _, _, err := started.CommandWithOperation(ctx, "", "cancel", nil); err != nil {
		t.Fatal(err)
	}

	lookup, err := machine.LookupOperation(ctx, "attempt-1", persisted, operationID, "start", nil)
	if err != nil {
		t.Fatal(err)
	}
	if lookup.Outcome != "committed_exact" || lookup.History == nil {
		t.Fatalf("lookup = %+v", lookup)
	}
	if lookup.History.FirstSequence != 1 || lookup.History.Generation != 1 || lookup.History.CurrentGeneration != 2 {
		t.Fatalf("history = %+v", lookup.History)
	}
}

func TestExactRepeatIsHistoryNotAHandleAndDoesNotMutate(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	started, _, err := created.CommandWithOperation(ctx, "repeat-1", "start", nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := started.CommandWithOperation(ctx, "", "cancel", nil); err != nil {
		t.Fatal(err)
	}

	next, historical, err := created.CommandWithOperation(ctx, "repeat-1", "start", nil)
	if err != nil {
		t.Fatal(err)
	}
	if next != nil || historical == nil {
		t.Fatalf("repeat must be history, got next=%v historical=%v", next, historical)
	}
	if historical.OperationID != "repeat-1" || historical.CurrentGeneration != 2 {
		t.Fatalf("historical = %+v", historical)
	}
	if _, present := historical.Raw["capability"]; present {
		t.Fatal("history must carry no capability")
	}
	shown, err := machine.Show(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	if shown.Generation != 2 || shown.State() != "cancelled" {
		t.Fatalf("repeat mutated: generation=%d state=%v", shown.Generation, shown.State())
	}
}

func TestStaleHandleUnderDifferentOperationIDStillRefusesAndCarriesID(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := created.CommandWithOperation(ctx, "first-op", "start", nil); err != nil {
		t.Fatal(err)
	}
	_, _, err = created.CommandWithOperation(ctx, "different-op", "cancel", nil)
	var stale *invariantkernel.StaleCapabilityError
	if !errors.As(err, &stale) {
		t.Fatalf("want StaleCapabilityError, got %v", err)
	}
	if got := invariantkernel.OperationIDOf(err); got != "different-op" {
		t.Fatalf("operation id on error = %q", got)
	}
}

func TestSameOperationIDWithChangedSemanticsConflicts(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := created.CommandWithOperation(ctx, "reused", "start", nil); err != nil {
		t.Fatal(err)
	}
	_, _, err = created.CommandWithOperation(ctx, "reused", "cancel", nil)
	var conflict *invariantkernel.OperationConflictError
	if !errors.As(err, &conflict) || conflict.Code != "IK_OPERATION_CONFLICT" {
		t.Fatalf("want OperationConflictError, got %v", err)
	}
	if got := invariantkernel.OperationIDOf(err); got != "reused" {
		t.Fatalf("operation id on error = %q", got)
	}
	lookup, err := machine.LookupOperation(ctx, "attempt-1", created.WireCapability(), "reused", "cancel", nil)
	if err != nil || lookup.Outcome != "conflict" {
		t.Fatalf("lookup = %+v err=%v", lookup, err)
	}
}

func TestLookupOfUnsentOperationIsNotRecorded(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	lookup, err := machine.LookupOperation(ctx, "attempt-1", created.WireCapability(), "never-sent", "start", nil)
	if err != nil {
		t.Fatal(err)
	}
	if lookup.Outcome != "not_recorded" || lookup.History != nil {
		t.Fatalf("lookup = %+v", lookup)
	}
	shown, err := machine.Show(ctx, "attempt-1")
	if err != nil || shown.Generation != 0 {
		t.Fatalf("lookup mutated: %v %v", shown, err)
	}
}

func TestGeneratedOperationIDSurvivesTheErrorPath(t *testing.T) {
	ctx := context.Background()
	_, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	args := map[string]any{"artifact_digest": "a", "baseline_digest": "b"}
	_, _, err = created.CommandWithOperation(ctx, "", "submit", args)
	generated := invariantkernel.OperationIDOf(err)
	if !strings.HasPrefix(generated, "go-") {
		t.Fatalf("generated id missing from error: %v", err)
	}
	lookup, err := machine.LookupOperation(ctx, "attempt-1", created.WireCapability(), generated, "submit", args)
	if err != nil || lookup.Outcome != "not_recorded" {
		t.Fatalf("a refusal must record nothing under the id: %+v %v", lookup, err)
	}
}

func TestMissingBinaryIsUnsentKeepsTheIDAndCommitsNothing(t *testing.T) {
	ctx := context.Background()
	kernel, machine := open(t)
	created, err := machine.Create(ctx, "attempt-1")
	if err != nil {
		t.Fatal(err)
	}
	realBinary := kernel.Binary
	kernel.Binary = filepath.Join(t.TempDir(), "no-such-ik")
	_, _, err = created.CommandWithOperation(ctx, "persisted-before-dispatch", "start", nil)
	if !invariantkernel.IsUnsent(err) {
		t.Fatalf("a missing binary must be provably unsent, got %v", err)
	}
	if got := invariantkernel.OperationIDOf(err); got != "persisted-before-dispatch" {
		t.Fatalf("operation id on error = %q", got)
	}
	kernel.Binary = realBinary
	lookup, err := machine.LookupOperation(ctx, "attempt-1", created.WireCapability(), "persisted-before-dispatch", "start", nil)
	if err != nil || lookup.Outcome != "not_recorded" {
		t.Fatalf("lookup = %+v err=%v", lookup, err)
	}
}

func TestTimeoutAfterDispatchIsSubmissionUnknownAndNeverRetried(t *testing.T) {
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
	if !errors.As(err, &transport) {
		t.Fatalf("want TransportError, got %v", err)
	}
	if transport.Unsent || invariantkernel.IsUnsent(err) {
		t.Fatal("a timeout after dispatch must not claim the request was unsent")
	}
	if got := invariantkernel.OperationIDOf(err); got != "maybe-committed" {
		t.Fatalf("operation id on error = %q", got)
	}
	logged, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Fields(string(logged)); len(got) != 1 {
		t.Fatalf("the client must dispatch exactly once, got %d", len(got))
	}
}

func TestPlainCommandAndRecordAlsoCarryAnOperationIDAfterAnUncertainTimeout(t *testing.T) {
	ctx := context.Background()
	kernel, machine := open(t)
	created, err := machine.Create(ctx, "attempt-plain")
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	stub := filepath.Join(dir, "slow-ik")
	if err := os.WriteFile(stub, []byte("#!/bin/sh\nsleep 5\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	kernel.Binary = stub
	kernel.Timeout = 300 * time.Millisecond
	_, err = created.Command(ctx, "start", nil)
	if id := invariantkernel.OperationIDOf(err); id == "" || invariantkernel.IsUnsent(err) {
		t.Fatalf("plain Command must be reconcilable: id=%q unsent=%v err=%v", id, invariantkernel.IsUnsent(err), err)
	}
	_, err = created.Record(ctx, "outcome", nil)
	if id := invariantkernel.OperationIDOf(err); id == "" || invariantkernel.IsUnsent(err) {
		t.Fatalf("plain Record must be reconcilable: id=%q unsent=%v err=%v", id, invariantkernel.IsUnsent(err), err)
	}
}
