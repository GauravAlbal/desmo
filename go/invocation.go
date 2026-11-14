package invariantkernel

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// InvocationWireVersion is the persisted-form version written by Invocation.JSON.
const InvocationWireVersion = 1

// Invocation is a recoverable description of one logical command invocation, built BEFORE
// anything is sent: the operation id and the original request (machine identity, aggregate, the
// capability the request was authorized against, and the command). Persist JSON() in the durable
// job record you already keep, then Machine.Dispatch. After a crash, InvocationFromJSON and
// Machine.Reconcile do an exact lookup of the original operation. Nothing here refreshes a
// capability, derives an id from the current generation, or resends. If the description is lost
// the invocation cannot be attributed from the store alone. The persisted form is authority
// (it carries the capability): store it like a secret.
//
// An Invocation owns a snapshot. The request is serialized once, in Prepare, and every read
// decodes a fresh copy, so mutating the caller's argument map, or a map returned by Command,
// cannot change the bound request. A value that cannot be serialized is refused by Prepare,
// before anything is sent. The persisted form is versioned and semantically lossless; the bytes
// are not promised identical to other clients' bytes for the same request (escaping differs).
type Invocation struct {
	raw []byte
}

// invocationWire is the persisted shape. Decoding uses json.Number so large integers survive.
type invocationWire struct {
	Version     int    `json:"invocation_version"`
	Kind        string `json:"kind"`
	AggregateID string `json:"aggregate_id"`
	OperationID string `json:"operation_id"`
	Machine     struct {
		MachineID string `json:"machine_id"`
		Version   uint64 `json:"version"`
		SpecHash  string `json:"spec_hash"`
	} `json:"machine"`
	Capability struct {
		Generation uint64 `json:"generation"`
		Nonce      string `json:"nonce"`
	} `json:"capability"`
	Command map[string]any `json:"command"`
}

// Prepare describes one command invocation without sending it. operationID is generated when
// empty. It returns an error when args cannot be serialized.
func (a *Aggregate) Prepare(operationID, name string, args map[string]any) (*Invocation, error) {
	if operationID == "" {
		operationID = NewOperationID()
	}
	payload := map[string]any{"name": name}
	if len(args) > 0 {
		payload["args"] = args
	}
	raw, err := json.Marshal(map[string]any{
		"invocation_version": InvocationWireVersion,
		"kind":               "command",
		"machine":            a.machine.Identity.wire(),
		"aggregate_id":       a.ID,
		"operation_id":       operationID,
		"capability":         a.capability.wire(),
		"command":            payload,
	})
	if err != nil {
		return nil, fmt.Errorf("invocation is not serializable: %w", err)
	}
	return &Invocation{raw: raw}, nil
}

// JSON is the persistable form: a copy of the snapshot, with keys sorted.
func (i *Invocation) JSON() []byte { return append([]byte(nil), i.raw...) }

func (i *Invocation) wire() (invocationWire, error) {
	var wire invocationWire
	dec := json.NewDecoder(bytes.NewReader(i.raw))
	dec.UseNumber()
	err := dec.Decode(&wire)
	return wire, err
}

func (i *Invocation) mustWire() invocationWire {
	// raw was validated at construction (Prepare marshals it; InvocationFromJSON decodes it),
	// so a failure here is unreachable; the zero value then fails every check downstream.
	wire, err := i.wire()
	if err != nil {
		return invocationWire{}
	}
	return wire
}

// OperationID is the stable id of this logical invocation.
func (i *Invocation) OperationID() string { return i.mustWire().OperationID }

// AggregateID is the aggregate the request was prepared against.
func (i *Invocation) AggregateID() string { return i.mustWire().AggregateID }

// Machine is the identity the request was prepared for.
func (i *Invocation) Machine() MachineIdentity {
	m := i.mustWire().Machine
	return NewIdentity(m.MachineID, m.Version, m.SpecHash)
}

// Command returns a fresh copy of the bound command; mutating it changes nothing.
func (i *Invocation) Command() map[string]any { return i.mustWire().Command }

// InvocationFromJSON restores a persisted invocation. Unknown fields, unknown versions, other
// kinds and empty ids are refused.
func InvocationFromJSON(data []byte) (*Invocation, error) {
	inv := &Invocation{raw: append([]byte(nil), data...)}
	var wire invocationWire
	dec := json.NewDecoder(bytes.NewReader(inv.raw))
	dec.DisallowUnknownFields()
	dec.UseNumber()
	if err := dec.Decode(&wire); err != nil {
		return nil, fmt.Errorf("invalid invocation: %w", err)
	}
	if wire.Version != InvocationWireVersion {
		return nil, errors.New("unknown invocation_version")
	}
	if wire.Kind != "command" {
		return nil, errors.New("this client restores command invocations only")
	}
	if wire.OperationID == "" || wire.AggregateID == "" || wire.Command == nil {
		return nil, errors.New("an invocation needs an operation id, an aggregate id and a command")
	}
	return inv, nil
}

func (m *Machine) checkInvocation(inv *Invocation) (invocationWire, error) {
	if inv == nil {
		return invocationWire{}, errors.New("nil invocation")
	}
	wire := inv.mustWire()
	if inv.Machine() != m.Identity {
		return wire, errors.New("this invocation was prepared for a different machine identity than this Machine")
	}
	return wire, nil
}

func (w invocationWire) capability() Capability {
	return Capability{generation: w.Capability.Generation, nonce: w.Capability.Nonce}
}

// Dispatch sends a prepared invocation exactly once under its own operation id and capability.
// No retry and no handle refresh. On failure, Reconcile with the same invocation; never resend
// under a new id. Exactly one of the first two results is non-nil on success.
func (m *Machine) Dispatch(ctx context.Context, inv *Invocation) (*Aggregate, *Historical, error) {
	wire, err := m.checkInvocation(inv)
	if err != nil {
		return nil, nil, err
	}
	result, err := m.kernel.call(ctx, "command", m, map[string]any{
		"aggregate_id": wire.AggregateID,
		"capability":   wire.capability().wire(),
		"command":      wire.Command,
		"operation_id": wire.OperationID,
	})
	if err != nil {
		return nil, nil, annotateOperation(err, wire.OperationID)
	}
	if historical, _ := result["historical"].(bool); historical {
		record, _ := result["operation"].(map[string]any)
		current, _ := result["current"].(map[string]any)
		return nil, historicalFromRecord(wire.AggregateID, record, current["generation"], current["sequence"], result), nil
	}
	next, err := aggregateFromResult(m, result)
	if err != nil {
		return nil, nil, annotateOperation(err, wire.OperationID)
	}
	return next, nil, nil
}

// Reconcile is an exact lookup of a prepared invocation: did THIS operation id commit under THIS
// request? "committed_exact" is history, not a current handle; "conflict" means the id was used
// for a different request; "not_recorded" is not permission to resend under a refreshed handle.
func (m *Machine) Reconcile(ctx context.Context, inv *Invocation) (*OperationLookup, error) {
	wire, err := m.checkInvocation(inv)
	if err != nil {
		return nil, err
	}
	result, err := m.kernel.call(ctx, "lookup_operation", m, map[string]any{
		"aggregate_id": wire.AggregateID,
		"capability":   wire.capability().wire(),
		"command":      wire.Command,
		"operation_id": wire.OperationID,
	})
	if err != nil {
		return nil, annotateOperation(err, wire.OperationID)
	}
	return lookupFromResult(wire.AggregateID, result), nil
}
