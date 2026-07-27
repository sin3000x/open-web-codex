"""Run initialize, tools/list, and tools/call against the real stdio server."""

from __future__ import annotations

import asyncio
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "bin" / "hello-agent-launcher"


async def call_tool(
    *,
    expected_tool: str,
    tool_args: dict[str, object],
) -> dict[str, object]:
    parameters = StdioServerParameters(
        command=str(LAUNCHER),
        args=[],
        cwd=str(ROOT),
    )
    async with stdio_client(parameters) as streams:
        async with ClientSession(*streams) as session:
            await session.initialize()
            tools = await session.list_tools()
            assert [tool.name for tool in tools.tools] == [expected_tool]
            result = await session.call_tool(expected_tool, tool_args)
            assert result.isError is not True
            assert result.structuredContent is not None
            return result.structuredContent


async def smoke() -> None:
    greeting = await call_tool(
        expected_tool="say_hello",
        tool_args={"name": "小林"},
    )
    assert greeting == {
        "name": "小林",
        "message": "你好，小林！",
    }


if __name__ == "__main__":
    asyncio.run(smoke())
    print("Hello Agent stdio smoke passed")
