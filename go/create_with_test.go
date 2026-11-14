package invariantkernel_test

import (
	"context"
	"path/filepath"
	"testing"

	invariantkernel "github.com/GauravAlbal/desmo/go"
)

func TestCreateWithHidesInvalidAdmissionAndReplays(t *testing.T) {
	ctx := context.Background()
	kernel, machine := openRevision(t)
	bad := map[string]any{"name": "approve", "args": map[string]any{"revision": 1}}
	if _, _, err := machine.CreateWith(ctx, "doc-bad", "op-bad", bad); err == nil {
		t.Fatal("invalid admission must refuse")
	}
	if _, err := machine.Show(ctx, "doc-bad"); err == nil {
		t.Fatal("invalid admission must expose no aggregate")
	}
	missing, err := machine.LookupCreateWith(ctx, "doc-bad", "op-bad", bad)
	if err != nil {
		t.Fatal(err)
	}
	if missing.Outcome != "not_recorded" {
		t.Fatalf("lookup = %+v", missing)
	}
	edit := map[string]any{"name": "edit", "args": map[string]any{"artifact_digest": "doc-1"}}
	created, historical, err := machine.CreateWith(ctx, "doc-good", "op-good", edit)
	if err != nil || created == nil || historical != nil {
		t.Fatalf("create: created=%v historical=%v err=%v", created, historical, err)
	}
	if created.State() != "draft" || created.Generation != 1 {
		t.Fatalf("created = state %v gen %d", created.State(), created.Generation)
	}
	again, history, err := machine.CreateWith(ctx, "doc-good", "op-good", edit)
	if err != nil || again != nil || history == nil || history.Kind != "create_with_command" {
		t.Fatalf("replay: again=%v history=%v err=%v", again, history, err)
	}
	found, err := machine.LookupCreateWith(ctx, "doc-good", "op-good", edit)
	if err != nil || found.Outcome != "committed_exact" || found.History == nil {
		t.Fatalf("lookup = %+v err=%v", found, err)
	}
	_ = kernel
}

func openRevision(t *testing.T) (*invariantkernel.Kernel, *invariantkernel.Machine) {
	t.Helper()
	k, err := invariantkernel.Open(t.TempDir()+"/kernel.db", invariantkernel.Options{Binary: ikBin(t)})
	if err != nil {
		t.Fatal(err)
	}
	machine, err := k.Machine(context.Background(), filepath.Join(repoRoot(t), "examples/approval-revision/approval_revision.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	return k, machine
}
