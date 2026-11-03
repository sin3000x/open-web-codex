from __future__ import annotations

import asyncio
import importlib.util
import subprocess
import sys
import tomllib
from pathlib import Path

from supply_chain_planner import __version__
from supply_chain_planner.data.server import mcp as data_mcp
from supply_chain_planner.network import server as network_server
from supply_chain_planner.network.server import mcp as network_mcp
from supply_chain_planner.shared.resource_identity import (
    DATA_MCP_SERVER_NAME,
    NETWORK_MCP_SERVER_NAME,
)

REMOVED_CASE_ENTRYPOINTS = (
    "create_network_case",
    "get_network_case_status",
    "archive_network_case",
    "refresh_case_sources",
    "inspect_case_sources",
    "propose_case_mapping",
    "apply_case_mapping",
    "normalize_case_input",
    "define_network_requirements",
)
REMOVED_CASE_LEGACY_ENTRYPOINTS = (
    "_legacy_plan_route_matrix",
    "_legacy_build_haversine_route_matrix",
    "_legacy_validate_route_matrix",
    "_legacy_register_navigation_route_matrix",
    "_legacy_plan_cost_matrix",
    "_legacy_evaluate_network_baseline",
    "_legacy_evaluate_facility_scenario",
    "_legacy_solve_p_median",
    "_legacy_solve_service_constrained_location",
    "_legacy_render_network_comparison_map",
    "_legacy_publish_network_planning_report",
)
REMOVED_MODULES = (
    "analysis_service",
    "case_models",
    "case_repository",
    "case_tools",
    "case_types",
    "core",
    "data_core",
    "decision_core",
    "demo_server",
    "indonesia_analysis",
    "indonesia_models",
    "indonesia_report",
    "indonesia_server",
    "mapping_service",
    "matrix_service",
    "network_data",
    "readiness",
    "requirements",
    "scenario_service",
    "workspace_dataset",
)
REMOVED_CONTRACT_FILES = (
    "supply_chain_planner/case_schema.sql",
    "contracts/warehouse-network-planning-1.0.0.json",
    "contracts/indonesia-warehouse-network-2.0.0.json",
)
DATA_TOOLS = {
    "discover_workspace_sources",
    "inspect_workspace_sources",
    "prepare_network_input",
    "prepare_network_geography",
    "query_prepared_network_input",
    "resolve_prepared_network_input",
}
NETWORK_TOOLS = {
    "assess_facility_change",
    "compare_network_scenarios",
    "create_navigation_matrix_request",
    "evaluate_network_baseline",
    "import_navigation_matrix",
    "plan_cost_matrix",
    "prepare_route_matrix",
    "prepare_network_comparison_map",
    "prepare_network_coverage_map",
    "prepare_network_distribution_map",
    "publish_network_planning_report",
    "solve_p_median",
}


def test_tool_author_source_has_no_runtime_transport_or_lifecycle_scripts() -> None:
    root = Path(__file__).resolve().parents[1]
    project = tomllib.loads((root / "pyproject.toml").read_text())

    assert project["project"]["version"] == __version__ == "0.4.0"
    assert not (root / ".codex-plugin").exists()
    assert not (root / ".mcp.json").exists()
    assert not (root / "bin").exists()


def test_generic_runtime_declaration_owns_dependencies_and_server_entries() -> None:
    root = Path(__file__).resolve().parents[1]
    runtime = tomllib.loads((root / "runtime.toml").read_text())

    assert runtime["schema_version"] == 1
    assert runtime["dependencies"] == [
        {
            "id": "python",
                "kind": "python-project",
                "manifest": "pyproject.toml",
                "lock": "requirements.lock",
                "platform_packages": ["open-web-codex-provider-sdk"],
            }
    ]
    servers = {server["id"]: server for server in runtime["servers"]}
    assert set(servers) == {"supply_chain_data", "supply_chain"}
    assert DATA_MCP_SERVER_NAME == "supply_chain_data"
    assert NETWORK_MCP_SERVER_NAME == "supply_chain"
    assert servers["supply_chain_data"]["entry"] == {
        "kind": "python-module",
        "dependency": "python",
        "module": "supply_chain_planner.data.server",
    }
    assert servers["supply_chain"]["entry"] == {
        "kind": "python-module",
        "dependency": "python",
        "module": "supply_chain_planner.network.server",
    }
    for server in servers.values():
        assert server["args"] == []
        bindings = {(entry["name"], entry["source"]) for entry in server["env"]}
        assert ("CODEX_HOME", "profile_home") in bindings
        assert ("HTTP_PROXY", "host") in bindings
        assert all("dependency" not in entry for entry in server["env"])

    lock = (root / "requirements.lock").read_text()
    assert "--hash=sha256:" in lock
    assert "setuptools==" in lock
    assert " -e " not in lock and "file:" not in lock


def test_network_server_exposes_only_network_tools() -> None:
    names = {tool.name for tool in asyncio.run(network_mcp.list_tools())}
    assert names == NETWORK_TOOLS
    for entrypoint in REMOVED_CASE_ENTRYPOINTS:
        assert not hasattr(network_server, entrypoint)
    for entrypoint in REMOVED_CASE_LEGACY_ENTRYPOINTS:
        assert not hasattr(network_server, entrypoint)
    for module_name in REMOVED_MODULES:
        assert importlib.util.find_spec(f"supply_chain_planner.{module_name}") is None


def test_network_server_startup_does_not_eagerly_load_native_solver() -> None:
    completed = subprocess.run(
        [
            sys.executable,
            "-c",
            (
                "import sys; "
                "import supply_chain_planner.network.server; "
                "assert not any(name == 'ortools' or name.startswith('ortools.') "
                "for name in sys.modules)"
            ),
        ],
        check=False,
        capture_output=True,
        text=True,
    )

    assert completed.returncode == 0, completed.stderr


def test_data_server_exposes_only_data_tools() -> None:
    names = {tool.name for tool in asyncio.run(data_mcp.list_tools())}
    assert names == DATA_TOOLS


def test_legacy_skill_surface_is_removed() -> None:
    root = Path(__file__).resolve().parents[1]
    assert not (root / "skills").exists()


def test_legacy_case_contract_surface_is_removed() -> None:
    root = Path(__file__).resolve().parents[1]
    for relative_path in REMOVED_CONTRACT_FILES:
        assert not (root / relative_path).exists()
