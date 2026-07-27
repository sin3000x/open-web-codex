"""Google Maps and Mapbox HTTP adapters used by the MCP tools."""

from __future__ import annotations

import asyncio
import math
from collections.abc import Awaitable
from collections.abc import Callable
from typing import Any
from urllib.parse import quote
from urllib.parse import urlencode

from .http import JsonHttpClient

MAX_BATCH_GEOCODES = 500
MAX_MATRIX_ELEMENTS = 2_500
MAX_ROUTE_POINTS = 25


class GoogleMapsClient:
    def __init__(self, api_key: str, http: JsonHttpClient | None = None) -> None:
        self.api_key = api_key
        self.http = http or JsonHttpClient()

    async def batch_geocode(
        self,
        addresses: list[str],
        *,
        language: str | None = None,
        region: str | None = None,
    ) -> dict[str, object]:
        addresses = _validate_addresses(addresses)

        async def geocode(index: int, address: str) -> dict[str, object]:
            query = _optional_query(languageCode=language, regionCode=region)
            url = f"https://geocode.googleapis.com/v4/geocode/address/{quote(address, safe='')}"
            if query:
                url = f"{url}?{query}"
            data = await self.http.request_json(
                "GET", url, headers={"X-Goog-Api-Key": self.api_key}, secret=self.api_key
            )
            return _google_geocode_result(index, address, data)

        results = await _bounded_batch(addresses, geocode, concurrency=8)
        return {"provider": "google", "count": len(results), "results": results}

    async def batch_reverse_geocode(
        self,
        points: list[dict[str, float]],
        *,
        language: str | None = None,
        region: str | None = None,
    ) -> dict[str, object]:
        points = _validate_points(points, max_items=MAX_BATCH_GEOCODES)

        async def reverse(index: int, point: dict[str, float]) -> dict[str, object]:
            query = _optional_query(languageCode=language, regionCode=region)
            location = f"{point['latitude']},{point['longitude']}"
            url = f"https://geocode.googleapis.com/v4/geocode/location/{location}"
            if query:
                url = f"{url}?{query}"
            data = await self.http.request_json(
                "GET", url, headers={"X-Goog-Api-Key": self.api_key}, secret=self.api_key
            )
            result = _google_geocode_result(index, location, data)
            result["input"] = point
            return result

        results = await _bounded_batch(points, reverse, concurrency=8)
        return {"provider": "google", "count": len(results), "results": results}

    async def get_route(
        self,
        origin: dict[str, float],
        destination: dict[str, float],
        *,
        waypoints: list[dict[str, float]] | None = None,
        mode: str = "driving",
        alternatives: bool = False,
        include_steps: bool = False,
        language: str | None = None,
    ) -> dict[str, object]:
        points = _validate_points(
            [origin, *(waypoints or []), destination], max_items=MAX_ROUTE_POINTS
        )
        field_mask = [
            "routes.distanceMeters",
            "routes.duration",
            "routes.polyline.encodedPolyline",
            "routes.legs.distanceMeters",
            "routes.legs.duration",
        ]
        if include_steps:
            field_mask.extend(
                [
                    "routes.legs.steps.distanceMeters",
                    "routes.legs.steps.staticDuration",
                    "routes.legs.steps.navigationInstruction",
                    "routes.legs.steps.polyline.encodedPolyline",
                ]
            )
        body: dict[str, object] = {
            "origin": _google_waypoint(points[0]),
            "destination": _google_waypoint(points[-1]),
            "travelMode": _google_mode(mode),
            "computeAlternativeRoutes": alternatives,
            "units": "METRIC",
        }
        if len(points) > 2:
            body["intermediates"] = [_google_waypoint(point) for point in points[1:-1]]
        if language:
            body["languageCode"] = language
        data = await self.http.request_json(
            "POST",
            "https://routes.googleapis.com/directions/v2:computeRoutes",
            headers={
                "X-Goog-Api-Key": self.api_key,
                "X-Goog-FieldMask": ",".join(field_mask),
            },
            body=body,
            secret=self.api_key,
        )
        routes = data.get("routes", []) if isinstance(data, dict) else []
        return {"provider": "google", "route_count": len(routes), "routes": routes}

    async def distance_matrix(
        self,
        origins: list[dict[str, float]],
        destinations: list[dict[str, float]],
        *,
        mode: str = "driving",
    ) -> dict[str, object]:
        origins, destinations = _validate_matrix_points(origins, destinations)
        entries: list[dict[str, object]] = []
        for origin_start, origin_chunk in _chunks(origins, 25):
            for destination_start, destination_chunk in _chunks(destinations, 25):
                body = {
                    "origins": [
                        {"waypoint": _google_waypoint(point)} for point in origin_chunk
                    ],
                    "destinations": [
                        {"waypoint": _google_waypoint(point)} for point in destination_chunk
                    ],
                    "travelMode": _google_mode(mode),
                }
                data = await self.http.request_json(
                    "POST",
                    "https://routes.googleapis.com/distanceMatrix/v2:computeRouteMatrix",
                    headers={
                        "X-Goog-Api-Key": self.api_key,
                        "X-Goog-FieldMask": (
                            "originIndex,destinationIndex,status,condition,distanceMeters,duration"
                        ),
                    },
                    body=body,
                    secret=self.api_key,
                )
                if not isinstance(data, list):
                    raise RuntimeError("Google route matrix returned a non-list response")
                for item in data:
                    if not isinstance(item, dict):
                        continue
                    entry = dict(item)
                    entry["originIndex"] = origin_start + int(entry.get("originIndex", 0))
                    entry["destinationIndex"] = destination_start + int(
                        entry.get("destinationIndex", 0)
                    )
                    entries.append(entry)
        entries.sort(key=lambda item: (item["originIndex"], item["destinationIndex"]))
        return {
            "provider": "google",
            "origin_count": len(origins),
            "destination_count": len(destinations),
            "element_count": len(entries),
            "entries": entries,
        }


