"""AW Provider entrypoint for agent-sec-core security capabilities.

The package exposes one stdin-JSON to stdout-JSON command so the AW Provider
Host can invoke agent-sec-core through the ``exec-json/v1`` driver. See
``handlers`` for the side-effect constraints this path must satisfy.
"""

from agent_sec_cli.aw_provider.runner import run_provider

__all__ = ["run_provider"]
