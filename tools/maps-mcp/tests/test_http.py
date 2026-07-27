from __future__ import annotations

import asyncio
import json
import os
import unittest
from unittest.mock import patch
from urllib.request import ProxyHandler

from maps_mcp.http import JsonHttpClient
from maps_mcp.http import _url_opener


class _Response:
    def __enter__(self) -> _Response:
        return self

    def __exit__(self, *_: object) -> None:
        return None

    def read(self) -> bytes:
        return json.dumps({"ok": True}).encode()


class _Opener:
    def __init__(self) -> None:
        self.open_calls: list[tuple[object, float]] = []

    def open(self, request: object, *, timeout: float) -> _Response:
        self.open_calls.append((request, timeout))
        return _Response()


class JsonHttpClientTests(unittest.TestCase):
    def test_request_uses_configured_opener(self) -> None:
        opener = _Opener()

        with patch("maps_mcp.http._url_opener", return_value=opener):
            result = asyncio.run(JsonHttpClient().request_json("GET", "https://maps.example.test"))

        self.assertEqual(result, {"ok": True})
        self.assertEqual(len(opener.open_calls), 1)

    def test_opener_uses_all_proxy_as_fallback(self) -> None:
        observed: list[object] = []

        def fake_build_opener(*handlers: object) -> _Opener:
            observed.extend(handlers)
            return _Opener()

        with patch.dict(os.environ, {"ALL_PROXY": "http://localhost:7890"}, clear=True):
            with patch("maps_mcp.http.build_opener", side_effect=fake_build_opener):
                _url_opener()

        proxy_handler = next(handler for handler in observed if isinstance(handler, ProxyHandler))
        self.assertEqual(proxy_handler.proxies["http"], "http://localhost:7890")
        self.assertEqual(proxy_handler.proxies["https"], "http://localhost:7890")


if __name__ == "__main__":
    unittest.main()
