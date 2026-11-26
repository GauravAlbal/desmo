package invariantkernel

import (
	"encoding/json"
	"fmt"
	"strconv"
)

// MachineIdentity is the kernel-authoritative (id, version, spec_hash) triple.
// SpecHash is carried for the wire; callers should not invent one.
type MachineIdentity struct {
	MachineID string
	Version   uint64
	specHash  string
}

func (m MachineIdentity) SpecHash() string { return m.specHash }

// NewIdentity builds a machine identity from kernel-authoritative fields.
// Callers must not invent specHash.
func NewIdentity(machineID string, version uint64, specHash string) MachineIdentity {
	return MachineIdentity{MachineID: machineID, Version: version, specHash: specHash}
}

func (m MachineIdentity) wire() map[string]any {
	return map[string]any{
		"machine_id": m.MachineID,
		"version":    m.Version,
		"spec_hash":  m.specHash,
	}
}

func identityFromWire(v map[string]any) (MachineIdentity, error) {
	id, _ := v["machine_id"].(string)
	hash, _ := v["spec_hash"].(string)
	var version uint64
	switch n := v["version"].(type) {
	case json.Number:
		parsed, err := strconv.ParseUint(n.String(), 10, 64)
		if err != nil {
			return MachineIdentity{}, &TransportError{Message: "identity has invalid version"}
		}
		version = parsed
	case float64:
		version = uint64(n)
	case int:
		version = uint64(n)
	case int64:
		version = uint64(n)
	case uint64:
		version = n
	default:
		return MachineIdentity{}, &TransportError{Message: "identity missing version"}
	}
	if id == "" || hash == "" {
		return MachineIdentity{}, &TransportError{Message: "identity missing machine_id or spec_hash"}
	}
	return MachineIdentity{MachineID: id, Version: version, specHash: hash}, nil
}

// Capability is opaque mutation authority. Do not construct or thread this by hand.
type Capability struct {
	generation uint64
	nonce      string
}

func (c Capability) wire() map[string]any {
	return map[string]any{"generation": c.generation, "nonce": c.nonce}
}

func capabilityFromWire(v map[string]any) (Capability, error) {
	nonce, _ := v["nonce"].(string)
	var gen uint64
	switch n := v["generation"].(type) {
	case json.Number:
		parsed, err := strconv.ParseUint(n.String(), 10, 64)
		if err != nil {
			return Capability{}, &TransportError{Message: "result has invalid capability.generation"}
		}
		gen = parsed
	case float64:
		gen = uint64(n)
	case int:
		gen = uint64(n)
	case int64:
		gen = uint64(n)
	case uint64:
		gen = n
	default:
		return Capability{}, &TransportError{Message: "result missing capability.generation"}
	}
	if nonce == "" {
		return Capability{}, &TransportError{Message: "result missing capability.nonce"}
	}
	return Capability{generation: gen, nonce: nonce}, nil
}

func (c Capability) String() string { return "Capability(<opaque>)" }

// Receipt is a committed transition receipt.
type Receipt struct {
	ReceiptID   string
	AggregateID string
	Sequence    uint64
	Generation  uint64
	Raw         map[string]any
}

func receiptFromWire(v map[string]any) *Receipt {
	if v == nil {
		return nil
	}
	return &Receipt{
		ReceiptID:   asString(v["receipt_id"]),
		AggregateID: asString(v["aggregate_id"]),
		Sequence:    asUint(v["sequence"]),
		Generation:  asUint(v["generation"]),
		Raw:         v,
	}
}

// EffectIntent is a pending or recorded effect identity.
type EffectIntent struct {
	EffectID       string
	AggregateID    string
	Generation     uint64
	EffectKind     string
	PayloadDigest  string
	IdempotencyKey string
	Payload        any
}

// Effect is the runtime effect status: none / pending / recorded.
type Effect struct {
	Status        string
	Intent        *EffectIntent
	OutcomeDigest string
	ReceiptID     string
}

func (e Effect) String() string { return fmt.Sprintf("Effect(status=%q)", e.Status) }

