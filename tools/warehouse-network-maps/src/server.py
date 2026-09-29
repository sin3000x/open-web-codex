"""FastMCP entry point for provider-neutral Google Maps and Mapbox tools."""

from __future__ import annotations

import argparse
import json
import math
import os
import stat
from pathlib import Path, PurePosixPath
from typing import Annotated, Literal
from uuid import uuid4

from mcp.server.fastmcp import Context, FastMCP
from mcp.server.session import ServerSession
from mcp.types import CallToolResult, ResourceLink, TextContent, ToolAnnotations
from open_web_codex_provider import (
    MAX_WORKSPACE_FILE_BYTES,
    GeoJsonResourceRef,
    ResourceRef,
    create_workspace_file,
    derive_geojson_profile,
    ensure_workspace_directory,
    trusted_workspace_root,
)
from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from .clients import GoogleMapsClient, MapboxMapsClient
from .credential_prompt import LoopbackCredentialPrompt
from .credentials import WorkspaceCredentialStore
from .data_refs import GeoJsonResourceStore, MapCardSpecStore, PublishedGeoJson
from .map_card import (
    Artifact,
    Embed,
    GeoJsonSource,
    MapCardPatch,
    MapCardSpec,
    MapExtensions,
    MapPayload,
    Renderer,
    ToolResult,
    extension_warnings,
    renderer_sources,
    sanitized_extensions,
    validate_extension_graph,
    validate_profile_graph,
    validate_style,
)

Provider = Literal["google", "mapbox"]
TravelMode = Literal["driving", "driving_traffic", "walking", "bicycling", "transit", "two_wheeler"]
MCP_SERVER_NAME = "map_utils"
NAVIGATION_OUTPUT_DIRECTORY = PurePosixPath("outputs/warehouse-network/requests")

LOCAL_PRESENTATION_TOOL = ToolAnnotations(
    readOnlyHint=True,
    destructiveHint=False,
    idempotentHint=False,
    openWorldHint=False,
)
LOCAL_RESOURCE_TOOL = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=False,
    idempotentHint=True,
    openWorldHint=False,
)
EXTERNAL_BILLABLE_TOOL = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=False,
    idempotentHint=False,
    openWorldHint=True,
)


class Point(BaseModel):
    latitude: float = Field(ge=-90, le=90)
    longitude: float = Field(ge=-180, le=180)


class GeoJsonToolResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    provider: str
    summary: str
    feature_count: int = Field(ge=0)
    data_ref: GeoJsonResourceRef = Field(
        description=(
            "Copy this object unchanged into create_map_card sources.<source-id>.data_ref. "
            "Its server and uri are the canonical MCP Resource routing identity."
        ),
    )


