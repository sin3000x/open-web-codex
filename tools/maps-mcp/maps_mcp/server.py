"""FastMCP entry point for provider-neutral Google Maps and Mapbox tools."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Annotated, Literal
from uuid import uuid4

from mcp.server.fastmcp import Context, FastMCP
from mcp.server.session import ServerSession
from mcp.types import CallToolResult, ResourceLink, TextContent
from pydantic import BaseModel, ConfigDict, Field

from .clients import GoogleMapsClient, MapboxMapsClient
from .credential_prompt import LoopbackCredentialPrompt
from .credentials import WorkspaceCredentialStore
from .data_refs import GeoJsonResourceStore, PublishedGeoJson
from .map_card import (
    Artifact as CurrentMapArtifact,
    Embed as CurrentMapEmbed,
    GeoJsonSource as CurrentGeoJsonSource,
    MapExtensions as CurrentMapExtensions,
    MapPayload as CurrentMapPayload,
    Renderer as CurrentMapRenderer,
    ToolResult as CurrentMapToolResult,
    extension_warnings,
    renderer_sources,
    sanitized_extensions,
    validate_extension_graph,
    validate_style,
)

Provider = Literal["google", "mapbox"]
TravelMode = Literal["driving", "driving_traffic", "walking", "bicycling", "transit", "two_wheeler"]
MCP_SERVER_NAME = "map_utils"


class Point(BaseModel):
    latitude: float = Field(ge=-90, le=90)
    longitude: float = Field(ge=-180, le=180)


class McpResourceMapData(BaseModel):
    model_config = ConfigDict(extra="forbid")

    type: Literal["mcp_resource"] = "mcp_resource"
    server: Literal["map_utils"] = Field(
        description=(
            "Raw MCP server ID for Resource reads. Use this exact value, not the "
            "model-visible mcp__map_utils Tool namespace."
        ),
    )
    uri: str = Field(
        pattern=r"^maps-data://geojson/[A-Za-z0-9_.-]{1,128}$",
        description=(
            "Canonical MCP Resource URI. Copy this unchanged into create_map_card and use "
            "the same value as read_mcp_resource.uri when the GeoJSON contents are needed."
        ),
    )
    format: Literal["geojson"] = "geojson"


class GeoJsonToolResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    provider: str
    summary: str
    feature_count: int = Field(ge=0)
    data_ref: McpResourceMapData = Field(
        description=(
            "Copy this object unchanged into create_map_card sources.<source-id>.data_ref. "
            "Its server and uri are the canonical MCP Resource routing identity."
        ),
    )


mcp = FastMCP(
    "Map Utils",
    instructions=(
        "Geocoding and routing tools publish GeoJSON as MCP Resources. Copy structuredContent."
        "data_ref unchanged into create_map_card sources.<source-id>.data_ref. For "
        "read_mcp_resource, pass "
        "data_ref.server as server and data_ref.uri as uri unchanged; the server is map_utils, "
        "never the model-visible mcp__map_utils namespace. Never copy Resource JSON into the "
        "assistant reply. "
        "create_map_card accepts standard Mapbox Style Specification layer JSON. Open Web "
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


@mcp.resource(
    "maps-data://geojson/{resource_id}",
    name="maps_geojson",
    title="Maps GeoJSON",
    mime_type="application/geo+json",
)
def read_geojson_resource(resource_id: str) -> str:
    """Read GeoJSON previously published by a maps data tool."""
    return _resource_store.read(resource_id)


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
        data_ref=McpResourceMapData(server=MCP_SERVER_NAME, uri=published.uri),
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


@mcp.tool(structured_output=True)
async def create_map_card(
    title: str,
    sources: dict[str, CurrentGeoJsonSource],
    layers: list[dict[str, object]],
    intent: str = "visualization",
    fallback_text: str | None = None,
    summary: str | None = None,
    center: tuple[float, float] | None = None,
    zoom: float | None = None,
    bearing: float | None = None,
    pitch: float | None = None,
    extensions: CurrentMapExtensions | None = None,
) -> Annotated[CallToolResult, CurrentMapToolResult]:
    """Create the current typed Map Artifact from Mapbox Style layer JSON.

    An MCP Resource data_ref must be copied unchanged from an earlier data tool result in the
    same Run and Thread. ``sources`` is an object keyed by source ID; every source is
    {type:"geojson", data:<direct GeoJSON>} or {type:"geojson", data_ref:<complete data_ref>},
    with exactly one data field. ``layers`` uses the official Mapbox Style Specification and
    is passed to ``map.addLayer`` unchanged except that each authorized source ID is replaced
    by its browser-local source ID. Standard Mapbox layer types, paint/layout properties,
    filters, expressions, minzoom/maxzoom, metadata, and source-layer are not redefined here.

    ``center`` and ``zoom`` are standard Mapbox camera fields and must be supplied together;
    omit both to fit all loaded GeoJSON. ``bearing`` and ``pitch`` are also standard camera
    fields. ``extensions.hover`` and ``extensions.legend`` are optional Open Web additions,
    not Mapbox Style fields. Omit either extension when it is not needed. Unknown Mapbox
    properties are returned as official validator warnings; invalid known Mapbox syntax
    fails validation. Unknown Open Web extension fields are ignored with warnings.

    Do not use ``style`` as a wrapper, put hover/legend inside a Mapbox layer, use source
    URLs, or put MCP Resource objects in ``source.data``. A successful call only creates the
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
    if extensions is not None and not isinstance(extensions, CurrentMapExtensions):
        extensions = CurrentMapExtensions.model_validate(extensions)
    validate_extension_graph(extensions, layers)
    warnings = validate_style(
        sources,
        layers,
        center=center,
        zoom=zoom,
        bearing=bearing,
        pitch=pitch,
    )
    warnings.extend(extension_warnings(extensions))
    card = CurrentMapPayload(
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
    artifact_ref = f"map-{uuid4()}"
    embed_code = f'::codex-inline-vis{{artifact="{artifact_ref}"}}'
    result = CurrentMapToolResult(
        type="open-web-artifact",
        kind="inline-visualization.v1",
        artifact=CurrentMapArtifact(
            ref=artifact_ref,
            renderer=CurrentMapRenderer(kind="map.v3", payload=card),
        ),
        embed=CurrentMapEmbed(
            syntax="codex-inline-vis.artifact.v1",
            code=embed_code,
        ),
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
            )
        ],
        structuredContent=structured_content,
    )


def _point(point: Point) -> dict[str, float]:
    return point.model_dump()


@mcp.tool(structured_output=True)
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


@mcp.tool(structured_output=True)
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


@mcp.tool(structured_output=True)
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


@mcp.tool()
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
    parser = argparse.ArgumentParser(description="Google Maps and Mapbox MCP server")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path.cwd(),
        help="Workspace whose .codex directory stores credentials and GeoJSON Resources",
    )
    parser.add_argument(
        "--transport",
        choices=("stdio", "streamable-http"),
        default="stdio",
    )
    args = parser.parse_args()

    global _credential_store, _resource_store
    _credential_store = WorkspaceCredentialStore(args.workspace_root)
    _resource_store = GeoJsonResourceStore(args.workspace_root)
    mcp.run(transport=args.transport)


if __name__ == "__main__":
    main()