func effectFromWire(v map[string]any) Effect {
	if v == nil {
		return Effect{Status: "none"}
	}
	out := Effect{
		Status:        asString(v["status"]),
		OutcomeDigest: asString(v["outcome_digest"]),
		ReceiptID:     asString(v["receipt_id"]),
	}
	if out.Status == "" {
		out.Status = "none"
	}
	if raw, ok := v["intent"].(map[string]any); ok {
		out.Intent = &EffectIntent{
			EffectID:       asString(raw["effect_id"]),
			AggregateID:    asString(raw["aggregate_id"]),
			Generation:     asUint(raw["generation"]),
			EffectKind:     asString(raw["effect_kind"]),
			PayloadDigest:  asString(raw["payload_digest"]),
			IdempotencyKey: asString(raw["idempotency_key"]),
			Payload:        raw["payload"],
		}
	}
	return out
}

// CheckResult is developer-facing ik check output.
// Verdict is "accept" or "refused". The JSON document's judgment.status
// distinguishes missing_evidence from witnessed_violation; this wrapper
// is text-mode and exposes the verdict only.
type CheckResult struct {
	Text    string
	OK      bool
	Verdict string
}

func asString(v any) string {
	s, _ := v.(string)
	return s
}

func asUint(v any) uint64 {
	switch n := v.(type) {
	case json.Number:
		if parsed, err := strconv.ParseUint(n.String(), 10, 64); err == nil {
			return parsed
		}
		if f, err := n.Float64(); err == nil && f >= 0 {
			return uint64(f)
		}
		return 0
	case float64:
		return uint64(n)
	case int:
		return uint64(n)
	case int64:
		return uint64(n)
	case uint64:
		return n
	default:
		return 0
	}
}

func asBool(v any) bool {
	b, _ := v.(bool)
	return b
}

// EventsOptions configures a read-only `Kernel.Events` query.
type EventsOptions struct {
	Since       uint64
	AggregateID string
	Limit       uint64
}

// EventProjection is the aggregate's current materialized state paired with each event cursor row.
type EventProjection struct {
	State      map[string]any `json:"state"`
	Sequence   uint64         `json:"sequence"`
	Generation uint64         `json:"generation"`
	Terminal   bool           `json:"terminal"`
}

// EventCursorRow is one ordered entry returned by `Kernel.Events`.
type EventCursorRow struct {
	Cursor               uint64          `json:"cursor"`
	AggregateID          string          `json:"aggregate_id"`
	Sequence             uint64          `json:"sequence"`
	Generation           uint64          `json:"generation"`
	MachineID            string          `json:"machine_id"`
	MachineVersion       uint64          `json:"machine_version"`
	MachineSpecHash      string          `json:"machine_spec_hash"`
	EventKind            string          `json:"event_kind"`
	EventType            string          `json:"event_type"`
	EventVersion         uint64          `json:"event_version"`
	Command              any             `json:"command"`
	StateDigest          string          `json:"state_digest"`
	Terminal             bool            `json:"terminal"`
	EffectDelta          any             `json:"effect_delta"`
	EventHash            string          `json:"event_hash"`
	PrevEventHash        string          `json:"prev_event_hash"`
	IsLatestForAggregate bool            `json:"is_latest_for_aggregate"`
	CurrentProjection    EventProjection `json:"current_projection"`
}

// EventFeed is the paginated response from `ik events <store> --json`.
type EventFeed struct {
	Schema     string           `json:"schema"`
	Store      string           `json:"store"`
	Since      uint64           `json:"since"`
	NextCursor uint64           `json:"next_cursor"`
	HasMore    bool             `json:"has_more"`
	Events     []EventCursorRow `json:"events"`
}

// ListOptions configures a read-only `Kernel.List` or `Machine.List` query.
type ListOptions struct {
	MachineID string
	State     string
	Effect    string
	Terminal  *bool
	After     string
	Limit     uint64
}

// AggregateSummary is one materialized aggregate row returned by `Kernel.List`.
type AggregateSummary struct {
	AggregateID     string
	MachineID       string
	MachineVersion  uint64
	MachineSpecHash string
	State           any
	StateName       string
	Terminal        bool
	Generation      uint64
	Sequence        uint64
	Effect          Effect
	EffectStatus    string
}

// AggregateList is the paginated response from `ik list <store> --json`.
type AggregateList struct {
	Schema     string
	Store      string
	NextAfter  string
	HasMore    bool
	Aggregates []AggregateSummary
}
