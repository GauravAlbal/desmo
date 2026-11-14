package invariantkernel

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
)

// NewOperationID returns a fresh caller operation id. Persist it (with the capability's
// wire form and the request body) before dispatch if you need to recover after your own
// process dies. The wire grammar is [A-Za-z0-9._:-]{1,128}.
func NewOperationID() string {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		panic("invariantkernel: crypto/rand unavailable: " + err.Error())
	}
	return "go-" + hex.EncodeToString(raw[:])
}

// Historical is an exact repeat of an already committed operation. It is history, not
// authority: it carries no capability and no state, and Current* may be well past the commit.
// Re-read with Machine.Show for a current handle.
type Historical struct {
	AggregateID       string
	OperationID       string
	Kind              string
	FirstSequence     uint64
	LastSequence      uint64
	Generation        uint64
	EventHashes       []string
	ReceiptIDs        []string
	CurrentGeneration uint64
	CurrentSequence   uint64
	Raw               map[string]any
}

func historicalFromRecord(aggregateID string, record map[string]any, currentGeneration, currentSequence any, raw map[string]any) *Historical {
	return &Historical{
		AggregateID:       aggregateID,
		OperationID:       asString(record["operation_id"]),
		Kind:              asString(record["kind"]),
		FirstSequence:     asUint(record["first_sequence"]),
		LastSequence:      asUint(record["last_sequence"]),
		Generation:        asUint(record["generation"]),
		EventHashes:       stringList(record["event_hashes"]),
		ReceiptIDs:        stringList(record["receipt_ids"]),
		CurrentGeneration: asUint(currentGeneration),
		CurrentSequence:   asUint(currentSequence),
		Raw:               raw,
	}
}

func stringList(v any) []string {
	items, _ := v.([]any)
	out := make([]string, 0, len(items))
	for _, item := range items {
		out = append(out, asString(item))
	}
	return out
}

// OperationLookup is the outcome of LookupOperation. Outcome is "committed_exact",
// "conflict" or "not_recorded". "not_recorded" is not permission to retry under a stale handle.
type OperationLookup struct {
	Outcome           string
	History           *Historical
	CurrentGeneration *uint64
	CurrentSequence   *uint64
	Raw               map[string]any
}

// WireCapability is the persistable form of a capability, for recovery after a process
// restart. It is authority: store it like a secret.
func (a *Aggregate) WireCapability() map[string]any { return a.capability.wire() }

// CommandWithOperation dispatches one command exactly once under operationID (generated when
// empty). There is no automatic retry and no handle refresh. Exactly one of the first two
// results is non-nil on success: a new *Aggregate for a new commit, or a *Historical for an
// exact repeat of a committed operation. Every error carries the id; see OperationIDOf.
func (a *Aggregate) CommandWithOperation(ctx context.Context, operationID, name string, args map[string]any) (*Aggregate, *Historical, error) {
	if operationID == "" {
		operationID = NewOperationID()
	}
	payload := map[string]any{"name": name}
	if len(args) > 0 {
		payload["args"] = args
	}
	result, err := a.machine.kernel.call(ctx, "command", a.machine, map[string]any{
		"aggregate_id": a.ID,
		"capability":   a.capability.wire(),
		"command":      payload,
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
	if err != nil {
		return nil, nil, annotateOperation(err, operationID)
	}
	return next, nil, nil
}

// LookupOperation asks, without mutating anything, whether operationID committed. Pass the
// original capability wire form (see WireCapability) and the ORIGINAL command; a changed
// command reports "conflict". Replay and Verify audit the journal; they do not answer this.
func (m *Machine) LookupOperation(ctx context.Context, aggregateID string, capability map[string]any, operationID, name string, args map[string]any) (*OperationLookup, error) {
	payload := map[string]any{"name": name}
	if len(args) > 0 {
		payload["args"] = args
	}
	result, err := m.kernel.call(ctx, "lookup_operation", m, map[string]any{
		"aggregate_id": aggregateID,
		"capability":   capability,
		"command":      payload,
		"operation_id": operationID,
	})
	if err != nil {
		return nil, annotateOperation(err, operationID)
	}
	lookup := &OperationLookup{Outcome: asString(result["outcome"]), Raw: result}
	if v, ok := result["current_generation"]; ok && v != nil {
		n := asUint(v)
		lookup.CurrentGeneration = &n
	}
	if v, ok := result["current_sequence"]; ok && v != nil {
		n := asUint(v)
		lookup.CurrentSequence = &n
	}
	if lookup.Outcome == "committed_exact" {
		record, _ := result["record"].(map[string]any)
		lookup.History = historicalFromRecord(aggregateID, record, result["current_generation"], result["current_sequence"], result)
	}
	return lookup, nil
}

// LookupCreateWith asks whether a CreateWith operation id committed. It sends no capability.
func (m *Machine) LookupCreateWith(ctx context.Context, aggregateID, operationID string, command map[string]any) (*OperationLookup, error) {
	result, err := m.kernel.call(ctx, "lookup_operation", m, map[string]any{
		"aggregate_id": aggregateID,
		"operation_id": operationID,
		"command":      command,
	})
	if err != nil {
		return nil, annotateOperation(err, operationID)
	}
	return lookupFromResult(aggregateID, result), nil
}

func historicalFromMap(aggregateID string, result map[string]any) *Historical {
	record, _ := result["operation"].(map[string]any)
	current, _ := result["current"].(map[string]any)
	return historicalFromRecord(aggregateID, record, current["generation"], current["sequence"], result)
}

func lookupFromResult(aggregateID string, result map[string]any) *OperationLookup {
	lookup := &OperationLookup{Outcome: asString(result["outcome"]), Raw: result}
	if v, ok := result["current_generation"]; ok && v != nil {
		n := asUint(v)
		lookup.CurrentGeneration = &n
	}
	if v, ok := result["current_sequence"]; ok && v != nil {
		n := asUint(v)
		lookup.CurrentSequence = &n
	}
	if lookup.Outcome == "committed_exact" {
		record, _ := result["record"].(map[string]any)
		lookup.History = historicalFromRecord(aggregateID, record, result["current_generation"], result["current_sequence"], result)
	}
	return lookup
}

// OperationIDOf returns the operation id carried by err, or "" when it has none.
func OperationIDOf(err error) string {
	var kernel *KernelError
	if errors.As(err, &kernel) && kernel.OperationID != "" {
		return kernel.OperationID
	}
	var transport *TransportError
	if errors.As(err, &transport) {
		return transport.OperationID
	}
	return ""
}

// IsUnsent reports whether err proves the request was never dispatched. Any other
// *TransportError means the request may have committed.
func IsUnsent(err error) bool {
	var transport *TransportError
	return errors.As(err, &transport) && transport.Unsent
}

func annotateOperation(err error, operationID string) error {
	var kernel *KernelError
	if errors.As(err, &kernel) {
		kernel.OperationID = operationID
	}
	var transport *TransportError
	if errors.As(err, &transport) {
		transport.OperationID = operationID
	}
	return err
}
