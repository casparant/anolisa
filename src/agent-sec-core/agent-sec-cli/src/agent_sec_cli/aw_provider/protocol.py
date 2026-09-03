"""Native wire protocol for the AW Provider entrypoint.

This protocol is deliberately separate from the canonical AW Capability
Contracts. The Provider Host connects the two with a declarative
``json-map/v1`` codec, so this module may keep a shape that suits the local
scanners without leaking their options into the public Capability.

Two invariants must hold for every response, in every disposition:

1. ``findings_total`` and ``scanned_bytes`` are always present. The Host
   resolves every declared meter pointer regardless of disposition, so a
   missing counter becomes an invalid-response failure rather than a bypass.
2. No field carries matched content. A finding reports which rule fired and
   how often, never the value it matched.
"""

from enum import StrEnum

from pydantic import BaseModel, Field

PROTOCOL_VERSION = 1
MAX_FINDINGS = 64
MAX_REASONS = 32
MAX_RULE_ID_BYTES = 64


class Operation(StrEnum):
    """Capability this invocation is expected to fulfil."""

    CONTENT_INSPECT = "content_inspect"
    CODE_INSPECT = "code_inspect"
    COMMAND_INSPECT = "command_inspect"


class Disposition(StrEnum):
    """Terminal outcome of one invocation.

    The Host maps ``completed`` to ``produced``, ``skipped`` to ``bypassed``,
    and ``error`` to ``failed``. All three are protocol successes and exit 0.
    """

    COMPLETED = "completed"
    SKIPPED = "skipped"
    ERROR = "error"


class RequestLanguage(StrEnum):
    """Source language the caller believes applies."""

    AUTO = "auto"
    BASH = "bash"
    PYTHON = "python"


class DetectedLanguage(StrEnum):
    """Source language the scanner actually analysed."""

    BASH = "bash"
    PYTHON = "python"
    UNKNOWN = "unknown"


class InspectionVerdict(StrEnum):
    """Conclusion of a content or code inspection."""

    CLEAN = "clean"
    SUSPICIOUS = "suspicious"
    SENSITIVE = "sensitive"


class CommandVerdict(StrEnum):
    """Conclusion of a pending-command inspection."""

    ALLOW = "allow"
    WARN = "warn"
    DENY = "deny"


class FindingCategory(StrEnum):
    """Broad class of a finding."""

    SECRET = "secret"
    PERSONAL_DATA = "personal_data"
    CREDENTIAL = "credential"
    DANGEROUS_PATTERN = "dangerous_pattern"
    OBFUSCATION = "obfuscation"
    OTHER = "other"


class FindingSeverity(StrEnum):
    """Severity attached to a finding class."""

    INFO = "info"
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"
    CRITICAL = "critical"


class FindingConfidence(StrEnum):
    """Confidence attached to a finding class."""

    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"


class ProviderRequest(BaseModel):
    """One native request read from standard input."""

    model_config = {"extra": "forbid"}

    protocol_version: int
    operation: Operation
    content: str
    source: str = "unknown"
    include_low_confidence: bool = False
    language: RequestLanguage = RequestLanguage.AUTO


class ProviderFinding(BaseModel):
    """Content-free count of one finding class."""

    model_config = {"extra": "forbid"}

    rule_id: str = Field(max_length=MAX_RULE_ID_BYTES)
    category: FindingCategory
    severity: FindingSeverity
    confidence: FindingConfidence
    count: int = Field(ge=0)


class ProviderResponse(BaseModel):
    """One native response written to standard output."""

    model_config = {"extra": "forbid"}

    protocol_version: int = PROTOCOL_VERSION
    disposition: Disposition
    findings_total: int = Field(default=0, ge=0)
    scanned_bytes: int = Field(default=0, ge=0)
    truncated: bool = False
    verdict: str | None = None
    findings: list[ProviderFinding] = Field(default_factory=list)
    reasons: list[str] = Field(default_factory=list)
    language_detected: DetectedLanguage | None = None
    engine: str | None = None
