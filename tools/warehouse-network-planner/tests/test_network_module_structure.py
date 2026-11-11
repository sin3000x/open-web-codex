from __future__ import annotations

import asyncio
from pathlib import Path

from supply_chain_planner.network import server, tool_runtime

NETWORK_ROOT = Path(__file__).parents[1] / "src" / "network"
EXPECTED_TOOLS = [
    "evaluate_baseline",
    "evaluate_scenario",
    "find_sites_for_service_target",
    "create_navigation_matrix_request",
    "prepare_network_distribution_map",
    "prepare_network_comparison_map",
    "prepare_network_coverage_map",
    "publish_network_planning_report",
]


def test_server_is_registration_only_and_owner_modules_have_no_second_mcp() -> None:
    server_text = (NETWORK_ROOT / "server.py").read_text(encoding="utf-8")
    assert len(server_text.splitlines()) < 120
    assert "SupplyChainResources" not in server_text
    assert "def plan_cost_matrix" not in server_text
    assert not hasattr(server, "plan_cost_matrix")
    runtime_text = (NETWORK_ROOT / "tool_runtime.py").read_text(encoding="utf-8")
    assert len(runtime_text.splitlines()) < 250
    assert "ruff: noqa: F401" not in runtime_text
    for forbidden_import in (
        "supply_chain_planner.delivery",
        "supply_chain_planner.network.matrix",
        "supply_chain_planner.network.solver",
        "supply_chain_planner.network.matrix_models",
    ):
        assert f"from {forbidden_import}" not in runtime_text
    for module_name in (
        "analysis_tools.py",
        "route_tools.py",
        "cost_tools.py",
        "facility_tools.py",
        "question_tools.py",
        "delivery_tools.py",
        "tool_runtime.py",
    ):
        module_text = (NETWORK_ROOT / module_name).read_text(encoding="utf-8")
        assert "FastMCP(" not in module_text


def test_registered_tools_and_resource_template_are_exact() -> None:
    async def collect() -> tuple[list[str], list[str]]:
        tools = await server.mcp.list_tools()
        templates = await server.mcp.list_resource_templates()
        return [tool.name for tool in tools], [template.uriTemplate for template in templates]

    tool_names, resource_templates = asyncio.run(collect())
    assert tool_names == EXPECTED_TOOLS
    assert resource_templates == ["supply-chain://resources/{resource_id}"]


def test_runtime_scope_initializes_one_resource_owner(monkeypatch, tmp_path: Path) -> None:
    calls = []

    class FakeResources:
        def __init__(self, workspace_root: Path, profile_state_root: Path) -> None:
            calls.append((workspace_root, profile_state_root))
            self.network = object()

    monkeypatch.setattr(tool_runtime, "SupplyChainResources", FakeResources)
    monkeypatch.setattr(tool_runtime, "_supply_chain_resources", None)
    tool_runtime.configure_runtime(tmp_path, tmp_path / "profile")
    assert tool_runtime._runtime() is not None
    assert tool_runtime._runtime() is not None
    assert len(calls) == 1
