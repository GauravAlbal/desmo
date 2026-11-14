package invariantkernel

import "fmt"

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
