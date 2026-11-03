"""Bounded stdio checks for the current Workspace-wide intake contract.

This smoke proves native-cwd discovery, inline inspection identity, and one
typed prepared Workspace handoff.
It inventories Network tools but never invokes them in the Data-only sample.
"""

from __future__ import annotations

import asyncio
import os
import sys
import tempfile
from pathlib import Path

import pytest
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

ROOT = Path(__file__).resolve().parents[1]
SANDBOX_META = "codex/sandbox-state-meta"


def server_environment(state_root: Path) -> dict[str, str]:
    environment = dict(os.environ)
    repository_data_dir = state_root / "runtime"
    environment.update(
        {
            "CODEX_HOME": str(state_root / "codex-home"),
            "OPEN_WEB_CODEX_DATA_DIR": str(repository_data_dir),
            # Force analysis MCP calls to fail closed in this isolated smoke.
            "OPEN_WEB_CODEX_ANALYSIS_GATE_URL": "",
            "OPEN_WEB_CODEX_ANALYSIS_GATE_KEY": "",
        }
    )
    return environment


def workspace_meta(root: Path) -> dict[str, object]:
    return {SANDBOX_META: {"sandboxCwd": root.as_uri()}}


async def smoke() -> None:
    with tempfile.TemporaryDirectory(prefix="supply-chain-mcp-smoke-") as directory:
        state_root = Path(directory)
        workspace = state_root / "workspace"
        workspace.mkdir()
        environment = server_environment(state_root)
        (workspace / "network.csv").write_text(
            "city_id,city_name,demand_quantity,latitude,longitude\ncity-1,Jakarta,10,-6.2,106.8\n",
            encoding="utf-8",
        )
        (workspace / "warehouse.csv").write_text(
            "warehouse_id,warehouse_name,warehouse_type,city_id,city_name,latitude,longitude,is_existing\n"
            "wh-1,Jakarta Center,center,city-1,Jakarta,-6.2,106.8,true\n",
            encoding="utf-8",
        )
        data_parameters = StdioServerParameters(
            command=sys.executable,
            args=["-m", "supply_chain_planner.data.server"],
            cwd=str(workspace),
            env=environment,
        )
        async with stdio_client(data_parameters) as streams:
            async with ClientSession(*streams) as session:
                await asyncio.wait_for(session.initialize(), timeout=10)
                tools = await asyncio.wait_for(session.list_tools(), timeout=10)
                names = {tool.name for tool in tools.tools}
                expected_tools = {
                    "discover_workspace_sources",
                    "inspect_workspace_sources",
                    "prepare_network_input",
                    "prepare_network_geography",
                    "query_prepared_network_input",
                }
                assert names == expected_tools
                assert (await asyncio.wait_for(session.list_resources(), timeout=10)).resources == []
                assert (
                    await asyncio.wait_for(session.list_resource_templates(), timeout=10)
                ).resourceTemplates == []
                meta = workspace_meta(workspace)
                discovered = await asyncio.wait_for(
                    session.call_tool("discover_workspace_sources", {}, meta=meta), timeout=10
                )
                assert discovered.isError is not True
                profiled = await asyncio.wait_for(
                    session.call_tool(
                        "inspect_workspace_sources",
                        {
                            "relative_paths": ["network.csv", "warehouse.csv"],
                            "required_roles": ["demand", "existing_warehouse"],
                            "country_code": "ID",
                        },
                        meta=meta,
                    ),
                    timeout=10,
                )
                assert profiled.isError is not True
                inspection = profiled.structuredContent
                assert "resource_ref" not in inspection
                assert inspection["outcome"] == "inspected"
                assert inspection["source_profile"]["sources"][0]["relative_path"] == "network.csv"
                assert inspection["source_profile"]["sources"][0]["units"][0][
                    "mapping_suggestions"
                ]
                assert inspection["inspection_identity"]["schemaVersion"] == "workspace_source_inspection.v2"
                prepared = await asyncio.wait_for(
                    session.call_tool(
                        "prepare_network_input",
                        {
                            "inspection_identity": inspection["inspection_identity"],
                            "inspected_relative_paths": inspection["inspected_relative_paths"],
                            "source_selections": [
                                {
                                    "relative_path": "network.csv",
                                    "unit_ref": "table",
                                    "role": "demand",
                                },
                                {
                                    "relative_path": "warehouse.csv",
                                    "unit_ref": "table",
                                    "role": "existing_warehouse",
                                },
                            ],
                            "country_code": "ID",
                            "output_relative_path": "outputs/warehouse-network/prepared/smoke.json",
                        },
                        meta=meta,
                    ),
                    timeout=10,
                )
                assert prepared.isError is not True
                assert prepared.structuredContent["prepared_input_relative_path"].startswith(
                    "outputs/warehouse-network/prepared/"
                )
                assert prepared.structuredContent["input_identity"]["content_sha256"]
                (workspace / "warehouses-missing-type.csv").write_text(
                    "warehouse_id,warehouse_name,city_id,city_name,is_existing\n"
                    "WH-1,Jakarta Center,city-1,Jakarta,true\n",
                    encoding="utf-8",
                )
                blocked_inspection = await asyncio.wait_for(
                    session.call_tool(
                        "inspect_workspace_sources",
                        {
                            "relative_paths": ["warehouses-missing-type.csv"],
                            "required_roles": ["existing_warehouse"],
                            "country_code": "ID",
                        },
                        meta=meta,
                    ),
                    timeout=10,
                )
                assert blocked_inspection.isError is not True
                blocked_profile = blocked_inspection.structuredContent
                assert blocked_profile["schemaVersion"] == "workspace_source_profile.v2"
                assert blocked_profile["outcome"] == "inspected"
                assert blocked_profile["next_action"] == "confirm_sources"
                assert blocked_profile["retryable"] is False
                assessment = blocked_profile["source_profile"]["sources"][0]["units"][0][
                    "role_assessments"
                ][0]
                assert assessment["state"] == "partial"
                assert assessment["missing_required_fields"] == ["warehouse_type"]
                blocked = await asyncio.wait_for(
                    session.call_tool(
                        "prepare_network_input",
                        {
                            "inspection_identity": blocked_profile["inspection_identity"],
                            "inspected_relative_paths": blocked_profile[
                                "inspected_relative_paths"
                            ],
                            "source_selections": [
                                {
                                    "relative_path": "warehouses-missing-type.csv",
                                    "unit_ref": "table",
                                    "role": "existing_warehouse",
                                }
                            ],
                            "country_code": "ID",
                            "output_relative_path": (
                                "outputs/warehouse-network/prepared/blocked.json"
                            ),
                        },
                        meta=meta,
                    ),
                    timeout=10,
                )
                assert blocked.isError is not True
                assert blocked.structuredContent["outcome"] == "needs_input"
                assert blocked.structuredContent["next_action"] == "request_user_input"
                assert blocked.structuredContent["retryable"] is False
                assert "prepared_input_relative_path" not in blocked.structuredContent
                assert not (workspace / "outputs/warehouse-network/prepared/blocked.json").exists()
        planning_parameters = StdioServerParameters(
            command=sys.executable,
            args=["-m", "supply_chain_planner.network.server"],
            cwd=str(workspace),
            env=environment,
        )
        async with stdio_client(planning_parameters) as streams:
            async with ClientSession(*streams) as session:
                await asyncio.wait_for(session.initialize(), timeout=10)
                names = {tool.name for tool in (await session.list_tools()).tools}
                expected_tools = {
                    "create_navigation_matrix_request",
                    "prepare_route_matrix",
                    "import_navigation_matrix",
                    "plan_cost_matrix",
                    "prepare_network_comparison_map",
                    "prepare_network_coverage_map",
                    "prepare_network_distribution_map",
                    "evaluate_network_baseline",
                    "assess_facility_change",
                    "solve_p_median",
                    "compare_network_scenarios",
                    "publish_network_planning_report",
                }
                assert expected_tools <= names, sorted(expected_tools - names)


def test_stdio_smoke() -> None:
    if os.environ.get("RUN_REAL_STDIO_SMOKE") != "1":
        pytest.skip("set RUN_REAL_STDIO_SMOKE=1 to launch both current MCP stdio servers")
    asyncio.run(asyncio.wait_for(smoke(), timeout=60))


if __name__ == "__main__":
    asyncio.run(asyncio.wait_for(smoke(), timeout=60))
