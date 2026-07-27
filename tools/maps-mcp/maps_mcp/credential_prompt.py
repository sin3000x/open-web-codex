"""One-time loopback page for selecting and configuring a maps provider."""

from __future__ import annotations

import asyncio
import queue
import secrets
import threading
import uuid
from dataclasses import dataclass
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler
from http.server import ThreadingHTTPServer
from urllib.parse import parse_qs

from .credentials import SUPPORTED_PROVIDERS


@dataclass(frozen=True)
class CredentialSubmission:
    provider: str
    api_key: str
    remember: bool


class LoopbackCredentialPrompt:
    """Serve a single-use password form on a randomized 127.0.0.1 URL."""

    def __init__(self) -> None:
        self.elicitation_id = str(uuid.uuid4())
        self._path_token = secrets.token_urlsafe(32)
        self._submissions: queue.Queue[CredentialSubmission] = queue.Queue(maxsize=1)
        handler = self._handler()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(
            target=self._server.serve_forever,
            name="map-utils-credential-prompt",
            daemon=True,
        )

    @property
    def url(self) -> str:
        port = self._server.server_address[1]
        return f"http://127.0.0.1:{port}/{self._path_token}"

    def start(self) -> None:
        self._thread.start()

    async def wait(self, timeout_seconds: float = 300) -> CredentialSubmission:
        try:
            return await asyncio.to_thread(self._submissions.get, True, timeout_seconds)
        except queue.Empty as exc:
            raise TimeoutError("Timed out waiting for the maps API key") from exc

    async def close(self) -> None:
        await asyncio.to_thread(self._server.shutdown)
        self._server.server_close()
        await asyncio.to_thread(self._thread.join, 2)

    def _handler(self):
        expected_path = f"/{self._path_token}"
        submissions = self._submissions

        class CredentialHandler(BaseHTTPRequestHandler):
            server_version = "MapsCredentialPrompt/1"

            def do_GET(self) -> None:
                if self.path != expected_path:
                    self.send_error(HTTPStatus.NOT_FOUND)
                    return
                body = f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Configure maps provider</title>
  <style>
    body {{ font: 16px system-ui; max-width: 42rem; margin: 4rem auto; padding: 0 1rem; }}
    label {{ display: block; margin: 1rem 0 .4rem; }}
    input[type=password], select {{ box-sizing: border-box; width: 100%; padding: .7rem; }}
    button {{ margin-top: 1rem; padding: .7rem 1.1rem; }}
    .note {{ color: #555; }}
  </style>
</head>
<body>
  <h1>Configure maps provider</h1>
  <p class="note">The key is sent only to the local MCP server on 127.0.0.1. It does not pass through the model.</p>
  <form method="post" action="{expected_path}">
    <label for="provider">Provider</label>
    <select id="provider" name="provider" required>
      <option value="mapbox">Mapbox</option>
      <option value="google">Google Maps</option>
    </select>
    <label for="api_key">API key / access token</label>
    <input id="api_key" name="api_key" type="password" required autofocus autocomplete="off">
    <label><input name="remember" type="checkbox" value="yes" checked> Remember in this workspace</label>
    <button type="submit">Save and continue</button>
  </form>
</body>
</html>""".encode()
                self._send(HTTPStatus.OK, body, "text/html; charset=utf-8")

            def do_POST(self) -> None:
                if self.path != expected_path:
                    self.send_error(HTTPStatus.NOT_FOUND)
                    return
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                except ValueError:
                    length = 0
                if length <= 0 or length > 16_384:
                    self.send_error(HTTPStatus.BAD_REQUEST)
                    return
                form = parse_qs(self.rfile.read(length).decode("utf-8", errors="strict"))
                provider = form.get("provider", [""])[0].strip().lower()
                api_key = form.get("api_key", [""])[0].strip()
                if provider not in SUPPORTED_PROVIDERS:
                    self.send_error(HTTPStatus.BAD_REQUEST, "A supported provider is required")
                    return
                if not api_key:
                    self.send_error(HTTPStatus.BAD_REQUEST, "API key is required")
                    return
                try:
                    submissions.put_nowait(
                        CredentialSubmission(
                            provider=provider,
                            api_key=api_key,
                            remember=form.get("remember") == ["yes"],
                        )
                    )
                except queue.Full:
                    self.send_error(HTTPStatus.GONE, "This credential request is already complete")
                    return
                body = b"""<!doctype html><html><body><h1>Key saved</h1><p>You can close this tab and return to Codex.</p></body></html>"""
                self._send(HTTPStatus.OK, body, "text/html; charset=utf-8")

            def _send(self, status: HTTPStatus, body: bytes, content_type: str) -> None:
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Cache-Control", "no-store")
                self.send_header("Referrer-Policy", "no-referrer")
                self.send_header("X-Content-Type-Options", "nosniff")
                self.send_header(
                    "Content-Security-Policy",
                    "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'",
                )
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, _format: str, *_args: object) -> None:
                return

        return CredentialHandler
