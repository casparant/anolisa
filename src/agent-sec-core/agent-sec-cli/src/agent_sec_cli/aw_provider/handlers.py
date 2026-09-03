"""Side-effect-free adapters from local scanners to the AW native protocol.

These handlers deliberately bypass ``security_middleware.invoke`` and its
lifecycle. That path writes a SecurityEvent to JSONL and SQLite and emits
telemetry on every call, which would contradict the Provider manifest's
``writes = []`` / ``retention = "none"`` / ``telemetry = "disabled"``
declarations. ``skill-ledger analyze`` established the same precedent.

Two further constraints follow from the AW execution model:

* Custom PII rules are disabled. Loading them reads a file under the user's
  configuration directory, which the manifest does not declare, and which
  would silently change behaviour under a cleared environment.
* Findings never carry matched content. Only the rule identity, its
  classification, and a count cross the boundary.
"""

from collections import Counter

from agent_sec_cli.aw_provider.protocol import (
    MAX_FINDINGS,
    MAX_REASONS,
    MAX_RULE_ID_BYTES,
    CommandVerdict,
    DetectedLanguage,
    Disposition,
    FindingCategory,
    FindingConfidence,
    FindingSeverity,
    InspectionVerdict,
    Operation,
    ProviderFinding,
    ProviderRequest,
    ProviderResponse,
    RequestLanguage,
)
from agent_sec_cli.code_scanner import scanner as code_scanner
from agent_sec_cli.code_scanner.models import Language as CodeLanguage
from agent_sec_cli.code_scanner.models import ScanResult as CodeScanResult
from agent_sec_cli.code_scanner.models import Verdict as CodeVerdict
from agent_sec_cli.pii_checker.detectors.regex import RegexPiiDetector
from agent_sec_cli.pii_checker.models import PiiScanResult
from agent_sec_cli.pii_checker.models import Verdict as PiiVerdict
from agent_sec_cli.pii_checker.scanner import PiiScanner

_RULE_ID_ALLOWED = set("abcdefghijklmnopqrstuvwxyz0123456789._-")

_PII_CATEGORIES = {
    "personal_data": FindingCategory.PERSONAL_DATA,
    "credential": FindingCategory.CREDENTIAL,
}

_SEVERITIES = {
    "warn": FindingSeverity.MEDIUM,
    "deny": FindingSeverity.HIGH,
}


def handle(request: ProviderRequest) -> ProviderResponse:
    """Dispatches one native request to its scanner and normalizes the result."""
    if request.operation is Operation.CONTENT_INSPECT:
        return _content_inspect(request)
    if request.operation is Operation.CODE_INSPECT:
        return _code_inspect(request)
    return _command_inspect(request)


def _content_inspect(request: ProviderRequest) -> ProviderResponse:
    """Reports secret and personal-data findings in model-visible content."""
    scanner = PiiScanner(detectors=[RegexPiiDetector()])
    result: PiiScanResult = scanner.scan(
        request.content,
        source=request.source,
        include_low_confidence=request.include_low_confidence,
        raw_evidence=False,
        redact_output=False,
    )
    if result.verdict == PiiVerdict.ERROR.value:
        return _failed(scanned_bytes=_summary_bytes(result))

    findings = _pii_findings(result)
    return ProviderResponse(
        disposition=Disposition.COMPLETED,
        verdict=_inspection_verdict(result.verdict).value,
        findings=findings,
        findings_total=sum(finding.count for finding in findings),
        scanned_bytes=_summary_bytes(result),
        truncated=bool(result.summary.get("truncated", False)),
        engine="pii-regex",
    )


def _code_inspect(request: ProviderRequest) -> ProviderResponse:
    """Reports dangerous constructs in code-bearing content."""
    result = _scan_code(request)
    if result.verdict is CodeVerdict.ERROR:
        return _failed(scanned_bytes=len(request.content.encode("utf-8")))

    findings = _code_findings(result)
    return ProviderResponse(
        disposition=Disposition.COMPLETED,
        verdict=_inspection_verdict(result.verdict.value).value,
        findings=findings,
        findings_total=sum(finding.count for finding in findings),
        scanned_bytes=len(request.content.encode("utf-8")),
        language_detected=_detected_language(result.language),
        engine=f"code-regex-{result.engine_version}",
    )


def _command_inspect(request: ProviderRequest) -> ProviderResponse:
    """Returns a gate verdict for a command that has not run yet."""
    result = _scan_code(request)
    if result.verdict is CodeVerdict.ERROR:
        return _failed(scanned_bytes=len(request.content.encode("utf-8")))

    findings = _code_findings(result)
    reasons = []
    for finding in findings:
        if finding.rule_id not in reasons:
            reasons.append(finding.rule_id)
    return ProviderResponse(
        disposition=Disposition.COMPLETED,
        verdict=_command_verdict(result.verdict).value,
        findings=findings,
        reasons=reasons[:MAX_REASONS],
        findings_total=sum(finding.count for finding in findings),
        scanned_bytes=len(request.content.encode("utf-8")),
        language_detected=_detected_language(result.language),
        engine=f"code-regex-{result.engine_version}",
    )


