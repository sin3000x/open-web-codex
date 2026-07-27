from __future__ import annotations

import json
from pathlib import Path


def test_plugin_manifest_and_mcp_server_are_wired() -> None:
    root = Path(__file__).resolve().parents[1]
    manifest = json.loads((root / ".codex-plugin" / "plugin.json").read_text())
    mcp_config = json.loads((root / ".mcp.json").read_text())

    assert manifest["name"] == "hello-agent"
    assert manifest["skills"] == "./skills/"
    assert manifest["mcpServers"] == "./.mcp.json"

    assert set(mcp_config["mcpServers"]) == {"hello"}
    server = mcp_config["mcpServers"]["hello"]
    assert server["command"] == "./bin/hello-agent-launcher"
    assert server["args"] == []
