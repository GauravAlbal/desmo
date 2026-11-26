package invariantkernel

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"time"
)

// Options configure a Kernel.
type Options struct {
	Binary  string
	Timeout time.Duration
	Session bool
}

// Kernel is a typed adapter over one ik store.
type Kernel struct {
	Store          string
	Binary         string
	Timeout        time.Duration
	runtimeVersion string
	protocolMajor  uint64
	protocolMinor  uint64
	nextID         atomic.Uint64
	useSession     bool
	sessionsMu     sync.Mutex
	sessions       map[string]*sessionProc
}

// Open locates ik and binds a store path. The store file is created only by create, never by a read.
func Open(store string, opts Options) (*Kernel, error) {
	bin, err := locateBinary(opts.Binary)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout(opts.Timeout))
	defer cancel()
	runtime, major, minor, err := parseVersion(ctx, bin)
	if err != nil {
		return nil, err
	}
	k := &Kernel{
		Store:          resolvePath(store),
		Binary:         bin,
		Timeout:        defaultTimeout(opts.Timeout),
		runtimeVersion: runtime,
		protocolMajor:  major,
		protocolMinor:  minor,
		useSession:     opts.Session,
		sessions:       map[string]*sessionProc{},
	}
	return k, nil
}

// Close shuts down any persistent `ik --session` processes opened by this Kernel.
func (k *Kernel) Close() error {
	k.sessionsMu.Lock()
	procs := k.sessions
	k.sessions = map[string]*sessionProc{}
	k.sessionsMu.Unlock()
	var firstErr error
	for _, proc := range procs {
		if err := proc.close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}

func (k *Kernel) sessionFor(machinePath string) (*sessionProc, error) {
	k.sessionsMu.Lock()
	defer k.sessionsMu.Unlock()
	if existing, ok := k.sessions[machinePath]; ok && !existing.broken {
		return existing, nil
	}
	proc, err := startSession(k.Binary, k.Store, machinePath)
	if err != nil {
		return nil, err
	}
	k.sessions[machinePath] = proc
	return proc, nil
}

// Machine loads identity from `ik identity` so callers never hash a spec.
func (k *Kernel) Machine(ctx context.Context, path string) (*Machine, error) {
	ctx, cancel := k.bound(ctx)
	defer cancel()
	abs := resolvePath(path)
	raw, err := runIdentity(ctx, k.Binary, abs)
	if err != nil {
		return nil, err
	}
	id, err := identityFromWire(raw)
	if err != nil {
		return nil, err
	}
	return &Machine{kernel: k, Path: abs, Identity: id}, nil
}

// Bind attaches a known machine identity without probing `ik identity`.
// Used by native machine packages whose identity is compiled in.
func (k *Kernel) Bind(id MachineIdentity) *Machine {
	return &Machine{kernel: k, Identity: id}
}

// Invoke sends one protocol envelope. Extra fields (package, observation) pass through.
// Pass m only when that operation's wire contract permits or requires `machine`.
// A Machine obtained via Bind does not imply that all companion operations are machine-scoped.
func (k *Kernel) Invoke(ctx context.Context, op string, m *Machine, fields map[string]any) (map[string]any, error) {
	return k.call(ctx, op, m, fields)
}

// Check is developer analysis. It is not a lifecycle operation.
func (k *Kernel) Check(ctx context.Context, path string, prior, corpus, baseline string) (CheckResult, error) {
	ctx, cancel := k.bound(ctx)
	defer cancel()
	text, ok, err := runCheck(ctx, k.Binary, resolvePath(path), prior, corpus, baseline)
	if err != nil {
		return CheckResult{}, err
	}
	verdict := "refused"
	if ok {
		verdict = "accept"
	}
	return CheckResult{Text: text, OK: ok, Verdict: verdict}, nil
}

// Show reads one aggregate. Machine is required when the caller wants a live handle.
func (k *Kernel) Show(ctx context.Context, m *Machine, aggregateID string) (*Aggregate, error) {
	result, err := k.call(ctx, "show", m, map[string]any{"aggregate_id": aggregateID})
	if err != nil {
		return nil, err
	}
	return aggregateFromResult(m, result)
}

// Replay rebuilds projections from the journal.
func (k *Kernel) Replay(ctx context.Context, m *Machine, aggregateID string) (map[string]any, error) {
	fields := map[string]any{}
	if aggregateID != "" {
		fields["aggregate_id"] = aggregateID
	}
	return k.call(ctx, "replay", m, fields)
}

// Verify checks journal/projection consistency.
func (k *Kernel) Verify(ctx context.Context, m *Machine, aggregateID string) (map[string]any, error) {
	fields := map[string]any{}
	if aggregateID != "" {
		fields["aggregate_id"] = aggregateID
	}
	return k.call(ctx, "verify", m, fields)
}

// Events streams committed events strictly after opts.Since paired with each aggregate's latest projection.
func (k *Kernel) Events(ctx context.Context, opts EventsOptions) (*EventFeed, error) {
	ctx, cancel := k.bound(ctx)
	defer cancel()
	return runEvents(ctx, k.Binary, k.Store, opts.Since, opts.AggregateID, opts.Limit)
}

// List queries materialized aggregates read-only with optional filters and keyset pagination.
func (k *Kernel) List(ctx context.Context, opts ListOptions) (*AggregateList, error) {
	ctx, cancel := k.bound(ctx)
	defer cancel()
	return runList(ctx, k.Binary, k.Store, opts)
}

func (k *Kernel) bound(ctx context.Context) (context.Context, context.CancelFunc) {
	if ctx == nil {
		ctx = context.Background()
	}
	if _, ok := ctx.Deadline(); ok {
		return ctx, func() {}
	}
	return context.WithTimeout(ctx, k.Timeout)
}

func (k *Kernel) requestID() string {
	n := k.nextID.Add(1)
	return fmt.Sprintf("go-%d", n)
}

func (k *Kernel) call(ctx context.Context, op string, m *Machine, fields map[string]any) (map[string]any, error) {
	ctx, cancel := k.bound(ctx)
	defer cancel()
	env := map[string]any{
		"protocol":        map[string]any{"major": k.protocolMajor, "minor": k.protocolMinor},
		"runtime_version": k.runtimeVersion,
		"request_id":      k.requestID(),
		"op":              op,
	}
	for key, value := range fields {
		if value != nil {
			env[key] = value
		}
	}
	var machinePath string
	if m != nil && m.Identity.MachineID != "" {
		env["machine"] = m.Identity.wire()
		machinePath = m.Path
	}
	if k.useSession {
		proc, err := k.sessionFor(machinePath)
		if err != nil {
			return nil, err
		}
		return proc.invoke(ctx, env)
	}
	return invoke(ctx, k.Binary, k.Store, machinePath, env)
}

// Machine is a compiled definition bound to a Kernel.
type Machine struct {
	kernel   *Kernel
	Path     string
	Identity MachineIdentity
}

func (m *Machine) Create(ctx context.Context, aggregateID string) (*Aggregate, error) {
	result, err := m.kernel.call(ctx, "create", m, map[string]any{"aggregate_id": aggregateID})
	if err != nil {
		return nil, err
	}
	return aggregateFromResult(m, result)
}

// CreateWith creates the aggregate and applies the initializing command in one
// validated operation. An invalid command or a law violation exposes no aggregate.
// The same operation id replays as history. Reconcile a dropped reply with
// LookupCreateWith. A bare Create followed by a required initialize is not this.
func (m *Machine) CreateWith(ctx context.Context, aggregateID, operationID string, command map[string]any) (*Aggregate, *Historical, error) {
	if operationID == "" {
		operationID = NewOperationID()
	}
	result, err := m.kernel.call(ctx, "create_with_command", m, map[string]any{
		"aggregate_id": aggregateID,
		"command":      command,
		"operation_id": operationID,
	})
	if err != nil {
		return nil, nil, annotateOperation(err, operationID)
	}
	if historical, _ := result["historical"].(bool); historical {
		return nil, historicalFromMap(aggregateID, result), nil
	}
	next, err := aggregateFromResult(m, result)
	if err != nil {
		return nil, nil, annotateOperation(err, operationID)
	}
	return next, nil, nil
}

func (m *Machine) Show(ctx context.Context, aggregateID string) (*Aggregate, error) {
	return m.kernel.Show(ctx, m, aggregateID)
}

// List queries aggregates belonging to this machine.
func (m *Machine) List(ctx context.Context, opts ListOptions) (*AggregateList, error) {
	opts.MachineID = m.Identity.MachineID
	return m.kernel.List(ctx, opts)
}

func (m *Machine) Replay(ctx context.Context, aggregateID string) (map[string]any, error) {
	return m.kernel.Replay(ctx, m, aggregateID)
}

func (m *Machine) Verify(ctx context.Context, aggregateID string) (map[string]any, error) {
	return m.kernel.Verify(ctx, m, aggregateID)
}

// Aggregate is a handle that carries the current capability internally.
type Aggregate struct {
	machine    *Machine
	ID         string
	capability Capability
	rawState   any
	Generation uint64
	Terminal   bool
	Effect     Effect
	Receipt    *Receipt
	Mutation   *bool
	Raw        map[string]any
}

func aggregateFromResult(m *Machine, result map[string]any) (*Aggregate, error) {
	capRaw, _ := result["capability"].(map[string]any)
	cap, err := capabilityFromWire(capRaw)
	if err != nil {
		return nil, err
	}
	var mutation *bool
	if v, ok := result["mutation"].(bool); ok {
		mutation = &v
	}
	receipt, _ := result["receipt"].(map[string]any)
	if receipt == nil {
		receipt, _ = result["last_receipt"].(map[string]any)
	}
	effect, _ := result["effect"].(map[string]any)
	return &Aggregate{
		machine:    m,
		ID:         asString(result["aggregate_id"]),
		capability: cap,
		rawState:   result["state"],
		Generation: asUint(result["generation"]),
		Terminal:   asBool(result["terminal"]),
		Effect:     effectFromWire(effect),
		Receipt:    receiptFromWire(receipt),
		Mutation:   mutation,
		Raw:        result,
	}, nil
}

// State is the kernel state name when the payload is a declarative object.
func (a *Aggregate) State() any {
	if obj, ok := a.rawState.(map[string]any); ok {
		if name, ok := obj["state"]; ok {
			return name
		}
	}
	return a.rawState
}

// StateName returns the declarative state name as a string, or "" for non-declarative payloads.
func (a *Aggregate) StateName() string {
	s, _ := a.State().(string)
	return s
}

// Owner is the identity that owns this aggregate in the store. After a machine version change it can
// differ from the handle's Machine.Identity on a Show through another version. Route an existing aggregate
// to the retained definition whose Identity equals Owner (MachineIdentity values are comparable).
func (a *Aggregate) Owner() MachineIdentity {
	if wire, ok := a.Raw["machine"].(map[string]any); ok {
		if id, err := identityFromWire(wire); err == nil {
			return id
		}
	}
	return a.machine.Identity
}

// Facts are the non-state fields of a declarative payload.
func (a *Aggregate) Facts() map[string]any {
	obj, ok := a.rawState.(map[string]any)
	if !ok {
		return map[string]any{}
	}
	out := make(map[string]any, len(obj))
	for k, v := range obj {
		if k != "state" {
			out[k] = v
		}
	}
	return out
}

// Command invokes a domain command under a freshly generated operation id (see
// CommandWithOperation to supply your own). Args may be nil. If the outcome is unknown,
// the error carries the id (OperationIDOf): reconcile with LookupOperation; do not resend.
func (a *Aggregate) Command(ctx context.Context, name string, args map[string]any) (*Aggregate, error) {
	next, historical, err := a.CommandWithOperation(ctx, "", name, args)
	if err != nil {
		return nil, err
	}
	if historical != nil {
		return nil, fmt.Errorf("unexpected historical result for a freshly generated operation id")
	}
	return next, nil
}

// Record binds an effect outcome under a fresh operation id. Missing identity fields are taken from the pending intent.
func (a *Aggregate) Record(ctx context.Context, outcomeDigest string, override map[string]any) (*Aggregate, error) {
	next, historical, err := a.RecordWithOperation(ctx, "", outcomeDigest, override)
	if err != nil {
		return nil, err
	}
	if historical != nil {
		return nil, fmt.Errorf("unexpected historical result for a freshly generated operation id")
	}
	return next, nil
}

// RecordWithOperation binds an effect outcome under an explicit operation id (or generated when empty),
// returning either the updated Aggregate or a Historical result on exact operation-id replay.
func (a *Aggregate) RecordWithOperation(ctx context.Context, operationID, outcomeDigest string, override map[string]any) (*Aggregate, *Historical, error) {
	wire := map[string]any{
		"aggregate_id":       a.ID,
		"outcome_digest":     outcomeDigest,
		"effect_id":          "",
		"payload_digest":     "",
		"idempotency_key":    "",
		"planned_generation": a.Generation,
	}
	if a.Effect.Intent != nil {
		wire["effect_id"] = a.Effect.Intent.EffectID
		wire["payload_digest"] = a.Effect.Intent.PayloadDigest
		wire["idempotency_key"] = a.Effect.Intent.IdempotencyKey
		wire["planned_generation"] = a.Effect.Intent.Generation
	}
	for k, v := range override {
		wire[k] = v
	}
	if operationID == "" {
		operationID = NewOperationID()
	}
	result, err := a.machine.kernel.call(ctx, "record", a.machine, map[string]any{
		"aggregate_id": a.ID,
		"capability":   a.capability.wire(),
		"effect":       wire,
		"operation_id": operationID,
	})
	if err != nil {
		return nil, nil, annotateOperation(err, operationID)
	}
	if historical, _ := result["historical"].(bool); historical {
		record, _ := result["operation"].(map[string]any)
		current, _ := result["current"].(map[string]any)
		return nil, historicalFromRecord(a.ID, record, current["generation"], current["sequence"], result), nil
	}
	next, err := aggregateFromResult(a.machine, result)
	return next, nil, annotateOperation(err, operationID)
}

// EffectRunResult holds the durable planned handle, recorded outcome handle (or historical
// replay), bound EffectIntent, and recorded outcome digest returned by Aggregate.RunEffect.
type EffectRunResult struct {
	Planned       *Aggregate
	Recorded      *Aggregate
	Historical    *Historical
	Intent        EffectIntent
	OutcomeDigest string
}

// RunEffect plans an external effect via commandName, invokes executor(ctx, intent) once
// with the kernel-bound EffectIntent (including IdempotencyKey), and records the returned
// outcome digest. Picky and explicit: refuses if planning replays as Historical (avoiding
// accidental duplicate side effects) or commits without a pending effect intent.
func (a *Aggregate) RunEffect(
	ctx context.Context,
	commandName string,
	args map[string]any,
	executor func(context.Context, EffectIntent) (string, error),
	planOpID string,
	recordOpID string,
) (*EffectRunResult, error) {
	if executor == nil {
		return nil, errors.New("executor callback is required")
	}
	planned, hist, err := a.CommandWithOperation(ctx, planOpID, commandName, args)
	if err != nil {
		return nil, err
	}
	if hist != nil {
		return nil, fmt.Errorf("planning command %q replayed as historical operation %s; inspect Show() and reconcile before re-running external side effects", commandName, hist.OperationID)
	}
	if planned.Effect.Status != "pending" || planned.Effect.Intent == nil {
		return nil, fmt.Errorf("command %q committed without planning a pending effect intent", commandName)
	}
	intent := *planned.Effect.Intent
	outcomeDigest, err := executor(ctx, intent)
	if err != nil {
		return nil, err
	}
	recorded, recHist, err := planned.RecordEffectOutcome(ctx, recordOpID, outcomeDigest)
	if err != nil {
		return nil, err
	}
	return &EffectRunResult{
		Planned:       planned,
		Recorded:      recorded,
		Historical:    recHist,
		Intent:        intent,
		OutcomeDigest: outcomeDigest,
	}, nil
}

// RecordEffectOutcome is the picky effect completion helper: requires a non-empty outcomeDigest
// and a pending EffectIntent on this handle before dispatching `record`.
func (a *Aggregate) RecordEffectOutcome(ctx context.Context, operationID, outcomeDigest string) (*Aggregate, *Historical, error) {
	if outcomeDigest == "" {
		return nil, nil, errors.New("outcome digest is required and must be non-empty")
	}
	if a.Effect.Status != "pending" || a.Effect.Intent == nil {
		return nil, nil, errors.New("aggregate has no pending effect intent to record")
	}
	return a.RecordWithOperation(ctx, operationID, outcomeDigest, nil)
}

// Transaction applies an atomic sequence of command steps inside one kernel SQLite transaction.
// Each step is a command map `{"name": ..., "args": ...}` (or `{"command": ..., "observation": ...}`).
func (a *Aggregate) Transaction(ctx context.Context, operationID string, steps []map[string]any) (*Aggregate, *Historical, error) {
	if operationID == "" {
		operationID = NewOperationID()
	}
	wireSteps := make([]map[string]any, 0, len(steps))
	for _, step := range steps {
		if _, hasCmd := step["command"]; hasCmd {
			wireSteps = append(wireSteps, step)
		} else {
			wireSteps = append(wireSteps, map[string]any{"command": step})
		}
	}
	result, err := a.machine.kernel.call(ctx, "transaction", a.machine, map[string]any{
		"aggregate_id": a.ID,
		"capability":   a.capability.wire(),
		"commands":     wireSteps,
		"operation_id": operationID,
	})
	if err != nil {
		return nil, nil, annotateOperation(err, operationID)
	}
	if historical, _ := result["historical"].(bool); historical {
		record, _ := result["operation"].(map[string]any)
		current, _ := result["current"].(map[string]any)
		return nil, historicalFromRecord(a.ID, record, current["generation"], current["sequence"], result), nil
	}
	next, err := aggregateFromResult(a.machine, result)
	return next, nil, annotateOperation(err, operationID)
}

func (a *Aggregate) Show(ctx context.Context) (*Aggregate, error) {
	return a.machine.Show(ctx, a.ID)
}

func (a *Aggregate) Replay(ctx context.Context) (map[string]any, error) {
	return a.machine.Replay(ctx, a.ID)
}

func (a *Aggregate) KernelVerify(ctx context.Context) (map[string]any, error) {
	return a.machine.Verify(ctx, a.ID)
}

// SetProtocolForTest overrides the spoken protocol. Production code must not call this.
func (k *Kernel) SetProtocolForTest(major, minor uint64) {
	k.protocolMajor = major
	k.protocolMinor = minor
}

// RepoRoot walks up from start looking for Cargo.toml + machines/.
func RepoRoot(start string) (string, error) {
	dir := start
	for range 8 {
		if fileExists(filepath.Join(dir, "Cargo.toml")) && fileExists(filepath.Join(dir, "machines")) {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return "", &TransportError{Message: "invariant-kernel repo root not found"}
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
