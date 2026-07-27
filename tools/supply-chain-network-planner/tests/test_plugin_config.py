from __future__ import annotations

import json
from pathlib import Path


def test_plugin_manifest_and_mcp_config_are_wired() -> None:
    root = Path(__file__).resolve().parents[1]
    manifest = json.loads((root / ".codex-plugin" / "plugin.json").read_text())
    mcp_config = json.loads((root / ".mcp.json").read_text())

    assert manifest["name"] == "supply-chain-network-planner"
    assert manifest["skills"] == "./skills/"
    assert manifest["mcpServers"] == "./.mcp.json"
    data_server = mcp_config["mcpServers"]["supply_chain_data"]
    assert data_server["command"] == "./bin/supply-chain-planner-launcher"
    assert data_server["args"][0] == "--data-server"
    assert data_server["cwd"] == "."
    server = mcp_config["mcpServers"]["supply_chain_planner"]
    assert server["command"] == "./bin/supply-chain-planner-launcher"
    assert server["cwd"] == "."
