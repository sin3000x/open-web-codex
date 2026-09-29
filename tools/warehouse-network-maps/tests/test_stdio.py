from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client


class WorkspaceMetadataTests(unittest.IsolatedAsyncioTestCase):
    async def test_initialize_and_publish_from_request_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "state"
            workspace = root / "workspace"
            state.mkdir()
            workspace.mkdir()
            payload = {
                "type": "FeatureCollection",
                "features": [{
                    "type": "Feature",
                    "geometry": {"type": "Point", "coordinates": [120, 30]},
                    "properties": {"kind": "demand", "service_status": "attained"},
                }],
            }
            (workspace / "coverage.geojson").write_text(json.dumps(payload))
            parameters = StdioServerParameters(
                command=sys.executable,
                args=["-m", "maps_mcp.server", "--workspace-root", str(state)],
                cwd=str(state),
                env=dict(os.environ),
            )
            async with stdio_client(parameters) as streams:
                async with ClientSession(*streams) as session:
                    initialized = await asyncio.wait_for(session.initialize(), timeout=10)
                    capability = "codex/sandbox-state-meta"
                    self.assertIn(capability, initialized.capabilities.experimental or {})
                    arguments = {"workspace_relative_path": "coverage.geojson"}
                    missing = await session.call_tool("publish_workspace_geojson", arguments)
                    self.assertTrue(missing.isError)
                    published = await session.call_tool(
                        "publish_workspace_geojson", arguments,
                        meta={capability: {"sandboxCwd": workspace.as_uri()}},
                    )
                    self.assertFalse(published.isError)
                    self.assertIsNotNone(published.structuredContent)
                    data_ref = published.structuredContent["data_ref"]
                    resource = await session.read_resource(data_ref["uri"])
                    self.assertEqual(json.loads(resource.contents[0].text), payload)


if __name__ == "__main__":
    unittest.main()
