package invariantkernel_test

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	invariantkernel "github.com/GauravAlbal/desmo/go"
)

// This schema-1 definition is authored independently, not frontend output.
// Its pinned hash is SHA-256 of its canonical typed Definition, including
// default state booleans and replace policy, calculated without invoking ik.
const regionIdentityHash = "73bb9c4b1fbcf9671b8d3236c276e85aacc226c09477ccd959ccad0da14592cb"
const regionIdentitySource = `schema_version = 2
machine_id = "client_identity"
machine_version = 7

[regions.power.states]
off = { initial = true }
on = { terminal = true }

[regions.power.commands.turn_on]
from = ["off"]
to = "on"
`
const regionIdentityFlat = `schema_version = 1
machine_id = "client_identity"
machine_version = 7

[facts.region_power]
type = "string"
policy = "replace"
initial = "off"

[states]
off = { initial = true }
on = { terminal = true }

[commands.turn_on]
from = ["off"]
[[commands.turn_on.outcomes]]
when = 'state == "off" && (region_power == "off")'
to = "on"
then = { region_power = '"on"' }

[invariants.region_projection_off]
when = 'state == "off"'
assert = 'region_power == "off"'
[invariants.region_projection_on]
when = 'state == "on"'
assert = 'region_power == "on"'
`

func regionBinary(t *testing.T) string {
	t.Helper()
	bin := os.Getenv("IK_BINARY")
	if bin == "" {
		t.Fatal("region contracts require explicit IK_BINARY; no build/download fallback")
	}
	abs, err := filepath.Abs(bin)
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(abs)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm()&0o111 == 0 {
		t.Fatalf("IK_BINARY is not an executable file: %s (%v)", abs, err)
	}
	return abs
}

