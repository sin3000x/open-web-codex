"""FastMCP entry point for supply-chain network planning."""

from __future__ import annotations

import argparse
import asyncio
import logging
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

from mcp.server.fastmcp import FastMCP
from mcp.server.stdio import stdio_server

from . import analysis_tools, cost_tools, delivery_tools, facility_tools, route_tools
from .tool_runtime import (
    SANDBOX_STATE_META_CAPABILITY,
    configure_runtime,
    register_resources,
)

mcp = FastMCP(
    "Supply Chain Network Planner",
    instructions=(
        "仓网工具以平台发布的 Resource 引用作为输入和输出，不把原始数据、矩阵行或"
        "内部 Work State 标识复制到 Agent 消息。球面距离必须明确提供绕路系数和"
        "平均速度；导航必须先报告路线数量和费用风险并获得用户许可。没有当前覆盖"
        "关系时，结果必须标记为现有仓优化基线，不能称为当前实际方案。"
    ),
    json_response=True,
)

# Keep this sequence identical to the historical public Tool registration order.
register_resources(mcp)
analysis_tools.register_tools(mcp, phase="compare")
delivery_tools.register_tools(mcp, phase="distribution")
route_tools.register_tools(mcp)
cost_tools.register_tools(mcp)
analysis_tools.register_tools(mcp, phase="baseline")
facility_tools.register_tools(mcp)
delivery_tools.register_tools(mcp, phase="final")


def main() -> None:
    parser = argparse.ArgumentParser(description="Supply-chain network planning MCP server")
    parser.add_argument(
        "--transport",
        choices=("stdio", "streamable-http"),
        default="stdio",
    )
    args = parser.parse_args()

    workspace_root = Path.cwd().resolve(strict=True)
    profile_state_root = Path(os.environ.get("CODEX_HOME", workspace_root / ".codex")).resolve()
    configure_runtime(workspace_root, profile_state_root)
    threading.Thread(target=_warm_solver_libraries, name="solver-warmup", daemon=True).start()
    if args.transport == "stdio":
        asyncio.run(run_stdio())
    else:
        mcp.run(transport=args.transport)


def _warm_solver_libraries() -> None:
    """Pay the native solver's first-load cost outside this server process.

    The first load of freshly materialized OR-Tools libraries can take minutes on
    some hosts while holding the loading process's GIL. A short-lived child
    process primes the host's library cache instead, so this server keeps
    answering Tool calls and its own later solver load is fast.
    """
    started_at = time.monotonic()
    try:
        completed = subprocess.run(
            [sys.executable, "-c", "from ortools.sat.python import cp_model"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError as error:
        logging.getLogger(__name__).warning("solver warm-up could not start: %s", error)
        return
    logging.getLogger(__name__).info(
        "solver warm-up finished: exit_code=%s seconds=%.1f",
        completed.returncode,
        time.monotonic() - started_at,
    )


async def run_stdio() -> None:
    initialization_options = mcp._mcp_server.create_initialization_options(
        experimental_capabilities={SANDBOX_STATE_META_CAPABILITY: {}},
    )
    async with stdio_server() as streams:
        await mcp._mcp_server.run(streams[0], streams[1], initialization_options)


if __name__ == "__main__":
    main()