class MapboxMapsClient:
    def __init__(self, api_key: str, http: JsonHttpClient | None = None) -> None:
        self.api_key = api_key
        self.http = http or JsonHttpClient()

    async def batch_geocode(
        self,
        addresses: list[str],
        *,
        language: str | None = None,
        region: str | None = None,
    ) -> dict[str, object]:
        addresses = _validate_addresses(addresses)
        results: list[dict[str, object]] = []
        for start, chunk in _chunks(addresses, 50):
            body = []
            for address in chunk:
                query: dict[str, object] = {"q": address, "limit": 1, "autocomplete": False}
                if language:
                    query["language"] = language
                if region:
                    query["country"] = region.lower()
                body.append(query)
            data = await self.http.request_json(
                "POST",
                _mapbox_url("https://api.mapbox.com/search/geocode/v6/batch", self.api_key),
                body=body,
                secret=self.api_key,
            )
            batches = data.get("batch", []) if isinstance(data, dict) else []
            for offset, address in enumerate(chunk):
                payload = batches[offset] if offset < len(batches) else {}
                results.append(_mapbox_geocode_result(start + offset, address, payload))
        return {"provider": "mapbox", "count": len(results), "results": results}

    async def batch_reverse_geocode(
        self,
        points: list[dict[str, float]],
        *,
        language: str | None = None,
        region: str | None = None,
    ) -> dict[str, object]:
        points = _validate_points(points, max_items=MAX_BATCH_GEOCODES)
        results: list[dict[str, object]] = []
        for start, chunk in _chunks(points, 50):
            body = []
            for point in chunk:
                query: dict[str, object] = {
                    "longitude": point["longitude"],
                    "latitude": point["latitude"],
                    "limit": 1,
                }
                if language:
                    query["language"] = language
                if region:
                    query["country"] = region.lower()
                body.append(query)
            data = await self.http.request_json(
                "POST",
                _mapbox_url("https://api.mapbox.com/search/geocode/v6/batch", self.api_key),
                body=body,
                secret=self.api_key,
            )
            batches = data.get("batch", []) if isinstance(data, dict) else []
            for offset, point in enumerate(chunk):
                payload = batches[offset] if offset < len(batches) else {}
                result = _mapbox_geocode_result(start + offset, str(point), payload)
                result["input"] = point
                results.append(result)
        return {"provider": "mapbox", "count": len(results), "results": results}

    async def get_route(
        self,
        origin: dict[str, float],
        destination: dict[str, float],
        *,
        waypoints: list[dict[str, float]] | None = None,
        mode: str = "driving",
        alternatives: bool = False,
        include_steps: bool = False,
        language: str | None = None,
    ) -> dict[str, object]:
        points = _validate_points(
            [origin, *(waypoints or []), destination], max_items=MAX_ROUTE_POINTS
        )
        coordinates = ";".join(_mapbox_coordinate(point) for point in points)
        profile = _mapbox_profile(mode, allow_traffic=True)
        query: dict[str, object] = {
            "access_token": self.api_key,
            "alternatives": str(alternatives).lower(),
            "steps": str(include_steps).lower(),
            "geometries": "geojson",
            "overview": "simplified",
        }
        if language:
            query["language"] = language
        url = f"https://api.mapbox.com/directions/v5/{profile}/{coordinates}?{urlencode(query)}"
        data = await self.http.request_json("GET", url, secret=self.api_key)
        routes = data.get("routes", []) if isinstance(data, dict) else []
        return {
            "provider": "mapbox",
            "code": data.get("code") if isinstance(data, dict) else None,
            "route_count": len(routes),
            "routes": routes,
            "waypoints": data.get("waypoints", []) if isinstance(data, dict) else [],
        }

    async def distance_matrix(
        self,
        origins: list[dict[str, float]],
        destinations: list[dict[str, float]],
        *,
        mode: str = "driving",
    ) -> dict[str, object]:
        origins, destinations = _validate_matrix_points(origins, destinations)
        profile = _mapbox_profile(mode, allow_traffic=True)
        max_coordinates = 10 if profile == "mapbox/driving-traffic" else 25
        origin_chunk_size = max_coordinates // 2
        entries: list[dict[str, object]] = []
        for origin_start, origin_chunk in _chunks(origins, origin_chunk_size):
            destination_chunk_size = max_coordinates - len(origin_chunk)
            for destination_start, destination_chunk in _chunks(
                destinations, destination_chunk_size
            ):
                combined = [*origin_chunk, *destination_chunk]
                coordinates = ";".join(_mapbox_coordinate(point) for point in combined)
                sources = ";".join(str(index) for index in range(len(origin_chunk)))
                destination_offset = len(origin_chunk)
                destination_indexes = ";".join(
                    str(destination_offset + index) for index in range(len(destination_chunk))
                )
                query = urlencode(
                    {
                        "access_token": self.api_key,
                        "sources": sources,
                        "destinations": destination_indexes,
                        "annotations": "distance,duration",
                    }
                )
                url = (
                    f"https://api.mapbox.com/directions-matrix/v1/{profile}/"
                    f"{coordinates}?{query}"
                )
                data = await self.http.request_json("GET", url, secret=self.api_key)
                distances = data.get("distances", []) if isinstance(data, dict) else []
                durations = data.get("durations", []) if isinstance(data, dict) else []
                for local_origin in range(len(origin_chunk)):
                    for local_destination in range(len(destination_chunk)):
                        entries.append(
                            {
                                "originIndex": origin_start + local_origin,
                                "destinationIndex": destination_start + local_destination,
                                "distanceMeters": _matrix_value(
                                    distances, local_origin, local_destination
                                ),
                                "durationSeconds": _matrix_value(
                                    durations, local_origin, local_destination
                                ),
                            }
                        )
        return {
            "provider": "mapbox",
            "origin_count": len(origins),
            "destination_count": len(destinations),
            "element_count": len(entries),
            "entries": entries,
        }


