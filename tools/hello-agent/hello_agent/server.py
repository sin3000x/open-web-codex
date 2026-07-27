"""MCP boundary for the Hello Agent tutorial Tool."""

from __future__ import annotations

from mcp.server.fastmcp import FastMCP

from .core import Greeting, build_greeting

mcp = FastMCP("Hello")


@mcp.tool()
def say_hello(name: str) -> Greeting:
    """Return one deterministic structured greeting for a named person."""

    return build_greeting(name)


def main() -> None:
    """Run the MCP Server over standard input and output."""

    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
