"""Kernel diagnostics as native exceptions. Codes stay authoritative."""

from __future__ import annotations

from typing import Any, Mapping, Optional, Union

from .types import HistoricalResult


class KernelError(Exception):
    """Any kernel-authored refusal or client transport failure.

    ``submission`` says what is known about the request: ``"refused"`` (the kernel answered
    with a diagnostic), ``"not-sent"`` (provably never dispatched), or ``"submission-unknown"``
    (it may or may not have committed). ``operation_id`` is the caller-correlatable id of the
    logical invocation, when the call was bound to one.
    """

    submission = "refused"
    operation_id: Optional[str] = None

    def __init__(
        self,
        code: str,
        message: str,
        rule: Optional[str] = None,
        diagnostic: Optional[Mapping[str, Any]] = None,
    ) -> None:
        self.code = code
        self.rule = rule
        self.message = message
        self.diagnostic = dict(diagnostic) if diagnostic is not None else {
            "code": code,
            "message": message,
            **({"rule": rule} if rule is not None else {}),
        }
        super().__init__(self._fmt())

    def _fmt(self) -> str:
        if self.rule:
            return f"{self.code} rule={self.rule}: {self.message}"
        return f"{self.code}: {self.message}"


class TransportError(KernelError):
    """Process, timeout, or malformed-output failure after dispatch. Not a domain diagnostic.

    The request may have committed: reconcile with ``Machine.lookup_operation`` using the
    same ``operation_id``, never by blindly repeating under a refreshed handle."""

    submission = "submission-unknown"


class PreDispatchError(TransportError):
    """The request was provably never sent (for example the ``ik`` binary is missing)."""

    submission = "not-sent"


class OperationConflictError(KernelError):
    """IK_OPERATION_CONFLICT — the operation id was already committed with different semantics."""


class ProtocolError(KernelError):
    """IK_PROTOCOL_* refusals."""


class MachineError(KernelError):
    """Unknown / mismatched machine identity."""


class StaleCapabilityError(KernelError):
    """IK_CAPABILITY_STALE — the handle's capability is no longer current."""


class NonceInvalidError(KernelError):
    """IK_NONCE_INVALID."""


class IllegalStateError(KernelError):
    """IK_ILLEGAL_STATE."""


class CommandInvalidError(KernelError):
    """IK_COMMAND_INVALID."""


class InvariantFailedError(KernelError):
    """IK_INVARIANT_FAILED."""


class EffectError(KernelError):
    """Effect-identity refusals."""


class EffectStaleError(EffectError):
    """IK_EFFECT_STALE."""


class EffectMismatchError(EffectError):
    """IK_EFFECT_MISMATCH."""


class EffectConflictError(EffectError):
    """IK_EFFECT_OUTCOME_CONFLICT."""


class EffectCompletionError(KernelError):
    """The outcome was not sent: empty digest, or the intent is not pending.

    ``submission`` is ``not-sent``. This is not a lost reply and must not be retried
    as if the kernel might have committed it.
    """

    submission = "not-sent"

    def __init__(self, message: str) -> None:
        super().__init__("CLIENT_EFFECT_COMPLETION", message)


class StoreNotFoundError(KernelError):
    """IK_STORE_NOT_FOUND — the store file does not exist. Not corruption."""


_NOT_FOUND_RULES = frozenset({"store.missing", "aggregate.not_found"})
_CORRUPT_CODES = frozenset({
    "IK_STORE_CORRUPT",
    "IK_JOURNAL_CORRUPT",
    "IK_PROJECTION_CORRUPT",
    "IK_REPLAY_MISMATCH",
})
_CONTRACT_CODES = frozenset({
    "IK_EVENT_SCHEMA_UNKNOWN",
    "IK_PERSISTED_SCHEMA_UNKNOWN",
})
# Stable domain-adapter vocabulary. ``historical`` is a result, not an error.
FAILURE_CLASSES = (
    "not_sent",
    "refused",
    "contract_mismatch",
    "not_found",
    "corrupt",
    "historical",
    "uncertain",
)


def classify(value: Union[BaseException, HistoricalResult]) -> str:
    """Classify a caught client error, or a ``HistoricalResult``.

    Returns one of ``FAILURE_CLASSES``. ``historical`` is returned only for a
    ``HistoricalResult``; it is not an error. Code, rule, and the diagnostic
    mapping on the error are left intact.
    """
    if isinstance(value, HistoricalResult):
        return "historical"
    if not isinstance(value, KernelError):
        raise TypeError("classify expects a kernel client error or HistoricalResult")
    return _classify_error(value)


def _classify_error(error: KernelError) -> str:
    if isinstance(error, PreDispatchError) or error.submission == "not-sent":
        return "not_sent"
    if isinstance(error, TransportError) or error.submission == "submission-unknown":
        return "uncertain"
    if error.code == "IK_STORE_NOT_FOUND" or error.rule in _NOT_FOUND_RULES:
        return "not_found"
    if error.code in _CORRUPT_CODES:
        return "corrupt"
    if error.code in _CONTRACT_CODES or error.code.startswith(("IK_MACHINE_", "IK_PROTOCOL_")):
        return "contract_mismatch"
    return "refused"


_CODE_TYPES: dict[str, type[KernelError]] = {
    "IK_PROTOCOL_INVALID": ProtocolError,
    "IK_PROTOCOL_VERSION_UNKNOWN": ProtocolError,
    "IK_MACHINE_UNKNOWN": MachineError,
    "IK_MACHINE_MISMATCH": MachineError,
    "IK_MACHINE_VERSION_UNKNOWN": MachineError,
    "IK_MACHINE_HASH_MISMATCH": MachineError,
    "IK_CAPABILITY_STALE": StaleCapabilityError,
    "IK_NONCE_INVALID": NonceInvalidError,
    "IK_ILLEGAL_STATE": IllegalStateError,
    "IK_COMMAND_INVALID": CommandInvalidError,
    "IK_INVARIANT_FAILED": InvariantFailedError,
    "IK_EFFECT_STALE": EffectStaleError,
    "IK_EFFECT_MISMATCH": EffectMismatchError,
    "IK_EFFECT_OUTCOME_CONFLICT": EffectConflictError,
    "IK_OPERATION_CONFLICT": OperationConflictError,
    "IK_STORE_NOT_FOUND": StoreNotFoundError,
}


def raise_diagnostic(diagnostic: Mapping[str, Any]) -> None:
    code = str(diagnostic.get("code") or "IK_INTERNAL")
    message = str(diagnostic.get("message") or "")
    rule = diagnostic.get("rule")
    rule_s = str(rule) if rule is not None else None
    cls = _CODE_TYPES.get(code, KernelError)
    raise cls(code, message, rule_s, diagnostic)
