from __future__ import annotations

import unittest

from mcp.types import CallToolResult
from mcp.server.fastmcp.exceptions import ToolError

import maps_mcp.server as server
from maps_mcp.map_card import GeoJsonSource


def geojson() -> dict[str, object]:
    return {"type": "FeatureCollection", "features": []}


class MapCardTests(unittest.IsolatedAsyncioTestCase):
    async def test_preserves_standard_mapbox_layers(self) -> None:
        layer = {
            "id": "routes",
            "type": "line",
            "source": "routes",
            "minzoom": 3,
            "layout": {"line-cap": "round"},
            "paint": {
                "line-color": [
                    "interpolate",
                    ["linear"],
                    ["zoom"],
                    4,
                    "#2563eb",
                    12,
                    "#ef4444",
                ],
                "line-width": 4,
            },
        }
        result = await server.create_map_card(
            title="路线",
            sources={
                "routes": GeoJsonSource(
                    type="geojson",
                    data=geojson(),
                    lineMetrics=True,
                )
            },
            layers=[layer],
            center=(114.0579, 22.5431),
            zoom=8,
            extensions={
                "hover": {
                    "layers": [
                        {
                            "layer": "routes",
                            "title_property": "name",
                            "fields": ["distance"],
                        }
                    ]
                },
                "legend": {
                    "items": [
                        {"label": "路线", "color": "#2563eb", "type": "line"}
                    ]
                },
            },
        )

        self.assertIsInstance(result, CallToolResult)
        assert result.structuredContent is not None
        renderer = result.structuredContent["artifact"]["renderer"]
        self.assertEqual(renderer["kind"], "map.v3")
        payload = renderer["payload"]
        self.assertEqual(payload["layers"], [layer])
        self.assertTrue(payload["sources"]["routes"]["lineMetrics"])
        self.assertEqual(payload["center"], [114.0579, 22.5431])
        self.assertEqual(
            payload["extensions"]["hover"]["layers"][0]["fields"],
            ["distance"],
        )

    async def test_resource_data_ref_is_managed_without_copying_geojson(self) -> None:
        uri = "maps-data://geojson/map-data-1234"
        result = await server.mcp.call_tool(
            "create_map_card",
            {
                "title": "路线",
                "sources": {
                    "route": {
                        "type": "geojson",
                        "data_ref": {
                            "type": "mcp_resource",
                            "server": "map_utils",
                            "uri": uri,
                            "format": "geojson",
                        },
                    }
                },
                "layers": [
                    {
                        "id": "route",
                        "type": "line",
                        "source": "route",
                        "paint": {"line-color": "#2563eb"},
                    }
                ],
            },
        )
        assert result.structuredContent is not None
        source = result.structuredContent["artifact"]["renderer"]["payload"]["sources"][
            "route"
        ]
        self.assertEqual(source["data"]["uri"], uri)

    async def test_result_requires_standalone_assistant_embed_paragraph(self) -> None:
        result = await server.create_map_card(
            title="路线",
            sources={
                "routes": GeoJsonSource(
                    type="geojson",
                    data=geojson(),
                )
            },
            layers=[
                {
                    "id": "routes",
                    "type": "line",
                    "source": "routes",
                    "paint": {"line-color": "#2563eb"},
                }
            ],
        )

        assert result.structuredContent is not None
        embed_code = result.structuredContent["embed"]["code"]
        self.assertEqual(len(result.content), 1)
        message = result.content[0]
        self.assertEqual(message.type, "text")
        assert message.text is not None
        self.assertIn("not displayed until", message.text)
        self.assertIn("standalone paragraph", message.text)
        self.assertIn("may appear anywhere", message.text)
        self.assertIn(f"\n\n{embed_code}\n\n", message.text)

    async def test_official_validator_warns_unknown_and_rejects_invalid_known_syntax(
        self,
    ) -> None:
        warning = await server.mcp.call_tool(
            "create_map_card",
            {
                "title": "Warning",
                "sources": {
                    "data": {"type": "geojson", "data": geojson()},
                },
                "layers": [
                    {
                        "id": "points",
                        "type": "circle",
                        "source": "data",
                        "paint": {
                            "circle-color": "#ef4444",
                            "unknown-property": 1,
                        },
                    }
                ],
            },
        )
        assert warning.structuredContent is not None
        self.assertEqual(
            warning.structuredContent["warnings"][0]["code"],
            "mapbox_style_warning",
        )

        with self.assertRaisesRegex(ToolError, "number expected"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Invalid",
                    "sources": {
                        "data": {"type": "geojson", "data": geojson()},
                    },
                    "layers": [
                        {
                            "id": "points",
                            "type": "circle",
                            "source": "data",
                            "paint": {"circle-radius": "large"},
                        }
                    ],
                },
            )

    async def test_schema_exposes_raw_layers_sources_and_optional_extensions(self) -> None:
        tool = next(
            tool
            for tool in await server.mcp.list_tools()
            if tool.name == "create_map_card"
        )
        assert tool.inputSchema is not None
        self.assertEqual(tool.inputSchema["properties"]["layers"]["items"]["type"], "object")
        self.assertEqual(tool.inputSchema["properties"]["sources"]["type"], "object")
        self.assertIn("extensions", tool.inputSchema["properties"])
        self.assertIn("center", tool.inputSchema["properties"])
        self.assertNotIn("view", tool.inputSchema["properties"])
        self.assertNotIn("legend", tool.inputSchema["properties"])
        assert tool.outputSchema is not None
        self.assertEqual(
            tool.outputSchema["$defs"]["Renderer"]["properties"]["kind"]["const"],
            "map.v3",
        )


if __name__ == "__main__":
    unittest.main()
