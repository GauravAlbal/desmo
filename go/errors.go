package invariantkernel

import (
	"errors"
	"fmt"
	"strings"
)

// KernelError is a kernel-authored refusal. Code is the stable IK_* / domain string.
type KernelError struct {
	Code       string
	Rule       string
	Message    string
	Diagnostic map[string]any
	// OperationID is the caller-correlatable id of the logical invocation, when bound to one.
	OperationID string
}

func (e *KernelError) Error() string {
	if e == nil {
		return ""
	}
	if e.Rule != "" {
		return fmt.Sprintf("%s rule=%s: %s", e.Code, e.Rule, e.Message)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

func (e *KernelError) Is(target error) bool {
	t, ok := target.(*KernelError)
	if !ok || e == nil || t == nil {
		return false
	}
	if t.Code == "" {
		return true
	}
	return e.Code == t.Code
}

// TransportError is process/timeout/malformed-output failure, not a domain diagnostic.
type TransportError struct {
	Op      string
	Message string
	Err     error
	// Unsent is true only when the request was provably never dispatched (for example the
	// `ik` binary is missing). Otherwise the request may have committed: reconcile with
	// Machine.LookupOperation using OperationID.
	Unsent bool
	// OperationID is the id of the logical invocation, when bound to one.
	OperationID string
}

func (e *TransportError) Error() string {
	if e.Op != "" {
		return fmt.Sprintf("ik transport (%s): %s", e.Op, e.Message)
	}
	return "ik transport: " + e.Message
}

func (e *TransportError) Unwrap() error { return e.Err }

type (
	ProtocolError        struct{ *KernelError }
	MachineError         struct{ *KernelError }
	StaleCapabilityError struct{ *KernelError }
	NonceInvalidError    struct{ *KernelError }
	IllegalStateError    struct{ *KernelError }
	CommandInvalidError  struct{ *KernelError }
	InvariantFailedError struct{ *KernelError }
	EffectError          struct{ *KernelError }
	EffectStaleError     struct{ *KernelError }
	EffectMismatchError  struct{ *KernelError }
	EffectConflictError  struct{ *KernelError }
	// OperationConflictError is IK_OPERATION_CONFLICT: the operation id already committed
	// with different semantics (or a refreshed capability).
	OperationConflictError struct{ *KernelError }
	// StoreNotFoundError is IK_STORE_NOT_FOUND: the store file does not exist.
	// It is not corruption.
	StoreNotFoundError struct{ *KernelError }
)

func (e *ProtocolError) Error() string          { return e.KernelError.Error() }
func (e *MachineError) Error() string           { return e.KernelError.Error() }
func (e *StaleCapabilityError) Error() string   { return e.KernelError.Error() }
func (e *NonceInvalidError) Error() string      { return e.KernelError.Error() }
func (e *IllegalStateError) Error() string      { return e.KernelError.Error() }
func (e *CommandInvalidError) Error() string    { return e.KernelError.Error() }
func (e *InvariantFailedError) Error() string   { return e.KernelError.Error() }
func (e *EffectError) Error() string            { return e.KernelError.Error() }
func (e *EffectStaleError) Error() string       { return e.KernelError.Error() }
func (e *OperationConflictError) Error() string { return e.KernelError.Error() }
func (e *OperationConflictError) Unwrap() error { return e.KernelError }
func (e *StoreNotFoundError) Error() string     { return e.KernelError.Error() }
func (e *StoreNotFoundError) Unwrap() error     { return e.KernelError }
func (e *EffectMismatchError) Error() string    { return e.KernelError.Error() }
func (e *EffectConflictError) Error() string    { return e.KernelError.Error() }

func (e *ProtocolError) Unwrap() error        { return e.KernelError }
func (e *MachineError) Unwrap() error         { return e.KernelError }
func (e *StaleCapabilityError) Unwrap() error { return e.KernelError }
func (e *NonceInvalidError) Unwrap() error    { return e.KernelError }
func (e *IllegalStateError) Unwrap() error    { return e.KernelError }
func (e *CommandInvalidError) Unwrap() error  { return e.KernelError }
func (e *InvariantFailedError) Unwrap() error { return e.KernelError }
func (e *EffectError) Unwrap() error          { return e.KernelError }
func (e *EffectStaleError) Unwrap() error     { return e.KernelError }
func (e *EffectMismatchError) Unwrap() error  { return e.KernelError }
func (e *EffectConflictError) Unwrap() error  { return e.KernelError }

func fromDiagnostic(d map[string]any) error {
	code, _ := d["code"].(string)
	if code == "" {
		code = "IK_INTERNAL"
	}
	msg, _ := d["message"].(string)
	rule, _ := d["rule"].(string)
	base := &KernelError{Code: code, Rule: rule, Message: msg, Diagnostic: d}
	switch code {
	case "IK_PROTOCOL_INVALID", "IK_PROTOCOL_VERSION_UNKNOWN":
		return &ProtocolError{base}
	case "IK_MACHINE_UNKNOWN", "IK_MACHINE_MISMATCH", "IK_MACHINE_VERSION_UNKNOWN", "IK_MACHINE_HASH_MISMATCH":
		return &MachineError{base}
	case "IK_CAPABILITY_STALE":
		return &StaleCapabilityError{base}
	case "IK_NONCE_INVALID":
		return &NonceInvalidError{base}
	case "IK_ILLEGAL_STATE":
		return &IllegalStateError{base}
	case "IK_COMMAND_INVALID":
		return &CommandInvalidError{base}
	case "IK_INVARIANT_FAILED":
		return &InvariantFailedError{base}
	case "IK_EFFECT_STALE":
		return &EffectStaleError{base}
	case "IK_EFFECT_MISMATCH":
		return &EffectMismatchError{base}
	case "IK_EFFECT_OUTCOME_CONFLICT":
		return &EffectConflictError{base}
	case "IK_OPERATION_CONFLICT":
		return &OperationConflictError{base}
	case "IK_STORE_NOT_FOUND":
		return &StoreNotFoundError{base}
	default:
		if strings.HasPrefix(code, "IK_EFFECT_") {
			return &EffectError{base}
		}
		return base
	}
}

// FailureClass is the stable domain-adapter vocabulary.
// historical is a result (*Historical), not an error.
type FailureClass string

const (
	ClassNotSent          FailureClass = "not_sent"
	ClassRefused          FailureClass = "refused"
	ClassContractMismatch FailureClass = "contract_mismatch"
	ClassNotFound         FailureClass = "not_found"
	ClassCorrupt          FailureClass = "corrupt"
	ClassHistorical       FailureClass = "historical"
	ClassUncertain        FailureClass = "uncertain"
)

// Classify reports the domain-adapter class of a caught error, or of a historical
// result. Pass a nil error and a non-nil historical for ClassHistorical. A
// historical result is not an error. Rule and Diagnostic on the error are not modified.
func Classify(err error, historical *Historical) (FailureClass, error) {
	if historical != nil {
		if err != nil {
			return "", fmt.Errorf("classify: historical result and error are mutually exclusive")
		}
		return ClassHistorical, nil
	}
	if err == nil {
		return "", fmt.Errorf("classify: nil error is not a failure")
	}
	return classifyError(err)
}

func classifyError(err error) (FailureClass, error) {
	var transport *TransportError
	if errors.As(err, &transport) {
		if transport.Unsent {
			return ClassNotSent, nil
		}
		return ClassUncertain, nil
	}
	var kernel *KernelError
	if !errors.As(err, &kernel) {
		return "", fmt.Errorf("classify: %v is not a kernel client error", err)
	}
	return classifyKernel(kernel), nil
}

func classifyKernel(err *KernelError) FailureClass {
	if err.Code == "IK_STORE_NOT_FOUND" || notFoundRule(err.Rule) {
		return ClassNotFound
	}
	if corruptCode(err.Code) {
		return ClassCorrupt
	}
	if contractCode(err.Code) {
		return ClassContractMismatch
	}
	return ClassRefused
}

func notFoundRule(rule string) bool {
	return rule == "store.missing" || rule == "aggregate.not_found"
}

func corruptCode(code string) bool {
	switch code {
	case "IK_STORE_CORRUPT", "IK_JOURNAL_CORRUPT", "IK_PROJECTION_CORRUPT", "IK_REPLAY_MISMATCH":
		return true
	default:
		return false
	}
}

func contractCode(code string) bool {
	switch code {
	case "IK_EVENT_SCHEMA_UNKNOWN", "IK_PERSISTED_SCHEMA_UNKNOWN":
		return true
	default:
		return strings.HasPrefix(code, "IK_MACHINE_") || strings.HasPrefix(code, "IK_PROTOCOL_")
	}
}
