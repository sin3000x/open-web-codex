from __future__ import annotations

import json
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path

import maps_mcp.server as server
from maps_mcp.data_refs import GeoJsonResourceStore, MapCardSpecStore
from maps_mcp.map_card import GeoJsonSource, MapCardPatch, MapCardSpec
from mcp.server.fastmcp.exceptions import ToolError
from mcp.types import CallToolResult
from open_web_codex_provider import GeoJsonResourceRef
from pydantic import ValidationError


def geojson() -> dict[str, object]:
    return {"type": "FeatureCollection", "features": []}


def data_ref(uri: str = "maps-data://geojson/map-data-1234") -> dict[str, object]:
    return {
        "type": "mcp_resource",
        "server": "map_utils",
        "uri": uri,
        "resource_schema": "geojson.v1",
        "format": "geojson",
        "profile": {
            "schema_version": "geojson-profile.v3",
            "feature_count": 1,
            "discriminator_property": None,
            "feature_types": [
                {
                    "value": "geometry:mixed",
                    "feature_count": 1,
                    "geometry_types": ["LineString", "Point"],
                    "properties": {"distance": "number", "name": "string"},
                }
            ],
        },
    }


def network_data_ref() -> dict[str, object]:
    return {
        "type": "mcp_resource",
        "server": "supply_chain",
        "uri": "supply-chain://resources/network_distribution_geojson.v1-digest",
        "resource_schema": "network_distribution_geojson.v1",
        "format": "geojson",
        "profile": {
            "schema_version": "geojson-profile.v3",
            "feature_count": 61,
            "discriminator_property": "kind",
            "feature_types": [
                {
                    "value": "demand",
                    "feature_count": 50,
                    "geometry_types": ["Point"],
                    "properties": {
                        "kind": "string",
                        "city_name": "string",
                        "duration_hours": "number?",
                    },
                },
                {
                    "value": "warehouse",
                    "feature_count": 11,
                    "geometry_types": ["Point"],
                    "properties": {
                        "kind": "string",
                        "warehouse_name": "string",
                        "warehouse_type": "string",
                    },
                },
            ],
        },
    }


def coverage_data_ref() -> dict[str, object]:
    return {
        "type": "mcp_resource",
        "server": "supply_chain",
        "uri": "supply-chain://resources/network_coverage_geojson.v2-digest",
        "resource_schema": "network_coverage_geojson.v2",
        "format": "geojson",
        "profile": {
            "schema_version": "geojson-profile.v3",
            "feature_count": 129,
            "discriminator_property": "kind",
            "feature_types": [
                {
                    "value": "demand",
                    "feature_count": 50,
                    "geometry_types": ["Point"],
                    "properties": {
                        "assigned_warehouse_id": "string",
                        "city_id": "string",
                        "city_name": "string",
                        "demand_quantity": "number",
                        "distance_km": "number",
                        "duration_hours": "number",
                        "kind": "string",
                        "service_status": "string",
                    },
                },
                {
                    "value": "last_mile_assignment",
                    "feature_count": 50,
                    "geometry_types": ["LineString"],
                    "properties": {
                        "demand_city_id": "string",
                        "distance_km": "number",
                        "duration_hours": "number",
                        "kind": "string",
                        "service_status": "string",
                        "warehouse_id": "string",
                    },
                },
                {
                    "value": "linehaul_connection",
                    "feature_count": 6,
                    "geometry_types": ["LineString"],
                    "properties": {
                        "assigned_demand": "number",
                        "crossdock_warehouse_id": "string",
                        "kind": "string",
                        "upstream_center_id": "string",
                    },
                },
                {
                    "value": "warehouse",
                    "feature_count": 23,
                    "geometry_types": ["Point"],
                    "properties": {
                        "city_name": "string",
                        "is_existing": "boolean",
                        "kind": "string",
                        "added_facility": "boolean",
                        "after_active": "boolean",
                        "before_active": "boolean",
                        "removed_facility": "boolean",
                        "warehouse_id": "string",
                        "warehouse_name": "string",
                        "warehouse_type": "string",
                    },
                    "boolean_property_counts": {
                        "is_existing": {"true_count": 11, "false_count": 12},
                        "added_facility": {"true_count": 2, "false_count": 21},
                        "after_active": {"true_count": 13, "false_count": 10},
                        "before_active": {"true_count": 11, "false_count": 12},
                        "removed_facility": {"true_count": 0, "false_count": 23},
                    },
                },
            ],
        },
    }


