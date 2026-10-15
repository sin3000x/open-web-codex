from __future__ import annotations

import asyncio
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode
from urllib.request import Request
from urllib.request import urlopen

import maps_mcp.server as server
from maps_mcp.clients import GoogleMapsClient, MapboxMapsClient
from maps_mcp.credentials import WorkspaceCredentialStore


class AcceptedResult:
    action = "accept"


class FakeSession:
    def __init__(self) -> None:
        self.completed: list[str] = []

    async def send_elicit_complete(self, elicitation_id: str) -> None:
        self.completed.append(elicitation_id)


class FakeContext:
    def __init__(self, provider="mapbox", api_key="workspace-secret") -> None:
        self.session = FakeSession()
        self.messages: list[str] = []
        self.provider = provider
        self.api_key = api_key

    async def elicit_url(self, *, message: str, url: str, elicitation_id: str):
        self.messages.append(message)
        body = urlencode(
            {"provider": self.provider, "api_key": self.api_key, "remember": "yes"}
        ).encode()
        request = Request(url, data=body, method="POST")
        await asyncio.to_thread(lambda: urlopen(request).read())
        return AcceptedResult()

    async def info(self, message: str) -> None:
        self.messages.append(message)


class ServerCredentialTests(unittest.IsolatedAsyncioTestCase):
    async def test_missing_key_is_collected_and_remembered_per_workspace(self) -> None:
        original_store = server._credential_store
        with tempfile.TemporaryDirectory() as temp_dir:
            store = WorkspaceCredentialStore(Path(temp_dir))
            server._credential_store = store
            context = FakeContext()
            try:
                client = await server._client(context)
            finally:
                server._credential_store = original_store

            self.assertIsInstance(client, MapboxMapsClient)
            credential = store.get_credential()
            self.assertEqual(credential.provider, "mapbox")
            self.assertEqual(credential.api_key, "workspace-secret")
            self.assertEqual(len(context.session.completed), 1)
            self.assertIn(
                "Configure Mapbox or Google Maps; remember it locally if desired.",
                context.messages,
            )
            self.assertTrue(
                any("Stored mapbox as the active maps provider" in message for message in context.messages)
            )

    async def test_platform_rotation_uses_fresh_delivery_not_local_credentials(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            store = WorkspaceCredentialStore(Path(temp_dir))
            store.set_credential("mapbox", "stale-local-secret")
            with (
                patch.object(server, "_credential_source", "platform"),
                patch.object(server, "_credential_store", store),
                patch.object(store, "get_credential", side_effect=AssertionError("local read")),
                patch.object(store, "set_credential", side_effect=AssertionError("local write")),
            ):
                first = FakeContext(api_key="first-platform-secret")
                first_client = await server._client(first)
                self.assertIsInstance(first_client, MapboxMapsClient)
                self.assertEqual(first_client.api_key, "first-platform-secret")
                replacement = FakeContext(provider="google", api_key="replacement-platform-secret")
                replacement_client = await server._client(replacement)
                self.assertIsInstance(replacement_client, GoogleMapsClient)
                self.assertEqual(replacement_client.api_key, "replacement-platform-secret")
                self.assertEqual(len(first.session.completed), 1)
                self.assertEqual(len(replacement.session.completed), 1)
                self.assertNotEqual(first.session.completed, replacement.session.completed)
                self.assertFalse(any("secret" in message for message in first.messages + replacement.messages))

    async def test_platform_rejection_does_not_use_a_local_key(self) -> None:
        class RejectedContext(FakeContext):
            async def elicit_url(self, **kwargs):
                class DeclinedResult:
                    action = "decline"
                return DeclinedResult()

        with tempfile.TemporaryDirectory() as temp_dir:
            store = WorkspaceCredentialStore(Path(temp_dir))
            store.set_credential("mapbox", "must-not-be-used")
            with patch.object(server, "_credential_source", "platform"), patch.object(server, "_credential_store", store):
                context = RejectedContext()
                with self.assertRaisesRegex(RuntimeError, "declined or cancelled"):
                    await server._client(context)
                self.assertEqual(context.session.completed, [])


if __name__ == "__main__":
    unittest.main()
