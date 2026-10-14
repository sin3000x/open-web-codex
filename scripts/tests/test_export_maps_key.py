"""Run with Python containing psycopg and cryptography; uses synthetic keys only."""
import base64
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "export-maps-key.py"
spec = importlib.util.spec_from_file_location("export_maps_key", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ExportTests(unittest.TestCase):
    def test_decrypts_fixture_generated_by_rust_secret_store(self):
        # Generated with SecretCipher::seal, not the Python encryptor under test.
        self.assertEqual(module.decrypt_credential(
            "KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio=",
            ("v1", base64.b64decode("6iCSfquAx4G2sYbd"), base64.b64decode(
                "4mh8p7vVQdLEtYYMFVgdA/XuJG+9d1wHj/q5yt5Hn2SlUACRFGfaxRmG/+nTcIBYYITYdTo1HQYLq8DsITZ4UoyDA9M=")),
            "v1"), ("mapbox", "synthetic-test-key"))

    def test_wrong_key_version_and_corruption_fail(self):
        for master, version, nonce in [("invalid", "v1", b"a" * 12),
                                       (base64.b64encode(b"a" * 32).decode(), "v2", b"a" * 12),
                                       (base64.b64encode(b"a" * 32).decode(), "v1", b"a" * 12)]:
            with self.assertRaises(module.ExportError):
                module.decrypt_credential(master, (version, nonce, b"corrupt"), "v1")

    def test_current_credential_format_only(self):
        for value in ["not-json", "[]", "{}", '{"version":1}',
                      '{"version":2,"provider":"google","api_key":"a\\nb"}']:
            with self.assertRaises(module.ExportError):
                module.parse_credential(value, memory=True)

    def test_standalone_file_mode_output_and_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / "standalone.py"
            script.write_bytes(SCRIPT.read_bytes())
            credential = Path(directory) / "memory.json"
            credential.write_text(json.dumps({"version": 2, "provider": "google", "api_key": "synthetic-key"}))
            # -S disables installed site packages: this mode is stdlib-only.
            result = subprocess.run([sys.executable, "-S", str(script), "--credential-file", str(credential)],
                                    cwd=directory, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "synthetic-key\n")
            self.assertIn("google", result.stderr)
            credential.unlink()
            result = subprocess.run([sys.executable, "-S", str(script), "--credential-file", str(credential)],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