class MapCardTests(unittest.IsolatedAsyncioTestCase):
    async def test_network_map_card_owns_the_standard_warehouse_layer_recipe(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            original = server._map_card_spec_store
            server._map_card_spec_store = MapCardSpecStore(Path(directory))
            try:
                result = await server.create_network_map_card(
                    "Coverage",
                    GeoJsonResourceRef.model_validate(coverage_data_ref()),
                )
                self.assertIsNotNone(result.structuredContent)
                spec_ref = result.structuredContent["map_spec_ref"]
                resource_id = spec_ref["uri"].rsplit("/", 1)[-1]
                spec = json.loads(server._map_card_spec_store.read(resource_id))
                layer_ids = [layer["id"] for layer in spec["layers"]]
                self.assertIn("existing-center", layer_ids)
                self.assertIn("candidate-cross-docking", layer_ids)
                self.assertIn("added-facilities", layer_ids)
                self.assertIn("attained-last-mile-coverage", layer_ids)
                self.assertIn("missed-last-mile-coverage", layer_ids)
                self.assertIn("linehaul-coverage", layer_ids)
                self.assertIn("attained-demand-cities", layer_ids)
                self.assertIn("missed-demand-cities", layer_ids)
                self.assertIn("unassigned-demand-cities", layer_ids)
                hover_layers = {
                    item["layer"]: item
                    for item in spec["extensions"]["hover"]["layers"]
                }
                self.assertIn("attained-demand-cities", hover_layers)
                self.assertIn("missed-demand-cities", hover_layers)
                self.assertIn("attained-last-mile-coverage", hover_layers)
                self.assertIn(
                    "service_status",
                    hover_layers["missed-demand-cities"]["fields"],
                )
                self.assertIn("before_active", hover_layers["existing-center"]["fields"])
                self.assertIn(
                    "duration_hours",
                    hover_layers["attained-demand-cities"]["fields"],
                )
                legend_labels = {
                    item["label"] for item in spec["extensions"]["legend"]["items"]
                }
                self.assertTrue(
                    {"达标城市", "未达标城市", "无法判断城市"} <= legend_labels
                )
            finally:
                server._map_card_spec_store = original

    async def test_comparison_profile_exposes_before_after_hover_fields(self) -> None:
        comparison = deepcopy(coverage_data_ref())
        demand = comparison["profile"]["feature_types"][0]
        assert isinstance(demand, dict)
        properties = demand["properties"]
        assert isinstance(properties, dict)
        properties.pop("service_status")
        last_mile = comparison["profile"]["feature_types"][1]
        assert isinstance(last_mile, dict)
        last_mile["properties"].pop("service_status")
        properties.update(
            {
                "after_duration_hours": "number",
                "after_warehouse_id": "string",
                "before_duration_hours": "number",
                "before_warehouse_id": "string",
            }
        )
        with tempfile.TemporaryDirectory() as directory:
            original = server._map_card_spec_store
            server._map_card_spec_store = MapCardSpecStore(Path(directory))
            try:
                result = await server.create_network_map_card(
                    "Comparison",
                    GeoJsonResourceRef.model_validate(comparison),
                )
                spec_ref = result.structuredContent["map_spec_ref"]
                resource_id = spec_ref["uri"].rsplit("/", 1)[-1]
                spec = json.loads(server._map_card_spec_store.read(resource_id))
                hover_layers = {
                    item["layer"]: item for item in spec["extensions"]["hover"]["layers"]
                }
                self.assertIn(
                    "after_warehouse_id",
                    hover_layers["demand-cities"]["fields"],
                )
                self.assertIn(
                    "before_duration_hours",
                    hover_layers["demand-cities"]["fields"],
                )
            finally:
                server._map_card_spec_store = original

    def test_provider_resources_are_content_addressed_and_immutable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            geojson_store = server.GeoJsonResourceStore(root)
            spec_store = MapCardSpecStore(root)

            geojson = {"type": "FeatureCollection", "features": []}
            spec = {"schemaVersion": "map_card_spec.v1", "title": "Coverage"}
            first_geojson = geojson_store.publish(geojson)
            second_geojson = geojson_store.publish(geojson)
            first_spec = spec_store.publish(spec)
            second_spec = spec_store.publish(spec)

            self.assertEqual(first_geojson.uri, second_geojson.uri)
            self.assertEqual(first_spec.uri, second_spec.uri)
            self.assertEqual(
                GeoJsonResourceStore(root).read(first_geojson.resource_id),
                json.dumps(geojson, ensure_ascii=False, separators=(",", ":")),
            )
            self.assertEqual(
                json.loads(MapCardSpecStore(root).read(first_spec.resource_id)),
                spec,
            )
            self.assertEqual(len(list((root / ".codex" / "maps-data").glob("*.geojson"))), 1)
            self.assertEqual(len(list((root / ".codex" / "map-card-specs").glob("*.json"))), 1)

    async def test_revise_card_creates_one_child_spec_and_reuses_geojson_reference(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            original_store = server._map_card_spec_store
            store = MapCardSpecStore(Path(directory))
            server._map_card_spec_store = store
            try:
                created = await server.create_map_card(
                    title="Coverage",
                    sources={"network": GeoJsonSource(type="geojson", data_ref=network_data_ref())},
                    layers=[
                        {
                            "id": "cities",
                            "type": "circle",
                            "source": "network",
                            "paint": {"circle-color": "#2563eb"},
                        }
                    ],
                )
                assert created.structuredContent is not None
                parent_ref = created.structuredContent["map_spec_ref"]
                revised = await server.revise_map_card(
                    server.ResourceRef.model_validate(parent_ref),
                    MapCardPatch(title="Coverage — revised style"),
                )
                assert revised.structuredContent is not None
                child_ref = revised.structuredContent["map_spec_ref"]
                self.assertNotEqual(parent_ref["uri"], child_ref["uri"])
                parent_id = parent_ref["uri"].rsplit("/", 1)[1]
                child_id = child_ref["uri"].rsplit("/", 1)[1]
                parent = MapCardSpec.model_validate(json.loads(store.read(parent_id)))
                child = MapCardSpec.model_validate(json.loads(store.read(child_id)))
                self.assertEqual(parent.sources, child.sources)
                self.assertEqual(child.title, "Coverage — revised style")
                self.assertEqual(
                    child.parent_spec_ref,
                    server.ResourceRef.model_validate(parent_ref),
                )
                self.assertNotIn("data", json.loads(store.read(child_id))["sources"]["network"])
            finally:
                server._map_card_spec_store = original_store

    async def test_present_card_reuses_exact_spec_with_fresh_embed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            original_store = server._map_card_spec_store
            store = MapCardSpecStore(Path(directory))
            server._map_card_spec_store = store
            try:
                created = await server.create_network_map_card(
                    "12小时覆盖",
                    GeoJsonResourceRef.model_validate(coverage_data_ref()),
                )
                assert created.structuredContent is not None
                spec_ref = created.structuredContent["map_spec_ref"]
                replayed = await server.present_map_card(
                    server.ResourceRef.model_validate(spec_ref)
                )
                assert replayed.structuredContent is not None
                self.assertEqual(replayed.structuredContent["map_spec_ref"], spec_ref)
                self.assertEqual(
                    replayed.structuredContent["artifact"]["renderer"]["payload"],
                    created.structuredContent["artifact"]["renderer"]["payload"],
                )
                self.assertNotEqual(
                    replayed.structuredContent["embed"]["code"],
                    created.structuredContent["embed"]["code"],
                )
                self.assertEqual(len(list(store.root.glob("*.json"))), 1)
            finally:
                server._map_card_spec_store = original_store

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
                    data_ref=data_ref(),
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
                "legend": {"items": [{"label": "路线", "color": "#2563eb", "type": "line"}]},
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
                        "data_ref": data_ref(uri),
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
        source = result.structuredContent["artifact"]["renderer"]["payload"]["sources"]["route"]
        self.assertEqual(source["data"]["uri"], uri)
        self.assertNotIn("profile", source["data"])

    async def test_rejects_inline_geojson_and_requires_data_ref(self) -> None:
        with self.assertRaises(ValidationError):
            GeoJsonSource(type="geojson", data=geojson())

        with self.assertRaisesRegex(ToolError, "data_ref"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Inline data",
                    "sources": {"network": {"type": "geojson", "data": geojson()}},
                    "layers": [
                        {
                            "id": "network",
                            "type": "circle",
                            "source": "network",
                            "paint": {"circle-color": "#2563eb"},
                        }
                    ],
                },
            )

    async def test_accepts_reviewed_external_local_geojson_reference(self) -> None:
        uri = "supply-chain://resources/network_distribution_geojson.v1-digest"
        result = await server.mcp.call_tool(
            "create_map_card",
            {
                "title": "Network comparison",
                "sources": {
                    "network": {
                        "type": "geojson",
                        "data_ref": network_data_ref(),
                    }
                },
                "layers": [
                    {
                        "id": "network",
                        "type": "circle",
                        "source": "network",
                        "paint": {"circle-color": "#2563eb"},
                    }
                ],
            },
        )

        assert result.structuredContent is not None
        source = result.structuredContent["artifact"]["renderer"]["payload"]["sources"]["network"]
        self.assertEqual(source["data"]["server"], "supply_chain")
        self.assertEqual(source["data"]["uri"], uri)

    async def test_coverage_card_uses_a_compact_profile_without_losing_validation(self) -> None:
        arguments = {
            "title": "印尼现有仓网 12 小时覆盖",
            "intent": "展示需求城市、末端和干线覆盖，以及中心仓和 XD 前置仓",
            "sources": {"network": {"type": "geojson", "data_ref": coverage_data_ref()}},
            "layers": [
                {
                    "id": "last-mile",
                    "type": "line",
                    "source": "network",
                    "filter": ["==", ["get", "kind"], "last_mile_assignment"],
                    "paint": {
                        "line-color": "#2563EB",
                        "line-opacity": 0.5,
                        "line-width": 1.5,
                    },
                },
                {
                    "id": "linehaul",
                    "type": "line",
                    "source": "network",
                    "filter": ["==", ["get", "kind"], "linehaul_connection"],
                    "paint": {
                        "line-color": "#1E3A8A",
                        "line-opacity": 0.65,
                        "line-width": 2.5,
                    },
                },
                {
                    "id": "on-time-cities",
                    "type": "circle",
                    "source": "network",
                    "filter": [
                        "all",
                        ["==", ["get", "kind"], "demand"],
                        ["<=", ["get", "duration_hours"], 12],
                    ],
                    "paint": {
                        "circle-color": "#16A34A",
                        "circle-radius": 5,
                        "circle-stroke-color": "#FFFFFF",
                        "circle-stroke-width": 1.5,
                    },
                },
                {
                    "id": "late-cities",
                    "type": "circle",
                    "source": "network",
                    "filter": [
                        "all",
                        ["==", ["get", "kind"], "demand"],
                        [">", ["get", "duration_hours"], 12],
                    ],
                    "paint": {
                        "circle-color": "#DC2626",
                        "circle-radius": 6,
                        "circle-stroke-color": "#FFFFFF",
                        "circle-stroke-width": 1.75,
                    },
                },
                {
                    "id": "xd-warehouses",
                    "type": "circle",
                    "source": "network",
                    "filter": [
                        "all",
                        ["==", ["get", "kind"], "warehouse"],
                        ["==", ["get", "warehouse_type"], "cross_docking"],
                    ],
                    "paint": {
                        "circle-color": "#F97316",
                        "circle-radius": 9,
                        "circle-stroke-color": "#FFFFFF",
                        "circle-stroke-width": 2,
                    },
                },
                {
                    "id": "center-warehouses",
                    "type": "circle",
                    "source": "network",
                    "filter": [
                        "all",
                        ["==", ["get", "kind"], "warehouse"],
                        ["==", ["get", "warehouse_type"], "center"],
                    ],
                    "paint": {
                        "circle-color": "#1D4ED8",
                        "circle-radius": 12,
                        "circle-stroke-color": "#FFFFFF",
                        "circle-stroke-width": 2.5,
                    },
                },
            ],
            "extensions": {
                "hover": {
                    "layers": [
                        {
                            "layer": "on-time-cities",
                            "title_property": "city_name",
                            "fields": ["demand_quantity", "assigned_warehouse_id"],
                        },
                        {
                            "layer": "xd-warehouses",
                            "title_property": "warehouse_name",
                            "fields": ["warehouse_id", "city_name"],
                        },
                        {
                            "layer": "center-warehouses",
                            "title_property": "warehouse_name",
                            "fields": ["warehouse_id", "city_name"],
                        },
                        {
                            "layer": "linehaul",
                            "title_property": "upstream_center_id",
                            "fields": ["crossdock_warehouse_id", "assigned_demand"],
                        },
                    ]
                },
                "legend": {
                    "items": [
                        {"label": "达标城市", "color": "#16A34A", "type": "circle"},
                        {"label": "未达标城市", "color": "#DC2626", "type": "circle"},
                        {"label": "XD 前置仓", "color": "#F97316", "type": "circle"},
                        {"label": "中心仓", "color": "#1D4ED8", "type": "circle"},
                        {"label": "末端覆盖", "color": "#2563EB", "type": "line"},
                        {"label": "干线覆盖", "color": "#1E3A8A", "type": "line"},
                    ]
                },
            },
        }

        result = await server.mcp.call_tool("create_map_card", arguments)

        assert result.structuredContent is not None
        payload = result.structuredContent["artifact"]["renderer"]["payload"]
        self.assertEqual(
            [layer["id"] for layer in payload["layers"]],
            [
                "last-mile",
                "linehaul",
                "on-time-cities",
                "late-cities",
                "xd-warehouses",
                "center-warehouses",
            ],
        )

    async def test_rejects_all_null_fields_and_wrong_numeric_expression_input(self) -> None:
        null_profile = coverage_data_ref()
        demand = null_profile["profile"]["feature_types"][0]
        assert isinstance(demand, dict)
        properties = demand["properties"]
        assert isinstance(properties, dict)
        properties["before_duration_hours"] = "null"
        with self.assertRaisesRegex(ToolError, "no non-null GeoJSON values"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Invalid empty time",
                    "sources": {"network": {"type": "geojson", "data_ref": null_profile}},
                    "layers": [
                        {
                            "id": "cities",
                            "type": "circle",
                            "source": "network",
                            "filter": ["==", ["get", "kind"], "demand"],
                            "paint": {
                                "circle-color": [
                                    "case",
                                    ["<=", ["get", "before_duration_hours"], 12],
                                    "#16A34A",
                                    "#DC2626",
                                ]
                            },
                        }
                    ],
                },
            )

        string_profile = deepcopy(coverage_data_ref())
        demand = string_profile["profile"]["feature_types"][0]
        assert isinstance(demand, dict)
        properties = demand["properties"]
        assert isinstance(properties, dict)
        properties["demand_quantity"] = "string"
        with self.assertRaisesRegex(ToolError, "demand_quantity to be number"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Invalid city size",
                    "sources": {"network": {"type": "geojson", "data_ref": string_profile}},
                    "layers": [
                        {
                            "id": "cities",
                            "type": "circle",
                            "source": "network",
                            "filter": ["==", ["get", "kind"], "demand"],
                            "paint": {
                                "circle-radius": [
                                    "interpolate",
                                    ["linear"],
                                    ["get", "demand_quantity"],
                                    300,
                                    5,
                                    2500,
                                    12,
                                ]
                            },
                        }
                    ],
                },
            )

    async def test_rejects_icon_images_until_the_card_declares_assets(self) -> None:
        with self.assertRaisesRegex(ToolError, "no declared image assets"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Unavailable symbol",
                    "sources": {
                        "network": {"type": "geojson", "data_ref": network_data_ref()}
                    },
                    "layers": [
                        {
                            "id": "warehouses",
                            "type": "symbol",
                            "source": "network",
                            "filter": ["==", ["get", "kind"], "warehouse"],
                            "layout": {"icon-image": "square"},
                        }
                    ],
                },
            )

    async def test_rejects_public_host_and_model_visible_resource_identities(
        self,
    ) -> None:
        for server_name, uri in [
            ("mcp__map_utils", "maps-data://geojson/map-data-1234"),
            ("map_utils", "file:///tmp/network.geojson"),
            ("map_utils", "https://example.com/network.geojson"),
        ]:
            with self.subTest(server_name=server_name, uri=uri):
                with self.assertRaises(ValidationError):
                    GeoJsonSource.model_validate(
                        {
                            "type": "geojson",
                            "data_ref": {
                                **data_ref(),
                                "server": server_name,
                                "uri": uri,
                            },
                        }
                    )

    async def test_result_requires_standalone_assistant_embed_paragraph(self) -> None:
        result = await server.create_map_card(
            title="路线",
            sources={
                "routes": GeoJsonSource(
                    type="geojson",
                    data_ref=data_ref(),
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
        self.assertEqual(len(result.content), 2)
        message = result.content[0]
        self.assertEqual(message.type, "text")
        assert message.text is not None
        self.assertIn("not displayed until", message.text)
        self.assertIn("standalone paragraph", message.text)
        self.assertIn("may appear anywhere", message.text)
        self.assertIn(f"\n\n{embed_code}\n\n", message.text)
        self.assertEqual(result.content[1].type, "resource_link")

    async def test_official_validator_warns_unknown_and_rejects_invalid_known_syntax(
        self,
    ) -> None:
        warning = await server.mcp.call_tool(
            "create_map_card",
            {
                "title": "Warning",
                "sources": {
                    "data": {"type": "geojson", "data_ref": data_ref()},
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
                        "data": {"type": "geojson", "data_ref": data_ref()},
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
            tool for tool in await server.mcp.list_tools() if tool.name == "create_map_card"
        )
        assert tool.inputSchema is not None
        self.assertEqual(tool.inputSchema["properties"]["layers"]["items"]["type"], "object")
        self.assertEqual(tool.inputSchema["properties"]["sources"]["type"], "object")
        self.assertIn("extensions", tool.inputSchema["properties"])
        self.assertIn("center", tool.inputSchema["properties"])
        self.assertNotIn("view", tool.inputSchema["properties"])
        self.assertNotIn("legend", tool.inputSchema["properties"])
        self.assertIn("data_ref", str(tool.inputSchema))
        assert tool.outputSchema is not None
        self.assertEqual(
            tool.outputSchema["$defs"]["Renderer"]["properties"]["kind"]["const"],
            "map.v3",
        )

    async def test_rejects_harbor_filters_that_do_not_exist_on_demand_points(self) -> None:
        with self.assertRaisesRegex(ToolError, "demand_city_id"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Invalid coverage",
                    "sources": {
                        "network": {"type": "geojson", "data_ref": network_data_ref()}
                    },
                    "layers": [
                        {
                            "id": "covered-cities",
                            "type": "circle",
                            "source": "network",
                            "filter": [
                                "all",
                                ["has", "demand_city_id"],
                                ["<=", ["get", "duration_hours"], 12],
                            ],
                            "paint": {"circle-color": "#16A34A"},
                        }
                    ],
                },
            )

    async def test_validates_fields_within_selected_feature_kind(self) -> None:
        with self.assertRaisesRegex(ToolError, "warehouse_name"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Invalid demand hover",
                    "sources": {
                        "network": {"type": "geojson", "data_ref": network_data_ref()}
                    },
                    "layers": [
                        {
                            "id": "demand",
                            "type": "circle",
                            "source": "network",
                            "filter": ["==", ["get", "kind"], "demand"],
                            "paint": {"circle-color": "#16A34A"},
                        }
                    ],
                    "extensions": {
                        "hover": {
                            "layers": [
                                {
                                    "layer": "demand",
                                    "title_property": "city_name",
                                    "fields": ["warehouse_name"],
                                }
                            ]
                        }
                    },
                },
            )

    async def test_rejects_line_layer_for_point_only_profile(self) -> None:
        with self.assertRaisesRegex(ToolError, "cannot render profiled geometries"):
            await server.mcp.call_tool(
                "create_map_card",
                {
                    "title": "Invalid lines",
                    "sources": {
                        "network": {"type": "geojson", "data_ref": network_data_ref()}
                    },
                    "layers": [
                        {
                            "id": "assignments",
                            "type": "line",
                            "source": "network",
                            "paint": {"line-color": "#94A3B8"},
                        }
                    ],
                },
            )


if __name__ == "__main__":
    unittest.main()