async def _bounded_batch(
    values: list[Any],
    function: Callable[[int, Any], Awaitable[dict[str, object]]],
    *,
    concurrency: int,
) -> list[dict[str, object]]:
    semaphore = asyncio.Semaphore(concurrency)

    async def run(index: int, value: Any) -> dict[str, object]:
        async with semaphore:
            try:
                return await function(index, value)
            except Exception as exc:
                return {"index": index, "ok": False, "error": str(exc)[:500]}

    return await asyncio.gather(*(run(index, value) for index, value in enumerate(values)))


def _validate_addresses(addresses: list[str]) -> list[str]:
    if not addresses:
        raise ValueError("At least one address is required")
    if len(addresses) > MAX_BATCH_GEOCODES:
        raise ValueError(f"At most {MAX_BATCH_GEOCODES} addresses are allowed per tool call")
    normalized = [address.strip() for address in addresses]
    if any(not address for address in normalized):
        raise ValueError("Addresses must not be empty")
    return normalized


def _validate_points(
    points: list[dict[str, float]], *, max_items: int
) -> list[dict[str, float]]:
    if not points:
        raise ValueError("At least one coordinate is required")
    if len(points) > max_items:
        raise ValueError(f"At most {max_items} coordinates are allowed")
    normalized: list[dict[str, float]] = []
    for point in points:
        try:
            latitude = float(point["latitude"])
            longitude = float(point["longitude"])
        except (KeyError, TypeError, ValueError) as exc:
            raise ValueError("Each point requires numeric latitude and longitude") from exc
        if not math.isfinite(latitude) or not -90 <= latitude <= 90:
            raise ValueError(f"Invalid latitude: {latitude}")
        if not math.isfinite(longitude) or not -180 <= longitude <= 180:
            raise ValueError(f"Invalid longitude: {longitude}")
        normalized.append({"latitude": latitude, "longitude": longitude})
    return normalized


