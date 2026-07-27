"""Small asynchronous JSON HTTP client with bounded retries and secret redaction."""

from __future__ import annotations

import asyncio
import json
from typing import Any
from urllib.error import HTTPError
from urllib.error import URLError
from urllib.parse import urlsplit
from urllib.parse import urlunsplit
from urllib.request import ProxyHandler
from urllib.request import Request
from urllib.request import build_opener
from urllib.request import getproxies


class MapsApiError(RuntimeError):
    """A sanitized error returned by an upstream maps provider."""


class JsonHttpClient:
    def __init__(self, *, timeout_seconds: float = 15, max_attempts: int = 2) -> None:
        self.timeout_seconds = timeout_seconds
        self.max_attempts = max_attempts

    async def request_json(
        self,
        method: str,
        url: str,
        *,
        headers: dict[str, str] | None = None,
        body: object | None = None,
        secret: str | None = None,
    ) -> Any:
        last_error: Exception | None = None
        for attempt in range(self.max_attempts):
            try:
                return await asyncio.to_thread(
                    self._request_json,
                    method,
                    url,
                    headers or {},
                    body,
                    secret,
                )
            except MapsApiError as exc:
                last_error = exc
                if not getattr(exc, "retryable", False) or attempt + 1 >= self.max_attempts:
                    raise
                await asyncio.sleep(0.5 * (2**attempt))
        raise MapsApiError(str(last_error or "maps request failed"))

    def _request_json(
        self,
        method: str,
        url: str,
        headers: dict[str, str],
        body: object | None,
        secret: str | None,
    ) -> Any:
        request_headers = {"Accept": "application/json", **headers}
        request_data = None
        if body is not None:
            request_headers.setdefault("Content-Type", "application/json")
            request_data = json.dumps(body, separators=(",", ":")).encode()
        request = Request(url, data=request_data, headers=request_headers, method=method)
        try:
            # The opener honours the HTTP(S)_PROXY/NO_PROXY variables
            # forwarded by the MCP declaration, with ALL_PROXY as a fallback.
            with _url_opener().open(request, timeout=self.timeout_seconds) as response:
                payload = response.read()
        except HTTPError as exc:
            response_text = exc.read(65_536).decode("utf-8", errors="replace")
            message = _redact(_provider_error_message(response_text), secret)
            safe_url = _safe_url(url)
            error = MapsApiError(f"Maps API HTTP {exc.code} for {safe_url}: {message}")
            error.retryable = exc.code == 429 or 500 <= exc.code < 600
            raise error from exc
        except (URLError, TimeoutError, OSError) as exc:
            safe_message = _redact(str(exc), secret)
            error = MapsApiError(f"Maps API request failed for {_safe_url(url)}: {safe_message}")
            error.retryable = True
            raise error from exc

        try:
            return json.loads(payload)
        except json.JSONDecodeError as exc:
            raise MapsApiError(f"Maps API returned invalid JSON for {_safe_url(url)}") from exc


def _provider_error_message(response_text: str) -> str:
    try:
        payload = json.loads(response_text)
    except json.JSONDecodeError:
        return response_text[:500] or "empty error response"
    if isinstance(payload, dict):
        error = payload.get("error")
        if isinstance(error, dict) and isinstance(error.get("message"), str):
            return error["message"][:500]
        if isinstance(error, str):
            return error[:500]
        if isinstance(payload.get("message"), str):
            return payload["message"][:500]
    return "provider rejected the request"


def _safe_url(url: str) -> str:
    parts = urlsplit(url)
    return urlunsplit((parts.scheme, parts.netloc, parts.path, "", ""))


def _redact(value: str, secret: str | None) -> str:
    return value.replace(secret, "[REDACTED]") if secret else value


def _url_opener():
    proxies = getproxies()
    all_proxy = proxies.pop("all", None)
    if all_proxy:
        proxies.setdefault("http", all_proxy)
        proxies.setdefault("https", all_proxy)
    return build_opener(ProxyHandler(proxies))
