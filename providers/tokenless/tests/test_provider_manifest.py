#!/usr/bin/env python3
"""Regression checks for the public AW Provider package."""

from __future__ import annotations

import hashlib
import json
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PROVIDER = ROOT / "provider"


class ProviderManifestTest(unittest.TestCase):
    """Keep the manifest aligned with Tokenless's stable native protocol."""

    @classmethod
    def setUpClass(cls) -> None:
        with (PROVIDER / "provider.toml").open("rb") as stream:
            cls.manifest = tomllib.load(stream)
        with (ROOT / "Cargo.toml").open("rb") as stream:
            cls.cargo = tomllib.load(stream)
        cls.schemas = {
            path.name: json.loads(path.read_text())
            for path in sorted((PROVIDER / "schemas").glob("*.schema.json"))
        }

    def test_identity_and_execution_boundary_are_explicit(self) -> None:
        self.assertEqual(self.manifest["api_version"], "providers.agentic-os.sh/v1")
        self.assertEqual(self.manifest["provider_id"], "tokenless")
        self.assertEqual(
            self.manifest["provider_version"], self.cargo["workspace"]["package"]["version"]
        )
        self.assertEqual(self.manifest["driver"], "exec-json/v1")
        self.assertEqual(self.manifest["lifecycle"], "one_shot")
        self.assertEqual(self.manifest["executable"]["command"], "tokenless")
        self.assertEqual(self.manifest["executable"]["args"], ["compress"])
        self.assertEqual(
            self.manifest["executable"]["environment"],
            {
                "TOKENLESS_COMPRESSION_ENABLED": "1",
                "TOKENLESS_STATS_ENABLED": "0",
                "TOKENLESS_SLS_ENABLED": "0",
            },
        )
        self.assertGreater(self.manifest["limits"]["wall_time_ms"], 0)
        self.assertGreater(self.manifest["limits"]["output_bytes"], 0)
        self.assertEqual(
            self.manifest["permissions"],
            {
                "network": "none",
                "inherit_environment": False,
                "filesystem_read": [],
                "filesystem_write": [],
            },
        )
        self.assertEqual(self.manifest["data"]["reads"], ["model_visible_context"])
        self.assertEqual(self.manifest["data"]["writes"], [])
        self.assertEqual(self.manifest["data"]["retention"], "none")

    def test_capability_and_native_protocol_schemas_are_pinned(self) -> None:
        self.assertEqual(len(self.manifest["capabilities"]), 1)
        capability = self.manifest["capabilities"][0]
        self.assertEqual(capability["capability"], "context.projection.prepare/v1")
        self.assertEqual(capability["authority"], "advise")
        self.assertEqual(capability["scopes"], ["agent_session", "turn", "tool_call"])

        for key in ("input_contract", "output_contract", "native_input", "native_output"):
            reference = capability[key]
            resource = PROVIDER / reference["resource"]
            self.assertTrue(resource.is_file(), f"missing {key} resource")
            self.assertEqual(hashlib.sha256(resource.read_bytes()).hexdigest(), reference["sha256"])

        input_schema = self.schemas["context-projection-prepare-input-v1.schema.json"]
        output_schema = self.schemas["context-projection-prepare-output-v1.schema.json"]
        native_input = self.schemas["tokenless-compression-request-v1.schema.json"]
        native_output = self.schemas["tokenless-compression-response-v1.schema.json"]
        self.assertEqual(
            set(input_schema["required"]),
            {"artifact", "boundary", "constraints"},
        )
        self.assertEqual(
            set(native_input["required"]),
            {"protocol_version", "content", "agent_id", "seam"},
        )
        self.assertEqual(output_schema["required"], ["candidate"])
        self.assertIn("tokenizer_id", native_output["required"])

    def test_disposition_and_meter_mapping_is_complete(self) -> None:
        codec = self.manifest["capabilities"][0]["codec"]
        self.assertEqual(codec["kind"], "json-map/v1")
        request_fields = {field["target"]: field for field in codec["request"]["fields"]}
        self.assertEqual(request_fields["/content"]["source"]["pointer"], "/artifact/content")
        self.assertEqual(request_fields["/agent_id"]["source"], {
            "kind": "scope", "field": "environment_id"
        })
        self.assertEqual(request_fields["/session_id"]["source"]["kind"], "scope")
        self.assertEqual(
            request_fields["/capabilities/publish_retrieve_tool"]["source"],
            {"kind": "const", "value": False},
        )

        response = codec["response"]
        self.assertEqual(response["disposition"]["source"], "/disposition")
        expected = {
            "applied": "produced",
            "dry_run": "bypassed",
            "passthrough": "bypassed",
            "no_savings": "bypassed",
            "reversibility_unavailable": "bypassed",
            "timeout": "failed",
            "error": "failed",
        }
        self.assertEqual(response["disposition"]["values"], expected)
        self.assertTrue(
            all(field["when_disposition"] == ["produced"] for field in response["output_fields"])
        )
        self.assertEqual(
            [meter["method_pointer"] for meter in response["meters"]],
            ["/tokenizer_id", "/tokenizer_id"],
        )
        self.assertTrue(
            all(meter["measurement_kind"] == "estimate" for meter in response["meters"])
        )

    def test_committed_fixture_is_a_real_post_tool_request(self) -> None:
        fixture = json.loads(
            (PROVIDER / "fixtures/context-projection-prepare.json").read_text()
        )
        self.assertEqual(fixture["boundary"], "post_tool")
        self.assertEqual(fixture["artifact"]["origin"], "api_response")
        self.assertEqual(
            hashlib.sha256(fixture["artifact"]["content"].encode()).hexdigest(),
            fixture["artifact"]["digest"],
        )
        self.assertNotIn("agent_id", fixture)
        self.assertNotIn("session_id", fixture)
        content = json.loads(fixture["artifact"]["content"])
        self.assertGreaterEqual(len(content["builds"]), 5)
        self.assertTrue(all(build["id"].startswith("build-") for build in content["builds"]))


if __name__ == "__main__":
    unittest.main()