func TestRegionSourceAwareCheckerAcceptsCompoundAndBindsParallelRefusal(t *testing.T) {
	ctx := context.Background()
	dir := t.TempDir()
	k, err := invariantkernel.Open(filepath.Join(dir, "check.db"), invariantkernel.Options{Binary: regionBinary(t)})
	if err != nil {
		t.Fatal(err)
	}
	corpus := filepath.Join(dir, "compound.cases.json")
	if err := os.WriteFile(corpus, []byte(`{"commands":{"power_off":[{"name":"child-enabled","args":{"override_enabled":true}},{"name":"parent-fallback","args":{"override_enabled":false}}]}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	compound, err := k.Check(ctx, filepath.Join(repoRoot(t), "machines", "region_compound.ik.toml"), "", corpus, "")
	if err != nil {
		t.Fatal(err)
	}
	if !compound.OK || compound.Verdict != "accept" || !strings.Contains(compound.Text, "VERDICT\n  ACCEPT\n") {
		t.Fatalf("compound must ACCEPT with both boolean guard cells: %+v", compound)
	}
	if strings.Contains(compound.Text, "invariant-refusal witness") {
		t.Fatalf("compound unexpectedly has invariant refusal: %s", compound.Text)
	}
	parallelPath := filepath.Join(repoRoot(t), "machines", "region_parallel.ik.toml")
	parallel, err := k.Check(ctx, parallelPath, "", "", "")
	if err != nil {
		t.Fatal(err)
	}
	if parallel.OK || parallel.Verdict != "refused" || !strings.Contains(parallel.Text, "VERDICT\n  REFUSED\n") {
		t.Fatalf("parallel unsafe mute must REFUSE: %+v", parallel)
	}
	if !strings.Contains(parallel.Text, "invariant-refusal witness IK_INVARIANT_FAILED rule=no_playing_muted\n") {
		t.Fatalf("parallel witness lost exact invariant binding: %s", parallel.Text)
	}
	machine, err := k.Machine(ctx, parallelPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(parallel.Text, "  spec_hash "+machine.Identity.SpecHash()+"\n") {
		t.Fatalf("checker and runtime loaded different lowered identities: %s", parallel.Text)
	}
}

func TestRegionSourceAndExplicitFlatSharePinnedIdentityAndStore(t *testing.T) {
	ctx := context.Background()
	dir := t.TempDir()
	sourcePath, flatPath := filepath.Join(dir, "source.ik.toml"), filepath.Join(dir, "flat.ik.toml")
	if err := os.WriteFile(sourcePath, []byte(regionIdentitySource), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(flatPath, []byte(regionIdentityFlat), 0o600); err != nil {
		t.Fatal(err)
	}
	k, err := invariantkernel.Open(filepath.Join(dir, "identity.db"), invariantkernel.Options{Binary: regionBinary(t)})
	if err != nil {
		t.Fatal(err)
	}
	source, err := k.Machine(ctx, sourcePath)
	if err != nil {
		t.Fatal(err)
	}
	flat, err := k.Machine(ctx, flatPath)
	if err != nil {
		t.Fatal(err)
	}
	if source.Identity.MachineID != "client_identity" || source.Identity.Version != 7 || source.Identity.SpecHash() != regionIdentityHash {
		t.Fatalf("source identity differs from independent schema-1 oracle: %+v", source.Identity)
	}
	if flat.Identity != source.Identity || flat.Identity.SpecHash() != regionIdentityHash {
		t.Fatalf("explicit flat identity mismatch: flat=%+v source=%+v", flat.Identity, source.Identity)
	}
	created, err := source.Create(ctx, "cross-source")
	if err != nil {
		t.Fatal(err)
	}
	if created.State() != "off" || !reflect.DeepEqual(created.Facts(), map[string]any{"region_power": "off"}) {
		t.Fatalf("initial projection mismatch: %v", created.Raw)
	}
	shown, err := flat.Show(ctx, "cross-source")
	if err != nil {
		t.Fatal(err)
	}
	completed, err := shown.Command(ctx, "turn_on", nil)
	if err != nil {
		t.Fatal(err)
	}
	expected := map[string]any{"state": "on", "region_power": "on"}
	if !reflect.DeepEqual(completed.Raw["state"], expected) || completed.Generation != 1 || !completed.Terminal {
		t.Fatalf("cross-source command mismatch: %v", completed.Raw)
	}
	replayed, err := source.Replay(ctx, "cross-source")
	if err != nil {
		t.Fatal(err)
	}
	rows := replayed["aggregates"].([]any)
	if len(rows) != 1 || !reflect.DeepEqual(rows[0].(map[string]any)["state"], expected) {
		t.Fatalf("source replay of flat command mismatch: %v", replayed)
	}
	verified, err := source.Verify(ctx, "cross-source")
	if err != nil {
		t.Fatal(err)
	}
	if verified["verified"] != true {
		t.Fatalf("cross-source verification failed: %v", verified)
	}
}

func TestRegionParallelRefusalPreservesAuthorityThenJointCommitsAndReplays(t *testing.T) {
	ctx := context.Background()
	bin := regionBinary(t)
	version, err := exec.Command(bin, "--version").Output()
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(strings.Fields(string(version)), []string{"invariant-kernel", "0.7.1", "protocol", "1.0"}) {
		t.Fatalf("runtime release contract mismatch: %s", version)
	}
	k, err := invariantkernel.Open(filepath.Join(t.TempDir(), "parallel.db"), invariantkernel.Options{Binary: bin})
	if err != nil {
		t.Fatal(err)
	}
	machine, err := k.Machine(ctx, filepath.Join(repoRoot(t), "machines", "region_parallel.ik.toml"))
	if err != nil {
		t.Fatal(err)
	}
	original, err := machine.Create(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	before, err := machine.Show(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	beforeReplay, err := machine.Replay(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	initial := map[string]any{"state": "playing__unmuted", "region_playback": "playing", "region_volume": "unmuted"}
	if !reflect.DeepEqual(before.Raw["state"], initial) || before.Generation != 0 {
		t.Fatalf("initial parallel projection mismatch: %v", before.Raw)
	}
	_, err = original.Command(ctx, "mute", nil)
	var refusal *invariantkernel.InvariantFailedError
	if !errors.As(err, &refusal) || refusal.Code != "IK_INVARIANT_FAILED" || refusal.Rule != "no_playing_muted" {
		t.Fatalf("unsafe mute lost typed invariant diagnostic: %v", err)
	}
	after, err := machine.Show(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(after.Raw, before.Raw) || !reflect.DeepEqual(after.Raw["capability"], original.Raw["capability"]) {
		t.Fatalf("refusal changed public state/authority: before=%v after=%v", before.Raw, after.Raw)
	}
	afterReplay, err := machine.Replay(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(afterReplay, beforeReplay) {
		t.Fatalf("refusal changed durable replay: before=%v after=%v", beforeReplay, afterReplay)
	}
	refusedVerify, err := machine.Verify(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	if refusedVerify["verified"] != true || refusedVerify["journal_ok"] != true || refusedVerify["projection_ok"] != true {
		t.Fatalf("refusal corrupted durable consistency: %v", refusedVerify)
	}
	// Original authority remains current; the joint cannot commit an unsafe
	// playing+muted intermediate projection.
	committed, err := original.Command(ctx, "pause_and_mute", nil)
	if err != nil {
		t.Fatal(err)
	}
	expected := map[string]any{"state": "paused__muted", "region_playback": "paused", "region_volume": "muted"}
	if !reflect.DeepEqual(committed.Raw["state"], expected) || committed.Generation != 1 || committed.Terminal {
		t.Fatalf("joint projection mismatch: %v", committed.Raw)
	}
	if !reflect.DeepEqual(committed.Facts(), map[string]any{"region_playback": "paused", "region_volume": "muted"}) {
		t.Fatalf("joint facts mismatch: %v", committed.Facts())
	}
	if reflect.DeepEqual(committed.Raw["capability"], original.Raw["capability"]) {
		t.Fatalf("joint failed to rotate authority: %v", committed.Raw)
	}
	replayed, err := machine.Replay(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	rows := replayed["aggregates"].([]any)
	if len(rows) != 1 || !reflect.DeepEqual(rows[0].(map[string]any)["state"], expected) || asUint(rows[0].(map[string]any)["generation"]) != 1 {
		t.Fatalf("joint replay mismatch: %v", replayed)
	}
	again, err := machine.Replay(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(again, replayed) {
		t.Fatalf("replay is nondeterministic: first=%v second=%v", replayed, again)
	}
	verified, err := machine.Verify(ctx, "playback")
	if err != nil {
		t.Fatal(err)
	}
	if verified["verified"] != true || verified["journal_ok"] != true || verified["projection_ok"] != true {
		t.Fatalf("joint verification failed: %v", verified)
	}
}
