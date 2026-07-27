from __future__ import annotations

import unittest
from typing import Any

from maps_mcp.clients import GoogleMapsClient
from maps_mcp.clients import MapboxMapsClient


class FakeHttpClient:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []

    async def request_json(
        self,
        method: str,
        url: str,
        *,
        headers: dict[str, str] | None = None,
        body: object | None = None,
        secret: str | None = None,
    ) -> Any:
        self.calls.append(
            {"method": method, "url": url, "headers": headers, "body": body, "secret": secret}
        )
        if "geocode.googleapis.com" in url:
            return {
                "results": [
                    {
                        "formattedAddress": "Google result",
                        "placeId": "google-place",
                        "location": {"latitude": 1, "longitude": 2},
                    }
                ]
            }
        if "search/geocode/v6/batch" in url:
            assert isinstance(body, list)
            return {
                "batch": [
                    {
                        "features": [
                            {
                                "id": f"mapbox-{index}",
                                "geometry": {"coordinates": [2, 1]},
                                "properties": {"full_address": f"Result {index}"},
                            }
                        ]
                    }
                    for index, _ in enumerate(body)
                ]
            }
        if "distanceMatrix/v2" in url:
            assert isinstance(body, dict)
            return [
                {
                    "originIndex": origin,
                    "destinationIndex": destination,
                    "distanceMeters": 100,
                    "duration": "10s",
                }
                for origin in range(len(body["origins"]))
                for destination in range(len(body["destinations"]))
            ]
        if "routes.googleapis.com/directions/v2" in url:
            return {"routes": [{"distanceMeters": 123, "duration": "12s"}]}
        if "api.mapbox.com/directions/v5" in url:
            return {
                "code": "Ok",
                "routes": [{"distance": 123, "duration": 12}],
                "waypoints": [],
            }
        if "directions-matrix" in url:
            assert "sources=" in url and "destinations=" in url
            sources = _query_count(url, "sources")
            destinations = _query_count(url, "destinations")
            return {
                "code": "Ok",
                "distances": [[100 for _ in range(destinations)] for _ in range(sources)],
                "durations": [[10 for _ in range(destinations)] for _ in range(sources)],
            }
        raise AssertionError(f"Unexpected fake request: {url}")


def _query_count(url: str, parameter: str) -> int:
    value = url.split(f"{parameter}=", 1)[1].split("&", 1)[0]
    return len(value.split("%3B"))


def point(index: int) -> dict[str, float]:
    return {"latitude": float(index % 80), "longitude": float(index % 170)}


class ClientTests(unittest.IsolatedAsyncioTestCase):
    async def test_google_batch_geocode_uses_key_header_and_preserves_order(self) -> None:
        http = FakeHttpClient()
        client = GoogleMapsClient("secret", http=http)

        result = await client.batch_geocode(["first", "second"])

        self.assertEqual(result["count"], 2)
        self.assertEqual([item["index"] for item in result["results"]], [0, 1])
        self.assertTrue(all(call["headers"]["X-Goog-Api-Key"] == "secret" for call in http.calls))

    async def test_mapbox_geocode_splits_at_fifty(self) -> None:
        http = FakeHttpClient()
        client = MapboxMapsClient("secret", http=http)

        result = await client.batch_geocode([f"address-{index}" for index in range(51)])

        self.assertEqual(result["count"], 51)
        self.assertEqual(len(http.calls), 2)
        self.assertEqual([len(call["body"]) for call in http.calls], [50, 1])

    async def test_reverse_geocode_works_for_both_providers(self) -> None:
        google_http = FakeHttpClient()
        mapbox_http = FakeHttpClient()

        google_result = await GoogleMapsClient("secret", http=google_http).batch_reverse_geocode(
            [point(1), point(2)]
        )
        mapbox_result = await MapboxMapsClient("secret", http=mapbox_http).batch_reverse_geocode(
            [point(1), point(2)]
        )

        self.assertEqual(google_result["count"], 2)
        self.assertEqual(mapbox_result["count"], 2)
        self.assertIn("/geocode/location/", google_http.calls[0]["url"])
        self.assertEqual(mapbox_http.calls[0]["method"], "POST")

    async def test_route_works_for_both_providers(self) -> None:
        google_http = FakeHttpClient()
        mapbox_http = FakeHttpClient()

        google_result = await GoogleMapsClient("secret", http=google_http).get_route(
            point(1), point(3), waypoints=[point(2)], include_steps=True
        )
        mapbox_result = await MapboxMapsClient("secret", http=mapbox_http).get_route(
            point(1), point(3), waypoints=[point(2)], mode="walking", include_steps=True
        )

        self.assertEqual(google_result["route_count"], 1)
        self.assertEqual(mapbox_result["route_count"], 1)
        self.assertEqual(google_http.calls[0]["method"], "POST")
        self.assertEqual(len(google_http.calls[0]["body"]["intermediates"]), 1)
        self.assertIn("/mapbox/walking/", mapbox_http.calls[0]["url"])
        self.assertIn("steps=true", mapbox_http.calls[0]["url"])

    async def test_google_matrix_reassembles_chunk_indexes(self) -> None:
        http = FakeHttpClient()
        client = GoogleMapsClient("secret", http=http)

        result = await client.distance_matrix(
            [point(index) for index in range(26)],
            [point(index) for index in range(26)],
        )

        self.assertEqual(result["element_count"], 676)
        self.assertEqual(len(http.calls), 4)
        self.assertEqual(result["entries"][-1]["originIndex"], 25)
        self.assertEqual(result["entries"][-1]["destinationIndex"], 25)

    async def test_mapbox_matrix_reassembles_chunk_indexes(self) -> None:
        http = FakeHttpClient()
        client = MapboxMapsClient("secret", http=http)

        result = await client.distance_matrix(
            [point(index) for index in range(20)],
            [point(index) for index in range(20)],
        )

        self.assertEqual(result["element_count"], 400)
        self.assertEqual(len(http.calls), 4)
        self.assertEqual(result["entries"][-1]["originIndex"], 19)
        self.assertEqual(result["entries"][-1]["destinationIndex"], 19)

    async def test_matrix_cost_limit_is_enforced_before_request(self) -> None:
        client = GoogleMapsClient("secret", http=FakeHttpClient())
        with self.assertRaisesRegex(ValueError, "billable elements"):
            await client.distance_matrix(
                [point(index) for index in range(51)],
                [point(index) for index in range(50)],
            )


if __name__ == "__main__":
    unittest.main()