def _scan_code(request: ProviderRequest) -> CodeScanResult:
    """Scans content with the regex engine, never the network-backed engine.

    ``auto`` resolves to Bash because the Bash path additionally extracts
    inline interpreter payloads, so it also covers Python embedded in a shell
    command. The scanner reports which language it settled on.
    """
    language = (
        CodeLanguage.PYTHON if request.language is RequestLanguage.PYTHON else CodeLanguage.BASH
    )
    return code_scanner.scan(request.content, language, mode="regex")


def _failed(*, scanned_bytes: int) -> ProviderResponse:
    """Builds a settled failure that carries no verdict and no findings."""
    return ProviderResponse(
        disposition=Disposition.ERROR,
        findings_total=0,
        scanned_bytes=scanned_bytes,
    )


def _summary_bytes(result: PiiScanResult) -> int:
    """Returns the byte count the PII scanner reported inspecting."""
    value = result.summary.get("bytes_scanned", 0)
    return value if isinstance(value, int) and value >= 0 else 0


def _pii_findings(result: PiiScanResult) -> list[ProviderFinding]:
    """Aggregates PII findings into content-free per-class counts."""
    counter: Counter[tuple[str, FindingCategory, FindingSeverity, FindingConfidence]] = Counter()
    for finding in result.findings:
        counter[
            (
                _rule_id(finding.type),
                _pii_category(finding.category),
                _severity(finding.severity),
                _confidence(finding.confidence),
            )
        ] += 1
    return _to_findings(counter)


def _code_findings(result: CodeScanResult) -> list[ProviderFinding]:
    """Aggregates code findings into content-free per-class counts.

    ``Finding.evidence`` holds the matched source lines and is deliberately
    dropped here; only the count of matches survives.
    """
    counter: Counter[tuple[str, FindingCategory, FindingSeverity, FindingConfidence]] = Counter()
    for finding in result.findings:
        key = (
            _rule_id(finding.rule_id),
            FindingCategory.DANGEROUS_PATTERN,
            _severity(finding.severity.value),
            FindingConfidence.HIGH,
        )
        counter[key] += max(len(finding.evidence), 1)
    return _to_findings(counter)


def _to_findings(
    counter: Counter[tuple[str, FindingCategory, FindingSeverity, FindingConfidence]],
) -> list[ProviderFinding]:
    """Returns deterministically ordered findings within the declared bound."""
    ordered = sorted(counter.items(), key=lambda item: item[0][0])
    return [
        ProviderFinding(
            rule_id=rule_id,
            category=category,
            severity=severity,
            confidence=confidence,
            count=count,
        )
        for (rule_id, category, severity, confidence), count in ordered[:MAX_FINDINGS]
    ]


def _rule_id(raw: str) -> str:
    """Normalizes a scanner rule name to the AW rule-identity character set.

    The AW Contract restricts a rule identity to ``[a-z0-9._-]`` so it cannot
    become a channel for matched content. Rule names are authored labels, so
    folding case and replacing separators loses no security meaning.
    """
    normalized = "".join(
        character if character in _RULE_ID_ALLOWED else "." for character in raw.lower()
    )
    trimmed = normalized.strip(".")[:MAX_RULE_ID_BYTES]
    return trimmed or "unnamed"


def _pii_category(raw: str) -> FindingCategory:
    """Maps a PII category to its AW class."""
    return _PII_CATEGORIES.get(raw, FindingCategory.OTHER)


def _severity(raw: str) -> FindingSeverity:
    """Maps a scanner severity to its AW class."""
    return _SEVERITIES.get(raw, FindingSeverity.MEDIUM)


def _confidence(score: float) -> FindingConfidence:
    """Buckets a numeric confidence into the AW confidence class."""
    if score < 0.5:
        return FindingConfidence.LOW
    if score < 0.8:
        return FindingConfidence.MEDIUM
    return FindingConfidence.HIGH


def _inspection_verdict(raw: str) -> InspectionVerdict:
    """Maps a scanner verdict to an inspection conclusion."""
    if raw == "deny":
        return InspectionVerdict.SENSITIVE
    if raw == "warn":
        return InspectionVerdict.SUSPICIOUS
    return InspectionVerdict.CLEAN


def _command_verdict(verdict: CodeVerdict) -> CommandVerdict:
    """Maps a scanner verdict to a Tool Call gate verdict."""
    if verdict is CodeVerdict.DENY:
        return CommandVerdict.DENY
    if verdict is CodeVerdict.WARN:
        return CommandVerdict.WARN
    return CommandVerdict.ALLOW


def _detected_language(language: CodeLanguage) -> DetectedLanguage:
    """Maps the scanner's resolved language to the AW reporting enum."""
    if language is CodeLanguage.PYTHON:
        return DetectedLanguage.PYTHON
    if language is CodeLanguage.BASH:
        return DetectedLanguage.BASH
    return DetectedLanguage.UNKNOWN
