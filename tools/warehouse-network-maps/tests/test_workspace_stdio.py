"""Exercise the real managed entry point and its Workspace capability handshake."""
from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

CAPABILITY = "codex/sandbox-state-meta"


class WorkspaceStdioTests(unittest.IsolatedAsyncioTestCase):
    async def test_handshake_publish_scope_switch_and_denials(self) -> None:
        async with asyncio.timeout(30):
            await self.exercise_server()

    async def exercise_server(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "provider-state"
            state.mkdir()
            workspaces = [root / "first", root / "second"]
            for index, workspace in enumerate(workspaces):
                workspace.mkdir()
                (workspace / "points.geojson").write_text(json.dumps({
                    "type": "FeatureCollection",
                    "features": [{
                        "type": "Feature", "properties": {"workspace": index},
                        "geometry": {"type": "Point", "coordinates": [index, 0]},
                    }],
                }))
            # A valid provider-state file must never substitute for missing scope.
            (state / "points.geojson").write_text((workspaces[0] / "points.geojson").read_text())
            (workspaces[0] / "escape.geojson").symlink_to(workspaces[1] / "points.geojson")
            parameters = StdioServerParameters(
                command=sys.executable,
                args=["-m", "maps_mcp.server", "--workspace-root", str(state)],
                cwd=str(state), env=dict(os.environ),
            )
            # Fresh process and reconnect must both negotiate the capability.
            for _ in range(2):
                async with stdio_client(parameters) as streams:
                    async with ClientSession(*streams) as session:
                        initialized = await session.initialize()
                        self.assertIn(CAPABILITY, initialized.capabilities.experimental or {})
                        tools = await session.list_tools()
                        self.assertIn("publish_workspace_geojson", {t.name for t in tools.tools})

                        async def publish(path="points.geojson", uri=None):
                            meta = None if uri is None else {CAPABILITY: {"sandboxCwd": uri}}
                            return await session.call_tool(
                                "publish_workspace_geojson",
                                {"workspace_relative_path": path}, meta=meta,
                            )

                        for index, workspace in enumerate(workspaces):
                            result = await publish(uri=workspace.as_uri())
                            self.assertFalse(result.isError, result.content)
                            links = [c for c in result.content if c.type == "resource_link"]
                            self.assertEqual(len(links), 1)
                            resource = await session.read_resource(links[0].uri)
                            data = json.loads(resource.contents[0].text)
                            self.assertEqual(data["features"][0]["properties"]["workspace"], index)

                        for path, uri, code in [
                            ("points.geojson", None, "workspace_scope_unavailable"),
                            ("points.geojson", "https://example.invalid/workspace", "workspace_scope_invalid"),
                            ("points.geojson", (root / "missing").as_uri(), "workspace_scope_invalid"),
                            ("../points.geojson", workspaces[0].as_uri(), "workspace_path_invalid"),
                            ("escape.geojson", workspaces[0].as_uri(), "workspace_source_symlink_rejected"),
                        ]:
                            result = await publish(path, uri)
                            self.assertTrue(result.isError)
                            self.assertIn(code, " ".join(c.text for c in result.content if c.type == "text"))


if __name__ == "__main__":
    unittest.main()
