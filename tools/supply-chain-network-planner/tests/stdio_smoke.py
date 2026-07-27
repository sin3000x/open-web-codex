"""Exercise both supply-chain MCP servers over their real stdio boundary."""

from __future__ import annotations

import asyncio
import os
import tempfile
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "bin" / "supply-chain-planner-launcher"


def server_environment(state_root: Path) -> dict[str, str]:
    """Build an isolated environment while preserving the caller's normal process setup."""

    environment = dict(os.environ)
    environment.update(
        {
            "CODEX_HOME": str(state_root / "codex-home"),
            "OPEN_WEB_CODEX_LOG_DIR": str(state_root / "logs"),
            "SUPPLY_CHAIN_DATA_RESOURCE_DIR": str(state_root / "data-resources"),
            "SUPPLY_CHAIN_RESOURCE_DIR": str(state_root / "planning-resources"),
        }
    )
    return environment


async def smoke_data_server(environment: dict[str, str]) -> None:
    """Inspect, build, and validate the fixture through the Data MCP."""

    parameters = StdioServerParameters(
        command=str(LAUNCHER),
        args=["--data-server", "--workspace-root", "."],
        cwd=str(ROOT),
        env=environment,
    )
    async with stdio_client(parameters) as streams:
        async with ClientSession(*streams) as session:
            await session.initialize()
            tools = await session.list_tools()
            assert {tool.name for tool in tools.tools} == {
                "inspect_planning_source",
                "build_planning_dataset",
                "validate_planning_dataset",
            }

            inspection = await session.call_tool(
                "inspect_planning_source",
                {"source_id": "warehouse-network-fixture"},
            )
            assert inspection.isError is not True
            assert inspection.structuredContent is not None
            assert inspection.structuredContent["source_summary"]["demand_units"] == 100

            build = await session.call_tool(
                "build_planning_dataset",
                {"source_id": "warehouse-network-fixture"},
            )
            assert build.isError is not True
            assert build.structuredContent is not None
            resource_ref = build.structuredContent["data_ref"]

            validation = await session.call_tool(
                "validate_planning_dataset",
                {"resource_ref": resource_ref},
            )
            assert validation.isError is not True
            assert validation.structuredContent is not None
            assert validation.structuredContent["valid"] is True


async def smoke_planning_server(environment: dict[str, str]) -> None:
    """Create and validate one snapshot through the Network Planning MCP."""

    parameters = StdioServerParameters(
        command=str(LAUNCHER),
        args=["--workspace-root", "."],
        cwd=str(ROOT),
        env=environment,
    )
    async with stdio_client(parameters) as streams:
        async with ClientSession(*streams) as session:
            await session.initialize()
            tools = await session.list_tools()
            assert {tool.name for tool in tools.tools} == {
                "prepare_network_snapshot",
                "register_route_matrix",
                "evaluate_current_coverage",
                "evaluate_network_scenario",
                "compare_network_scenarios",
                "solve_facility_location",
                "validate_network_resource",
            }

            snapshot = await session.call_tool(
                "prepare_network_snapshot",
                {"source_path": "examples/network-input.json"},
            )
            assert snapshot.isError is not True
            assert snapshot.structuredContent is not None
            resource_ref = snapshot.structuredContent["data_ref"]

            validation = await session.call_tool(
                "validate_network_resource",
                {"resource_ref": resource_ref},
            )
            assert validation.isError is not True
            assert validation.structuredContent is not None
            assert validation.structuredContent["valid"] is True


async def smoke() -> None:
    """Run both isolated server checks using one temporary state root."""

    with tempfile.TemporaryDirectory(prefix="supply-chain-mcp-smoke-") as directory:
        environment = server_environment(Path(directory))
        await smoke_data_server(environment)
        await smoke_planning_server(environment)


if __name__ == "__main__":
    asyncio.run(smoke())
    print("Supply-chain MCP stdio smoke passed")
