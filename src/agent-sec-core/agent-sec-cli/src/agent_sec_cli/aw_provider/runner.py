"""Bounded stdin-to-stdout runner for the AW Provider entrypoint.

The AW ``exec-json/v1`` driver contract is narrow and the exit-code rule is the
part most easily got wrong: every protocol-level outcome, including a denied
verdict and a settled scanner failure, exits 0. A non-zero exit means the
process crashed and the Host records a content-free failure receipt instead of
reading standard output.
"""

import json
from typing import IO

from pydantic import ValidationError

from agent_sec_cli.aw_provider.handlers import handle
from agent_sec_cli.aw_provider.protocol import PROTOCOL_VERSION, ProviderRequest

MAX_REQUEST_BYTES = 64 * 1024 * 1024


class ProviderProtocolError(Exception):
    """Raised when standard input is not one usable native request.

    This is the only condition that exits non-zero: the Host cannot be given a
    typed outcome for a request it never managed to express.
    """


def run_provider(stdin: IO[str], stdout: IO[str]) -> None:
    """Reads one native request, writes one native response.

    # Errors

    Raises [`ProviderProtocolError`] when input exceeds the request bound, is
    not JSON, does not satisfy the request schema, or declares an unsupported
    protocol version.
    """
    raw = stdin.read(MAX_REQUEST_BYTES + 1)
    if len(raw) > MAX_REQUEST_BYTES:
        raise ProviderProtocolError(f"request exceeds the {MAX_REQUEST_BYTES}-byte limit")
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ProviderProtocolError(f"request is not valid JSON: {exc.msg}") from exc
    try:
        request = ProviderRequest.model_validate(payload)
    except ValidationError as exc:
        raise ProviderProtocolError(
            f"request does not satisfy the native schema: {exc.error_count()} problems"
        ) from exc
    if request.protocol_version != PROTOCOL_VERSION:
        raise ProviderProtocolError(f"unsupported protocol version {request.protocol_version}")

    response = handle(request)
    json.dump(response.model_dump(mode="json", exclude_none=True), stdout)
    stdout.write("\n")
    stdout.flush()