class NavigationInputIdentity(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_version: Literal["prepared_network_input.v2"]
    content_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")


class NavigationRouteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    origin_id: str = Field(min_length=1, max_length=128)
    destination_id: str = Field(min_length=1, max_length=128)
    layer: Literal["linehaul", "last_mile"]
    origin_longitude: float = Field(ge=-180, le=180)
    origin_latitude: float = Field(ge=-90, le=90)
    destination_longitude: float = Field(ge=-180, le=180)
    destination_latitude: float = Field(ge=-90, le=90)


class ExistingOnlyWarehouseScope(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: Literal["existing_only"] = "existing_only"


class ExistingPlusCandidatesWarehouseScope(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: Literal["existing_plus_candidates"] = "existing_plus_candidates"
    candidate_ids: list[str] = Field(min_length=1, max_length=256)

    @field_validator("candidate_ids")
    @classmethod
    def normalize_candidate_ids(cls, values: list[str]) -> list[str]:
        if any(not value for value in values):
            raise ValueError("warehouse_scope_candidate_id_invalid")
        if len(values) != len(set(values)):
            raise ValueError("warehouse_scope_candidate_ids_duplicate")
        return sorted(values)


class AllWarehousesScope(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: Literal["all_warehouses"] = "all_warehouses"


WarehouseScope = Annotated[
    ExistingOnlyWarehouseScope | ExistingPlusCandidatesWarehouseScope | AllWarehousesScope,
    Field(discriminator="kind"),
]


class NavigationMatrixRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_version: Literal["navigation_matrix_request.v2"]
    input_identity: NavigationInputIdentity
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    routes: list[NavigationRouteRequest] = Field(min_length=1, max_length=2_500)
    estimated_billable_elements: int = Field(ge=0)

    @model_validator(mode="after")
    def validate_warehouse_ids(self):
        if self.warehouse_ids != sorted(set(self.warehouse_ids)):
            raise ValueError("warehouse_ids_not_canonical")
        keys = [(route.origin_id, route.destination_id, route.layer) for route in self.routes]
        if len(keys) != len(set(keys)):
            raise ValueError("navigation_route_duplicate_pair")
        if any(route.origin_id not in self.warehouse_ids for route in self.routes):
            raise ValueError("navigation_route_origin_outside_warehouse_set")
        return self


class NavigationRouteRow(BaseModel):
    model_config = ConfigDict(extra="forbid")

    origin_id: str
    destination_id: str
    layer: Literal["linehaul", "last_mile"]
    distance_km: float = Field(ge=0)
    duration_hours: float = Field(ge=0)
    method: Literal["navigation"] = "navigation"
    tool_version: str = "maps-navigation.v1"
    status: Literal["ready", "unreachable", "error"]
    origin_longitude: float
    origin_latitude: float
    destination_longitude: float
    destination_latitude: float
    navigation_provider: str = Field(min_length=1, max_length=128)
    navigation_profile: str = Field(min_length=1, max_length=128)


class NavigationMatrixResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_version: Literal["navigation_matrix_result.v2"] = "navigation_matrix_result.v2"
    input_identity: NavigationInputIdentity
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    rows: list[NavigationRouteRow] = Field(min_length=1, max_length=2_500)

    @model_validator(mode="after")
    def validate_warehouse_ids(self):
        if self.warehouse_ids != sorted(set(self.warehouse_ids)):
            raise ValueError("warehouse_ids_not_canonical")
        keys = [(row.origin_id, row.destination_id, row.layer) for row in self.rows]
        if len(keys) != len(set(keys)):
            raise ValueError("navigation_route_duplicate_pair")
        if any(row.origin_id not in self.warehouse_ids for row in self.rows):
            raise ValueError("navigation_route_origin_outside_warehouse_set")
        return self


class NavigationExecutionToolResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    summary: str
    navigation_matrix_relative_path: str = Field(min_length=1, max_length=1024)
    provider: Provider
    ready_pair_count: int = Field(ge=0)
    unreachable_pair_count: int = Field(ge=0)
    error_pair_count: int = Field(ge=0)


mcp = FastMCP(
    "Map Utils",
    instructions=(
        "Geocoding and routing tools publish GeoJSON as MCP Resources plus a bounded profile "
        "derived from that exact content. Copy structuredContent.data_ref, including profile, "
        "unchanged into create_map_card sources.<source-id>.data_ref. For "
        "read_mcp_resource, pass "
        "data_ref.server as server and data_ref.uri as uri unchanged. References produced by "
        "this server use map_utils; create_map_card may also consume an unchanged GeoJSON "
        "reference from another reviewed local MCP server. Never use a model-visible mcp__ "
        "namespace or copy Resource JSON into the assistant reply. "
        "create_map_card accepts standard non-image Mapbox Style Specification layer JSON. "
        "The current map.v3 renderer does not declare image assets, so icon-image is rejected. "
        "Open Web "
        "manages GeoJSON source data and adds optional extensions.hover and "
        "extensions.legend. The official Mapbox validator reports unknown style properties "
        "as warnings and rejects invalid known syntax. The Tool returns a schema-validated "
        "typed visualization Artifact and an exact assistant embed directive. A successful Tool "
        "call only creates the Artifact and does not display the map. To display it, copy "
        "structuredContent.embed.code verbatim into the Assistant response as a standalone "
        "paragraph, with a blank line before and after it. The paragraph may appear anywhere in "
        "the response where the map should be shown. Do not wrap it in a code fence, blockquote, "
        "or list. One selected provider and API key are shared by every maps tool."
    ),
    json_response=True,
)
_credential_store = WorkspaceCredentialStore(Path.cwd())
_resource_store = GeoJsonResourceStore(Path.cwd())
_map_card_spec_store = MapCardSpecStore(Path.cwd())


def _map_card_spec_ref(uri: str) -> ResourceRef:
    return ResourceRef(
        server=MCP_SERVER_NAME,
        uri=uri,
        resource_schema="map_card_spec.v1",
    )


def _map_card_spec_link(resource_id: str, uri: str, size: int) -> ResourceLink:
    return ResourceLink(
        type="resource_link",
        name=resource_id,
        title="map_card_spec.v1",
        uri=uri,
        description="Reusable map presentation spec",
        mimeType="application/json",
        size=size,
    )


@mcp.resource(
    "maps-data://geojson/{resource_id}",
    name="maps_geojson",
    title="Maps GeoJSON",
    mime_type="application/geo+json",
)
def read_geojson_resource(resource_id: str) -> str:
    """Read GeoJSON previously published by a maps data tool."""
    return _resource_store.read(resource_id)


@mcp.resource(
    "maps-data://map-card-spec/{resource_id}",
    name="map_card_spec",
    title="Map card presentation spec",
    mime_type="application/json",
)
def read_map_card_spec(resource_id: str) -> str:
    """Read one immutable provider-owned map presentation spec."""
    return _map_card_spec_store.read(resource_id)


async def _client(ctx: Context[ServerSession, None]):
    credential = _credential_store.get_credential()
    if credential is None:
        prompt = LoopbackCredentialPrompt()
        prompt.start()
        try:
            result = await ctx.elicit_url(
                message=(
                    "A maps provider and API key are required. Configure Mapbox or Google "
                    "in this app; the selected provider will be saved globally and reused."
                ),
                url=prompt.url,
                elicitation_id=prompt.elicitation_id,
            )
            action = getattr(result.action, "value", result.action)
            if action != "accept":
                raise RuntimeError("Maps provider configuration was declined or cancelled")
            submission = await prompt.wait()
            credential = submission
            if submission.remember:
                _credential_store.set_credential(submission.provider, submission.api_key)
                await ctx.info(
                    f"Stored {submission.provider} as the active maps provider "
                    "in workspace credential memory"
                )
            await ctx.session.send_elicit_complete(prompt.elicitation_id)
        finally:
            await prompt.close()
    if credential.provider == "google":
        return GoogleMapsClient(credential.api_key)
    return MapboxMapsClient(credential.api_key)


def _workspace_json_path(
    ctx: Context[ServerSession, None], relative_path: str
) -> tuple[Path, Path]:
    """Resolve one model-visible Workspace JSON path without symlink traversal."""
    try:
        workspace = trusted_workspace_root(ctx.request_context.meta)
    except Exception as error:
        raise ValueError(
            "workspace_scope_invalid: 无法从请求元数据 sandboxCwd 解析工作区根目录；"
            "请检查服务器会话配置"
        ) from error
    if not isinstance(relative_path, str) or not relative_path or "\\" in relative_path:
        raise ValueError("workspace_path_invalid")
    relative = PurePosixPath(relative_path)
    if (
        relative.is_absolute()
        or not relative.parts
        or any(part in {"", ".", ".."} for part in relative.parts)
    ):
        raise ValueError("workspace_path_invalid")
    path = workspace
    for part in relative.parts:
        path = path / part
        try:
            mode = path.lstat().st_mode
        except FileNotFoundError as error:
            raise ValueError(f"workspace_source_not_found: {relative_path} 不存在") from error
        if stat.S_ISLNK(mode):
            raise ValueError("workspace_source_symlink_rejected")
    if not stat.S_ISREG(path.lstat().st_mode) or path.suffix.lower() not in {".json", ".geojson"}:
        raise ValueError("workspace_json_required")
    try:
        path.resolve(strict=True).relative_to(workspace.resolve(strict=True))
    except ValueError as error:
        raise ValueError(
            f"workspace_source_escape_rejected: {relative_path} 超出授权工作区根目录"
        ) from error
    return workspace, path


def _read_navigation_request(
    ctx: Context[ServerSession, None],
    navigation_request_relative_path: str,
) -> tuple[Path, NavigationMatrixRequest]:
    workspace, path = _workspace_json_path(ctx, navigation_request_relative_path)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        return workspace, NavigationMatrixRequest.model_validate(payload)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        raise ValueError("navigation_request_invalid") from error


def _duration_seconds(value: object) -> float | None:
    if isinstance(value, (int, float)) and math.isfinite(float(value)) and float(value) >= 0:
        return float(value)
    if isinstance(value, str) and value.endswith("s"):
        try:
            seconds = float(value[:-1])
        except ValueError:
            return None
        return seconds if math.isfinite(seconds) and seconds >= 0 else None
    return None


def _matrix_row(
    request: NavigationRouteRequest,
    entry: object,
    *,
    provider: Provider,
    mode: TravelMode,
) -> NavigationRouteRow:
    item = entry if isinstance(entry, dict) else {}
    distance = item.get("distanceMeters")
    duration = _duration_seconds(item.get("durationSeconds") or item.get("duration"))
    valid_distance = (
        isinstance(distance, (int, float))
        and math.isfinite(float(distance))
        and float(distance) >= 0
    )
    condition = str(item.get("condition") or "").upper()
    if valid_distance and duration is not None:
        status: Literal["ready", "unreachable", "error"] = "ready"
        distance_km = float(distance) / 1000
        duration_hours = duration / 3600
    elif (
        condition in {"ROUTE_NOT_FOUND", "ROUTE_NOT_EXISTS", "NO_ROUTE"}
        or not item
        or (distance is None and duration is None and "status" not in item)
    ):
        status = "unreachable"
        distance_km = 0
        duration_hours = 0
    else:
        status = "error"
        distance_km = 0
        duration_hours = 0
    return NavigationRouteRow(
        origin_id=request.origin_id,
        destination_id=request.destination_id,
        layer=request.layer,
        distance_km=distance_km,
        duration_hours=duration_hours,
        status=status,
        origin_longitude=request.origin_longitude,
        origin_latitude=request.origin_latitude,
        destination_longitude=request.destination_longitude,
        destination_latitude=request.destination_latitude,
        navigation_provider=provider,
        navigation_profile=mode,
    )


def _resource_result(
    provider: object,
    summary: str,
    geojson: dict[str, object],
) -> CallToolResult:
    published = _resource_store.publish(geojson)
    structured = GeoJsonToolResult(
        provider=str(provider),
        summary=summary,
        feature_count=len(geojson.get("features", [])),
        data_ref=GeoJsonResourceRef(
            server=MCP_SERVER_NAME,
            uri=published.uri,
            resource_schema="geojson.v1",
            profile=derive_geojson_profile(geojson),
        ),
    ).model_dump(mode="json")
    return CallToolResult(
        content=[
            TextContent(type="text", text=summary),
            _resource_link(published, summary),
        ],
        structuredContent=structured,
    )


def _resource_link(published: PublishedGeoJson, description: str) -> ResourceLink:
    return ResourceLink(
        type="resource_link",
        name=published.resource_id,
        title="Maps GeoJSON",
        uri=published.uri,
        description=description,
        mimeType="application/geo+json",
        size=published.size,
    )


@mcp.tool(structured_output=True, annotations=LOCAL_RESOURCE_TOOL)
async def publish_workspace_geojson(
    workspace_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    ctx: Context[ServerSession, None],
    require_polygon: bool = False,
) -> Annotated[CallToolResult, GeoJsonToolResult]:
    """Publish one validated Workspace GeoJSON source for a map card."""
    _workspace, path = _workspace_json_path(ctx, workspace_relative_path)
    try:
        geojson = json.loads(path.read_text(encoding="utf-8"))
        profile = derive_geojson_profile(geojson)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        raise ValueError("workspace_geojson_invalid") from error
    if require_polygon and not {
        geometry
        for feature_type in profile.feature_types
        for geometry in feature_type.geometry_types
    } & {"Polygon", "MultiPolygon"}:
        raise ValueError("workspace_geojson_polygons_required")
    return _resource_result(
        "workspace",
        (
            f"Published {profile.feature_count} Workspace GeoJSON features from "
            f"{workspace_relative_path}."
        ),
        geojson,
    )


def _geocode_geojson(result: dict[str, object]) -> dict[str, object]:
    features: list[dict[str, object]] = []
    results = result.get("results")
    if isinstance(results, list):
        for entry in results:
            if not isinstance(entry, dict):
                continue
            match = entry.get("match")
            if not isinstance(match, dict):
                continue
            location = match.get("location")
            if not isinstance(location, dict):
                continue
            longitude = location.get("longitude")
            latitude = location.get("latitude")
            if not isinstance(longitude, (int, float)) or not isinstance(latitude, (int, float)):
                continue
            properties = {
                "index": entry.get("index"),
                "label": match.get("formatted_address") or entry.get("query"),
                "description": match.get("formatted_address"),
                "query": entry.get("query"),
                "place_id": match.get("place_id") or match.get("mapbox_id"),
            }
            features.append(
                {
                    "type": "Feature",
                    "properties": {key: value for key, value in properties.items() if value},
                    "geometry": {
                        "type": "Point",
                        "coordinates": [longitude, latitude],
                    },
                }
            )
    return {"type": "FeatureCollection", "features": features}


def _decode_polyline(encoded: str) -> list[list[float]]:
    coordinates: list[list[float]] = []
    latitude = 0
    longitude = 0
    index = 0
    while index < len(encoded):
        deltas: list[int] = []
        for _ in range(2):
            shift = 0
            result = 0
            while True:
                byte = ord(encoded[index]) - 63
                index += 1
                result |= (byte & 0x1F) << shift
                shift += 5
                if byte < 0x20:
                    break
            deltas.append(~(result >> 1) if result & 1 else result >> 1)
        latitude += deltas[0]
        longitude += deltas[1]
        coordinates.append([longitude / 100000, latitude / 100000])
    return coordinates


def _route_geojson(
    result: dict[str, object],
    fallback: list[Point],
) -> dict[str, object]:
    features: list[dict[str, object]] = []
    routes = result.get("routes")
    if isinstance(routes, list):
        for index, route in enumerate(routes):
            if not isinstance(route, dict):
                continue
            geometry = route.get("geometry")
            if not isinstance(geometry, dict):
                polyline = route.get("polyline")
                encoded = polyline.get("encodedPolyline") if isinstance(polyline, dict) else None
                if isinstance(encoded, str):
                    geometry = {
                        "type": "LineString",
                        "coordinates": _decode_polyline(encoded),
                    }
            if not isinstance(geometry, dict):
                geometry = {
                    "type": "LineString",
                    "coordinates": [[point.longitude, point.latitude] for point in fallback],
                }
            features.append(
                {
                    "type": "Feature",
                    "properties": {
                        "index": index,
                        "label": f"Route {index + 1}",
                        "distance_meters": route.get("distanceMeters") or route.get("distance"),
                        "duration": route.get("duration"),
                    },
                    "geometry": geometry,
                }
            )
    return {"type": "FeatureCollection", "features": features}


def _network_feature_type(data_ref: GeoJsonResourceRef, value: str):
    if data_ref.profile.discriminator_property != "kind":
        raise ValueError("network_map_requires_kind_profile")
    return next((item for item in data_ref.profile.feature_types if item.value == value), None)


def _network_layers(
    network_data_ref: GeoJsonResourceRef,
    boundary_data_ref: GeoJsonResourceRef | None,
) -> tuple[dict[str, GeoJsonSource], list[dict[str, object]], MapExtensions | None]:
    """Build the fixed domain presentation from exact profiled map facts."""
    sources: dict[str, GeoJsonSource] = {
        "network": GeoJsonSource(type="geojson", data_ref=network_data_ref),
    }
    layers: list[dict[str, object]] = []
    legend_items: list[dict[str, str]] = []
    if boundary_data_ref is not None:
        geometry_types = {
            geometry
            for feature_type in boundary_data_ref.profile.feature_types
            for geometry in feature_type.geometry_types
        }
        if not geometry_types & {"Polygon", "MultiPolygon"}:
            raise ValueError("network_map_boundary_polygons_required")
        sources["boundaries"] = GeoJsonSource(type="geojson", data_ref=boundary_data_ref)
        layers.extend(
            [
                {
                    "id": "administrative-boundaries-fill",
                    "type": "fill",
                    "source": "boundaries",
                    "paint": {"fill-color": "#94A3B8", "fill-opacity": 0.12},
                },
                {
                    "id": "administrative-boundaries-line",
                    "type": "line",
                    "source": "boundaries",
                    "paint": {"line-color": "#64748B", "line-width": 1, "line-opacity": 0.7},
                },
            ]
        )
        legend_items.append({"label": "行政区边界", "color": "#64748B", "type": "line"})
    last_mile = _network_feature_type(network_data_ref, "last_mile_assignment")
    if last_mile is not None and last_mile.properties.get("service_status") == "string":
        for status, layer_id, color, label in (
            ("attained", "attained-last-mile-coverage", "#16A34A", "达标覆盖"),
            ("missed", "missed-last-mile-coverage", "#DC2626", "未达标覆盖"),
        ):
            layers.append(
                {
                    "id": layer_id,
                    "type": "line",
                    "source": "network",
                    "filter": [
                        "all",
                        ["==", ["get", "kind"], "last_mile_assignment"],
                        ["==", ["get", "service_status"], status],
                    ],
                    "paint": {
                        "line-color": color,
                        "line-width": 1.75,
                        "line-opacity": 0.7,
                    },
                }
            )
            legend_items.append({"label": label, "color": color, "type": "line"})
    elif last_mile is not None:
        layers.append(
            {
                "id": "last-mile-coverage",
                "type": "line",
                "source": "network",
                "filter": ["==", ["get", "kind"], "last_mile_assignment"],
                "paint": {
                    "line-color": "#2563EB",
                    "line-width": 1.5,
                    "line-opacity": 0.65,
                },
            }
        )
        legend_items.append({"label": "末端覆盖", "color": "#2563EB", "type": "line"})
    if _network_feature_type(network_data_ref, "linehaul_connection") is not None:
        layers.append(
            {
                "id": "linehaul-coverage",
                "type": "line",
                "source": "network",
                "filter": ["==", ["get", "kind"], "linehaul_connection"],
                "paint": {"line-color": "#1E3A8A", "line-width": 2.5, "line-opacity": 0.65},
            }
        )
        legend_items.append({"label": "干线连接", "color": "#1E3A8A", "type": "line"})
    demand = _network_feature_type(network_data_ref, "demand")
    if demand is not None and demand.properties.get("service_status") == "string":
        for status, layer_id, color, radius, label in (
            ("attained", "attained-demand-cities", "#16A34A", 5, "达标城市"),
            ("missed", "missed-demand-cities", "#DC2626", 6, "未达标城市"),
            ("unassigned", "unassigned-demand-cities", "#64748B", 6, "无法判断城市"),
        ):
            layers.append(
                {
                    "id": layer_id,
                    "type": "circle",
                    "source": "network",
                    "filter": [
                        "all",
                        ["==", ["get", "kind"], "demand"],
                        ["==", ["get", "service_status"], status],
                    ],
                    "paint": {
                        "circle-color": color,
                        "circle-radius": radius,
                        "circle-stroke-color": "#FFFFFF",
                        "circle-stroke-width": 1.5,
                    },
                }
            )
            legend_items.append({"label": label, "color": color, "type": "circle"})
    elif demand is not None:
        layers.append(
            {
                "id": "demand-cities",
                "type": "circle",
                "source": "network",
                "filter": ["==", ["get", "kind"], "demand"],
                "paint": {
                    "circle-color": "#16A34A",
                    "circle-radius": 5,
                    "circle-stroke-color": "#FFFFFF",
                    "circle-stroke-width": 1.5,
                },
            }
        )
        legend_items.append({"label": "需求城市", "color": "#16A34A", "type": "circle"})
    warehouse = _network_feature_type(network_data_ref, "warehouse")
    if warehouse is not None:
        properties = warehouse.properties
        boolean_counts = warehouse.boolean_property_counts
        definitions = (
            ("existing-center", True, "center", "#1D4ED8", 12, "现有中心仓"),
            ("existing-cross-docking", True, "cross_docking", "#F97316", 9, "现有 XD"),
            ("candidate-center", False, "center", "#7C3AED", 12, "候选中心仓"),
            ("candidate-cross-docking", False, "cross_docking", "#7C3AED", 9, "候选 XD"),
        )
        if {"warehouse_type", "is_existing"} <= set(properties):
            for layer_id, is_existing, warehouse_type, color, radius, label in definitions:
                counts = boolean_counts.get("is_existing")
                if counts is None or (is_existing and counts.true_count == 0) or (
                    not is_existing and counts.false_count == 0
                ):
                    continue
                layers.append(
                    {
                        "id": layer_id,
                        "type": "circle",
                        "source": "network",
                        "filter": [
                            "all",
                            ["==", ["get", "kind"], "warehouse"],
                            ["==", ["get", "is_existing"], is_existing],
                            ["==", ["get", "warehouse_type"], warehouse_type],
                        ],
                        "paint": {
                            "circle-color": color,
                            "circle-radius": radius,
                            "circle-opacity": 0.9,
                            "circle-stroke-color": "#FFFFFF",
                            "circle-stroke-width": 2,
                        },
                    }
                )
                legend_items.append({"label": label, "color": color, "type": "circle"})
            for field, layer_id, color, label in (
                ("added_facility", "added-facilities", "#C026D3", "新增启用仓"),
                ("removed_facility", "removed-facilities", "#64748B", "移除现有仓"),
            ):
                counts = boolean_counts.get(field)
                if properties.get(field) != "boolean" or counts is None or counts.true_count == 0:
                    continue
                layers.append(
                    {
                        "id": layer_id,
                        "type": "circle",
                        "source": "network",
                        "filter": [
                            "all",
                            ["==", ["get", "kind"], "warehouse"],
                            ["==", ["get", field], True],
                        ],
                        "paint": {
                            "circle-color": color,
                            "circle-radius": 13,
                            "circle-stroke-color": "#FFFFFF",
                            "circle-stroke-width": 2.5,
                        },
                    }
                )
                legend_items.append({"label": label, "color": color, "type": "circle"})
        else:
            layers.append(
                {
                    "id": "warehouses",
                    "type": "circle",
                    "source": "network",
                    "filter": ["==", ["get", "kind"], "warehouse"],
                    "paint": {"circle-color": "#1D4ED8", "circle-radius": 10},
                }
            )
            legend_items.append({"label": "仓库", "color": "#1D4ED8", "type": "circle"})
    if not layers:
        raise ValueError("network_map_features_unavailable")
    extension_values: dict[str, object] = {}
    if legend_items:
        extension_values["legend"] = {"title": "仓网图例", "items": legend_items}
    hover_layers = _network_hover_layers(network_data_ref, layers)
    if hover_layers:
        extension_values["hover"] = {"layers": hover_layers}
    extensions = MapExtensions.model_validate(extension_values) if extension_values else None
    return sources, layers, extensions


def _network_hover_layers(
    network_data_ref: GeoJsonResourceRef,
    layers: list[dict[str, object]],
) -> list[dict[str, object]]:
    """Create bounded hover fields from the exact v2 GeoJSON profile."""
    layer_ids = {str(layer["id"]) for layer in layers if "id" in layer}
    profile_by_kind = {
        item.value: set(item.properties)
        for item in network_data_ref.profile.feature_types
    }
    result: list[dict[str, object]] = []

    def add(
        layer_id: str,
        title_property: str,
        fields: tuple[str, ...],
        kind: str,
    ) -> None:
        if layer_id not in layer_ids:
            return
        properties = profile_by_kind.get(kind, set())
        available = [field for field in fields if field in properties]
        title = title_property if title_property in properties else None
        if title is None and not available:
            return
        result.append(
            {
                "layer": layer_id,
                "title_property": title,
                "fields": available,
            }
        )

    for layer_id in (
        "demand-cities",
        "attained-demand-cities",
        "missed-demand-cities",
        "unassigned-demand-cities",
    ):
        add(
            layer_id,
            "city_name",
            (
                "city_id",
                "demand_quantity",
                "assigned_warehouse_id",
                "before_warehouse_id",
                "after_warehouse_id",
                "duration_hours",
                "before_duration_hours",
                "after_duration_hours",
                "unit_cost",
                "before_unit_cost",
                "after_unit_cost",
                "service_status",
            ),
            "demand",
        )
    for layer_id in (
        "last-mile-coverage",
        "attained-last-mile-coverage",
        "missed-last-mile-coverage",
    ):
        add(
            layer_id,
            "demand_city_id",
            (
                "scenario",
                "warehouse_id",
                "demand_quantity",
                "duration_hours",
                "unit_cost",
                "service_status",
            ),
            "last_mile_assignment",
        )
    add(
        "linehaul-coverage",
        "crossdock_warehouse_id",
        ("scenario", "upstream_center_id", "assigned_demand"),
        "linehaul_connection",
    )
    for layer_id in (
        "existing-center",
        "existing-cross-docking",
        "candidate-center",
        "candidate-cross-docking",
        "added-facilities",
        "removed-facilities",
    ):
        add(
            layer_id,
            "warehouse_name",
            (
                "warehouse_id",
                "warehouse_type",
                "before_active",
                "after_active",
                "added_facility",
                "removed_facility",
            ),
            "warehouse",
        )
    return result


@mcp.tool(structured_output=True, annotations=LOCAL_PRESENTATION_TOOL)
async def create_map_card(
    title: str,
    sources: dict[str, GeoJsonSource],
    layers: list[dict[str, object]],
    intent: str = "visualization",
    fallback_text: str | None = None,
    summary: str | None = None,
    center: tuple[float, float] | None = None,
    zoom: float | None = None,
    bearing: float | None = None,
    pitch: float | None = None,
    extensions: MapExtensions | None = None,
    parent_spec_ref: ResourceRef | None = None,
) -> Annotated[CallToolResult, ToolResult]:
    """Create the current typed Map Artifact from Mapbox Style layer JSON.

    An MCP Resource data_ref must be copied unchanged from an earlier data tool result in the
    same Run and Thread. ``sources`` is an object keyed by source ID; every source is
    {type:"geojson", data_ref:<complete data_ref including its derived profile>}. GeoJSON
    contents must remain in the authorized Resource and must not be passed through the model
    context. Layer expressions, labels, hover fields, and geometry types are validated against
    that bounded profile. ``layers`` uses the
    official Mapbox Style Specification and
    is passed to ``map.addLayer`` unchanged except that each authorized source ID is replaced
    by its browser-local source ID. Standard Mapbox layer types, paint/layout properties,
    filters, expressions, minzoom/maxzoom, metadata, and source-layer are not redefined here.
    The current ``map.v3`` renderer does not declare image assets, so ``icon-image`` is rejected
    rather than relying on an undeclared Mapbox sprite name. Use standard non-image layers, or a
    declared image-asset contract when that capability is introduced.

    ``center`` and ``zoom`` are standard Mapbox camera fields and must be supplied together;
    omit both to fit all loaded GeoJSON. ``bearing`` and ``pitch`` are also standard camera
    fields. ``extensions.hover`` and ``extensions.legend`` are optional Open Web additions,
    not Mapbox Style fields. Omit either extension when it is not needed. Unknown Mapbox
    properties are returned as official validator warnings; invalid known Mapbox syntax
    fails validation. Unknown Open Web extension fields are ignored with warnings.

    Do not use ``style`` as a wrapper, put hover/legend inside a Mapbox layer, use source
    URLs or put GeoJSON contents in the source object. A successful call only creates the
    Artifact and does not display the map. To display it, copy structuredContent.embed.code
    verbatim into the Assistant response as a standalone paragraph, with a blank line before and
    after it. The paragraph may appear anywhere in the response where the map should be shown. Do
    not wrap it in a code fence, blockquote, or list, and do not reproduce renderer JSON.
    """
    clean_title = title.strip()
    if not clean_title:
        raise ValueError("title is required")
    if not sources:
        raise ValueError("sources must not be empty")
    if not layers:
        raise ValueError("layers must not be empty")
    if (center is None) != (zoom is None):
        raise ValueError("center and zoom must be provided together")
    if center is not None:
        longitude, latitude = center
        if not -180 <= longitude <= 180 or not -90 <= latitude <= 90:
            raise ValueError("center must be [longitude, latitude]")
    if zoom is not None and not 0 <= zoom <= 24:
        raise ValueError("zoom must be between 0 and 24")
    if bearing is not None and not -180 <= bearing <= 180:
        raise ValueError("bearing must be between -180 and 180")
    if pitch is not None and not 0 <= pitch <= 85:
        raise ValueError("pitch must be between 0 and 85")
    if extensions is not None and not isinstance(extensions, MapExtensions):
        extensions = MapExtensions.model_validate(extensions)
    validate_extension_graph(extensions, layers)
    validate_profile_graph(sources, layers, extensions)
    warnings = validate_style(
        sources,
        layers,
        center=center,
        zoom=zoom,
        bearing=bearing,
        pitch=pitch,
    )
    warnings.extend(extension_warnings(extensions))
    card = MapPayload(
        title=clean_title,
        intent=intent.strip() or "visualization",
        status="ready",
        fallback_text=fallback_text.strip() if fallback_text else None,
        summary=summary.strip() if summary else None,
        sources=renderer_sources(sources),
        layers=layers,
        center=center,
        zoom=zoom,
        bearing=bearing,
        pitch=pitch,
        extensions=sanitized_extensions(extensions),
    )
    spec = MapCardSpec(
        title=clean_title,
        intent=intent.strip() or "visualization",
        fallback_text=fallback_text.strip() if fallback_text else None,
        summary=summary.strip() if summary else None,
        sources=sources,
        layers=layers,
        center=center,
        zoom=zoom,
        bearing=bearing,
        pitch=pitch,
        extensions=sanitized_extensions(extensions),
        parentSpecRef=parent_spec_ref,
    )
    published_spec = _map_card_spec_store.publish(spec.model_dump(mode="json", by_alias=True))
    map_spec_ref = _map_card_spec_ref(published_spec.uri)
    artifact_ref = f"map-{uuid4()}"
    embed_code = f'::codex-inline-vis{{artifact="{artifact_ref}"}}'
    result = ToolResult(
        type="open-web-artifact",
        kind="inline-visualization.v1",
        artifact=Artifact(
            ref=artifact_ref,
            renderer=Renderer(kind="map.v3", payload=card),
        ),
        embed=Embed(
            syntax="codex-inline-vis.artifact.v1",
            code=embed_code,
        ),
        map_spec_ref=map_spec_ref,
        warnings=warnings or None,
    )
    structured_content = result.model_dump(mode="json", exclude_none=True)
    warning_text = ""
    if warnings:
        shown_paths = ", ".join(warning.path for warning in warnings[:10])
        remaining = len(warnings) - min(len(warnings), 10)
        suffix = f" and {remaining} more" if remaining else ""
        warning_text = f"Warning: Map input diagnostics: {shown_paths}{suffix}."
    return CallToolResult(
        content=[
            TextContent(
                type="text",
                text=(
                    f"Map visualization ready: {clean_title}. The map is not displayed until "
                    "the Assistant response includes the embed code. Copy this exact code "
                    "verbatim as a standalone paragraph, with a blank line before and after it. "
                    "It may appear anywhere in the response where the map should be shown; do "
                    "not wrap it in a code fence, blockquote, or list:\n\n"
                    f"{embed_code}\n\n"
                    f"{warning_text}"
                ),
            ),
            _map_card_spec_link(
                published_spec.resource_id,
                published_spec.uri,
                published_spec.size,
            ),
        ],
        structuredContent=structured_content,
    )


@mcp.tool(structured_output=True, annotations=LOCAL_PRESENTATION_TOOL)
async def create_network_map_card(
    title: str,
    network_data_ref: GeoJsonResourceRef,
    boundary_data_ref: GeoJsonResourceRef | None = None,
) -> Annotated[CallToolResult, ToolResult]:
    """Create a validated point-line-polygon warehouse map from exact domain GeoJSON."""
    sources, layers, extensions = _network_layers(network_data_ref, boundary_data_ref)
    return await create_map_card(
        title=title,
        sources=sources,
        layers=layers,
        intent="warehouse_network",
        summary="已按仓网语义生成点、覆盖线和可选行政区边界图层。",
        extensions=extensions,
    )


@mcp.tool(structured_output=True, annotations=LOCAL_PRESENTATION_TOOL)
async def present_map_card(
    map_spec_ref: ResourceRef,
) -> Annotated[CallToolResult, ToolResult]:
    """Present an exact existing map spec as a fresh Artifact and embed code.

    Use only a previously returned map_spec_ref whose source result and service
    target have been verified for the current request. This does not recompute
    data or change the immutable map spec.
    """
    if (
        map_spec_ref.server != MCP_SERVER_NAME
        or map_spec_ref.resource_schema != "map_card_spec.v1"
        or not map_spec_ref.uri.startswith("maps-data://map-card-spec/")
    ):
        raise ValueError("map_card_spec_ref_invalid")
    resource_id = map_spec_ref.uri.removeprefix("maps-data://map-card-spec/")
    try:
        spec = MapCardSpec.model_validate(json.loads(_map_card_spec_store.read(resource_id)))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise ValueError("map_card_spec_unavailable") from error
    return await create_map_card(
        title=spec.title,
        sources=spec.sources,
        layers=spec.layers,
        intent=spec.intent,
        fallback_text=spec.fallback_text,
        summary=spec.summary,
        center=spec.center,
        zoom=spec.zoom,
        bearing=spec.bearing,
        pitch=spec.pitch,
        extensions=spec.extensions,
        parent_spec_ref=spec.parent_spec_ref,
    )


@mcp.tool(structured_output=True, annotations=LOCAL_PRESENTATION_TOOL)
async def revise_map_card(
    map_spec_ref: ResourceRef,
    patch: MapCardPatch,
) -> Annotated[CallToolResult, ToolResult]:
    """Create a new map card by applying a bounded patch to one exact map spec."""
    if (
        map_spec_ref.server != MCP_SERVER_NAME
        or map_spec_ref.resource_schema != "map_card_spec.v1"
        or not map_spec_ref.uri.startswith("maps-data://map-card-spec/")
    ):
        raise ValueError("map_card_spec_ref_invalid")
    resource_id = map_spec_ref.uri.removeprefix("maps-data://map-card-spec/")
    try:
        parent = MapCardSpec.model_validate(json.loads(_map_card_spec_store.read(resource_id)))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise ValueError("map_card_spec_unavailable") from error
    values = parent.model_dump(mode="python", by_alias=True)
    for field in patch.model_fields_set:
        values[field] = getattr(patch, field)
    values["parentSpecRef"] = map_spec_ref
    revised = MapCardSpec.model_validate(values)
    result = await create_map_card(
        title=revised.title,
        sources=revised.sources,
        layers=revised.layers,
        intent=revised.intent,
        fallback_text=revised.fallback_text,
        summary=revised.summary,
        center=revised.center,
        zoom=revised.zoom,
        bearing=revised.bearing,
        pitch=revised.pitch,
        extensions=revised.extensions,
        parent_spec_ref=map_spec_ref,
    )
    return result


def _point(point: Point) -> dict[str, float]:
    return point.model_dump()


@mcp.tool(structured_output=True, annotations=EXTERNAL_BILLABLE_TOOL)
async def batch_geocode(
    addresses: list[str],
    ctx: Context[ServerSession, None],
    language: str | None = None,
    region: str | None = None,
) -> Annotated[CallToolResult, GeoJsonToolResult]:
    """Batch-convert addresses to GeoJSON and return its MCP Resource reference."""
    client = await _client(ctx)
    result = await client.batch_geocode(addresses, language=language, region=region)
    geojson = _geocode_geojson(result)
    count = len(geojson["features"])
    return _resource_result(
        result.get("provider"),
        (
            f"Geocoded {count} of {len(addresses)} addresses; use data_ref.server and "
            "data_ref.uri unchanged for map cards and Resource reads."
        ),
        geojson,
    )


@mcp.tool(structured_output=True, annotations=EXTERNAL_BILLABLE_TOOL)
async def batch_reverse_geocode(
    points: list[Point],
    ctx: Context[ServerSession, None],
    language: str | None = None,
    region: str | None = None,
) -> Annotated[CallToolResult, GeoJsonToolResult]:
    """Batch-convert coordinates to GeoJSON and return its MCP Resource reference."""
    client = await _client(ctx)
    result = await client.batch_reverse_geocode(
        [_point(point) for point in points], language=language, region=region
    )
    geojson = _geocode_geojson(result)
    count = len(geojson["features"])
    return _resource_result(
        result.get("provider"),
        (
            f"Reverse-geocoded {count} of {len(points)} coordinates; use data_ref.server and "
            "data_ref.uri unchanged for map cards and Resource reads."
        ),
        geojson,
    )


@mcp.tool(structured_output=True, annotations=EXTERNAL_BILLABLE_TOOL)
async def get_route(
    origin: Point,
    destination: Point,
    ctx: Context[ServerSession, None],
    waypoints: list[Point] | None = None,
    mode: TravelMode = "driving",
    alternatives: bool = False,
    include_steps: bool = False,
    language: str | None = None,
) -> Annotated[CallToolResult, GeoJsonToolResult]:
    """Request route GeoJSON and return its MCP Resource reference."""
    client = await _client(ctx)
    route_points = [origin, *(waypoints or []), destination]
    result = await client.get_route(
        _point(origin),
        _point(destination),
        waypoints=[_point(point) for point in waypoints or []],
        mode=mode,
        alternatives=alternatives,
        include_steps=include_steps,
        language=language,
    )
    geojson = _route_geojson(result, route_points)
    count = len(geojson["features"])
    return _resource_result(
        result.get("provider"),
        (
            f"Created {count} route geometries; use data_ref.server and data_ref.uri unchanged "
            "for map cards and Resource reads."
        ),
        geojson,
    )


@mcp.tool(structured_output=True, annotations=EXTERNAL_BILLABLE_TOOL)
async def execute_navigation_matrix(
    navigation_request_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    output_relative_path: Annotated[
        str,
        Field(
            min_length=1,
            max_length=1024,
            description=(
                "Create-new JSON path directly under outputs/warehouse-network/requests/."
            ),
        ),
    ],
    ctx: Context[ServerSession, None],
    mode: TravelMode = "driving",
) -> NavigationExecutionToolResult:
    """Execute one approved Workspace navigation request and write exact lane facts."""
    workspace, request = _read_navigation_request(ctx, navigation_request_relative_path)
    output_path = PurePosixPath(output_relative_path)
    if (
        output_path.is_absolute()
        or output_path.parent != NAVIGATION_OUTPUT_DIRECTORY
        or output_path.suffix.lower() != ".json"
    ):
        raise ValueError("generated_output_path_invalid")
    ensure_workspace_directory(workspace, NAVIGATION_OUTPUT_DIRECTORY.as_posix())
    client = await _client(ctx)
    grouped: dict[tuple[float, float], list[NavigationRouteRequest]] = {}
    for route in request.routes:
        grouped.setdefault((route.origin_longitude, route.origin_latitude), []).append(route)

    rows: list[NavigationRouteRow] = []
    for (longitude, latitude), routes in grouped.items():
        result = await client.distance_matrix(
            [{"longitude": longitude, "latitude": latitude}],
            [
                {
                    "longitude": route.destination_longitude,
                    "latitude": route.destination_latitude,
                }
                for route in routes
            ],
            mode=mode,
        )
        provider = result.get("provider")
        if provider not in {"google", "mapbox"}:
            raise RuntimeError("navigation_provider_result_invalid")
        entries = result.get("entries")
        if not isinstance(entries, list):
            raise RuntimeError("navigation_matrix_entries_invalid")
        by_destination = {
            item.get("destinationIndex"): item
            for item in entries
            if isinstance(item, dict) and item.get("originIndex") == 0
        }
        for index, route in enumerate(routes):
            rows.append(
                _matrix_row(
                    route,
                    by_destination.get(index),
                    provider=provider,
                    mode=mode,
                )
            )

    matrix = NavigationMatrixResult(
        input_identity=request.input_identity,
        warehouse_scope=request.warehouse_scope,
        warehouse_ids=request.warehouse_ids,
        rows=rows,
    )
    try:
        created = create_workspace_file(
            workspace,
            output_path.as_posix(),
            matrix.model_dump_json().encode("utf-8"),
            max_bytes=MAX_WORKSPACE_FILE_BYTES,
        )
    except Exception as error:
        raise ValueError("navigation_matrix_workspace_write_failed") from error
    ready_count = sum(row.status == "ready" for row in rows)
    unreachable_count = sum(row.status == "unreachable" for row in rows)
    error_count = sum(row.status == "error" for row in rows)
    return NavigationExecutionToolResult(
        summary=(
            f"Executed {len(rows)} navigation lanes with {provider}; ready {ready_count}, "
            f"unreachable {unreachable_count}, provider-error {error_count}."
        ),
        navigation_matrix_relative_path=created.relative_path,
        provider=provider,
        ready_pair_count=ready_count,
        unreachable_pair_count=unreachable_count,
        error_pair_count=error_count,
    )


@mcp.tool(annotations=EXTERNAL_BILLABLE_TOOL)
async def distance_matrix(
    origins: list[Point],
    destinations: list[Point],
    ctx: Context[ServerSession, None],
    mode: TravelMode = "driving",
) -> dict[str, object]:
    """Calculate a billable distance/time matrix (not a map data source)."""
    client = await _client(ctx)
    return await client.distance_matrix(
        [_point(point) for point in origins],
        [_point(point) for point in destinations],
        mode=mode,
    )


def main() -> None:
    managed_state_root = os.environ.get("OPEN_WEB_CODEX_DATA_DIR")
    parser = argparse.ArgumentParser(description="Google Maps and Mapbox MCP server")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(managed_state_root) if managed_state_root else Path.cwd(),
        help="Workspace whose .codex directory stores credentials and GeoJSON Resources",
    )
    parser.add_argument(
        "--transport",
        choices=("stdio", "streamable-http"),
        default="stdio",
    )
    args = parser.parse_args()

    global _credential_store, _resource_store, _map_card_spec_store
    _credential_store = WorkspaceCredentialStore(args.workspace_root)
    _resource_store = GeoJsonResourceStore(args.workspace_root)
    _map_card_spec_store = MapCardSpecStore(args.workspace_root)
    mcp.run(transport=args.transport)


if __name__ == "__main__":
    main()