def _validate_matrix_points(
    origins: list[dict[str, float]], destinations: list[dict[str, float]]
) -> tuple[list[dict[str, float]], list[dict[str, float]]]:
    origins = _validate_points(origins, max_items=MAX_MATRIX_ELEMENTS)
    destinations = _validate_points(destinations, max_items=MAX_MATRIX_ELEMENTS)
    elements = len(origins) * len(destinations)
    if elements > MAX_MATRIX_ELEMENTS:
        raise ValueError(
            f"Matrix contains {elements} billable elements; maximum is {MAX_MATRIX_ELEMENTS}"
        )
    return origins, destinations


def _google_geocode_result(
    index: int, query: str, data: object
) -> dict[str, object]:
    results = data.get("results", []) if isinstance(data, dict) else []
    if not results or not isinstance(results[0], dict):
        return {"index": index, "ok": True, "query": query, "match": None}
    first = results[0]
    location = first.get("location")
    if not isinstance(location, dict):
        geometry = first.get("geometry")
        location = geometry.get("location") if isinstance(geometry, dict) else None
    return {
        "index": index,
        "ok": True,
        "query": query,
        "match": {
            "formatted_address": first.get("formattedAddress")
            or first.get("formatted_address"),
            "place_id": first.get("placeId") or first.get("place_id"),
            "location": location,
            "granularity": first.get("granularity"),
            "types": first.get("types", []),
        },
    }


def _mapbox_geocode_result(
    index: int, query: str, data: object
) -> dict[str, object]:
    features = data.get("features", []) if isinstance(data, dict) else []
    if not features or not isinstance(features[0], dict):
        return {"index": index, "ok": True, "query": query, "match": None}
    feature = features[0]
    properties = feature.get("properties") if isinstance(feature.get("properties"), dict) else {}
    geometry = feature.get("geometry") if isinstance(feature.get("geometry"), dict) else {}
    coordinates = geometry.get("coordinates")
    location = None
    if isinstance(coordinates, list) and len(coordinates) >= 2:
        location = {"longitude": coordinates[0], "latitude": coordinates[1]}
    return {
        "index": index,
        "ok": True,
        "query": query,
        "match": {
            "formatted_address": properties.get("full_address")
            or properties.get("place_formatted")
            or feature.get("place_name")
            or properties.get("name"),
            "mapbox_id": properties.get("mapbox_id") or feature.get("id"),
            "location": location,
            "feature_type": properties.get("feature_type") or feature.get("place_type"),
        },
    }


def _google_waypoint(point: dict[str, float]) -> dict[str, object]:
    return {
        "location": {
            "latLng": {
                "latitude": point["latitude"],
                "longitude": point["longitude"],
            }
        }
    }


def _google_mode(mode: str) -> str:
    modes = {
        "driving": "DRIVE",
        "walking": "WALK",
        "bicycling": "BICYCLE",
        "transit": "TRANSIT",
        "two_wheeler": "TWO_WHEELER",
    }
    try:
        return modes[mode.lower()]
    except KeyError as exc:
        raise ValueError(f"Unsupported Google travel mode: {mode}") from exc


def _mapbox_profile(mode: str, *, allow_traffic: bool) -> str:
    profiles = {
        "driving": "mapbox/driving",
        "walking": "mapbox/walking",
        "bicycling": "mapbox/cycling",
    }
    if allow_traffic:
        profiles["driving_traffic"] = "mapbox/driving-traffic"
    try:
        return profiles[mode.lower()]
    except KeyError as exc:
        raise ValueError(f"Unsupported Mapbox travel mode: {mode}") from exc


def _mapbox_coordinate(point: dict[str, float]) -> str:
    return f"{point['longitude']},{point['latitude']}"


def _mapbox_url(base_url: str, api_key: str) -> str:
    return f"{base_url}?{urlencode({'access_token': api_key})}"


def _optional_query(**values: str | None) -> str:
    return urlencode({key: value for key, value in values.items() if value})


def _chunks(values: list[Any], size: int):
    if size <= 0:
        raise ValueError("Chunk size must be positive")
    for start in range(0, len(values), size):
        yield start, values[start : start + size]


def _matrix_value(matrix: object, row: int, column: int) -> object:
    if not isinstance(matrix, list) or row >= len(matrix) or not isinstance(matrix[row], list):
        return None
    return matrix[row][column] if column < len(matrix[row]) else None
