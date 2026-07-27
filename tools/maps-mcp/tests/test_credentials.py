from __future__ import annotations

import json
import stat
import tempfile
import unittest
from pathlib import Path

from maps_mcp.credentials import WorkspaceCredentialStore


class WorkspaceCredentialStoreTests(unittest.TestCase):
    def test_round_trip_is_workspace_scoped_and_owner_only(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            store = WorkspaceCredentialStore(Path(temp_dir))

            self.assertIsNone(store.get_credential())
            store.set_credential("google", "google-secret")
            self.assertEqual(store.get_credential().provider, "google")
            store.set_credential("mapbox", "mapbox-secret")

            credential = store.get_credential()
            self.assertEqual(credential.provider, "mapbox")
            self.assertEqual(credential.api_key, "mapbox-secret")
            self.assertEqual(stat.S_IMODE(store.memory_path.stat().st_mode), 0o600)
            data = json.loads(store.memory_path.read_text())
            self.assertEqual(data["version"], 2)
            self.assertEqual(set(data), {"version", "provider", "api_key"})

    def test_rejects_unknown_provider(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            store = WorkspaceCredentialStore(Path(temp_dir))
            with self.assertRaisesRegex(ValueError, "Unsupported maps provider"):
                store.set_credential("other", "secret")

    def test_reads_legacy_memory_as_one_selected_provider(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            store = WorkspaceCredentialStore(Path(temp_dir))
            store.memory_dir.mkdir()
            store.memory_path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "providers": {
                            "google": {"api_key": "google-secret"},
                            "mapbox": {"api_key": "mapbox-secret"},
                        },
                    }
                )
            )

            credential = store.get_credential()

            self.assertEqual(credential.provider, "mapbox")
            self.assertEqual(credential.api_key, "mapbox-secret")


if __name__ == "__main__":
    unittest.main()
