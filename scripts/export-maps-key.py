#!/usr/bin/env python3
"""Standalone operator export. Copy this file alone; requires Python 3.10+."""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

CONFIG_KEY = "maps.active_credential"
AAD = b"open-web-codex:platform_configuration:global:global:maps.active_credential"
DEPENDENCIES = ("psycopg[binary]==3.3.6", "cryptography==46.0.3")


class ExportError(Exception):
    pass


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data-dir", type=Path,
                        default=Path(os.environ.get("OPEN_WEB_CODEX_DATA_DIR", ".")),
                        help="directory containing database-url and master-key (default: env or cwd)")
    parser.add_argument("--database-url-file", type=Path)
    parser.add_argument("--master-key-file", type=Path)
    parser.add_argument("--credential-file", type=Path,
                        help="explicit MCP maps-tool-memory.json; reads it instead of the database")
    parser.add_argument("--no-install", action="store_true",
                        help="fail if database dependencies are missing; never download packages")
    return parser.parse_args()


def read_file(path):
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        raise ExportError("Cannot read configuration file; check the file path and permissions.") from None


def setting(variable, explicit_file, default_file):
    if explicit_file is not None:
        return read_file(explicit_file).strip()
    if variable in os.environ:
        return os.environ[variable].strip()
    return read_file(default_file).strip()


def parse_credential(plaintext, memory=False):
    try:
        value = json.loads(plaintext)
    except (ValueError, UnicodeError):
        raise ExportError("Invalid Maps credential JSON.") from None
    if not isinstance(value, dict) or (memory and value.get("version") != 2):
        raise ExportError("Unsupported Maps credential format.")
    provider, key = value.get("provider"), value.get("api_key")
    if provider not in ("mapbox", "google"):
        raise ExportError("Unsupported Maps provider.")
    if not isinstance(key, str) or not key.strip():
        raise ExportError("Stored Maps key is empty or invalid.")
    key = key.strip()
    if any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in key):
        raise ExportError("Stored Maps key contains control characters.")
    return provider, key


def decrypt_credential(master, row, expected_version):
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    version, nonce, ciphertext = row
    if version != expected_version:
        raise ExportError("Key version mismatch; set OPEN_WEB_CODEX_MASTER_KEY_VERSION to the server value.")
    try:
        raw_key = base64.b64decode(master, validate=True)
    except (ValueError, UnicodeError):
        raise ExportError("Master key must be base64 encoding of 32 bytes.") from None
    if len(raw_key) != 32:
        raise ExportError("Master key must be base64 encoding of 32 bytes.")
    if len(nonce) != 12:
        raise ExportError("Invalid stored nonce.")
    try:
        plaintext = AESGCM(raw_key).decrypt(bytes(nonce), bytes(ciphertext), AAD)
    except Exception:
        raise ExportError("Decryption failed; check the database and the server's master key.") from None
    return parse_credential(plaintext)


def missing_dependencies():
    return any(importlib.util.find_spec(name) is None for name in ("psycopg", "cryptography"))


def temporary_environment():
    """Install outside the repository; remove dependencies after child exit."""
    import venv
    try:
        with tempfile.TemporaryDirectory(prefix="owc-maps-export-") as directory:
            root = Path(directory)
            print("Installing database dependencies in a temporary environment…", file=sys.stderr)
            venv.EnvBuilder(with_pip=True).create(root)
            python = root / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
            installed = subprocess.run(
                [str(python), "-m", "pip", "install", "--disable-pip-version-check",
                 "--no-input", "--no-cache-dir", "--timeout", "15", "--retries", "1", *DEPENDENCIES],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False, timeout=120,
            )
            if installed.returncode:
                raise ExportError("Dependency installation failed. Install psycopg[binary] and cryptography in an accessible Python environment, then retry.")
            return subprocess.run(
                [str(python), str(Path(__file__).resolve()), *sys.argv[1:], "--no-install"],
                check=False,
            ).returncode
    except subprocess.TimeoutExpired:
        raise ExportError("Dependency installation timed out; preinstall dependencies and retry with --no-install.") from None
    except (OSError, subprocess.SubprocessError):
        raise ExportError("Cannot create the temporary Python environment; check Python venv support and network access.") from None


def export_database(args):
    import psycopg
    database_url = setting("DATABASE_URL", args.database_url_file, args.data_dir / "database-url")
    master = setting("OPEN_WEB_CODEX_MASTER_KEY", args.master_key_file, args.data_dir / "master-key")
    try:
        with psycopg.connect(database_url, connect_timeout=10,
                             autocommit=True, application_name="owc-maps-key-export") as connection:
            # Session-only limits. No migration, service restart, or credential write.
            connection.execute("SET default_transaction_read_only = on")
            connection.execute("SET statement_timeout = '10s'")
            row = connection.execute(
                "SELECT key_version, nonce, ciphertext FROM platform_configuration_secrets "
                "WHERE scope_kind='global' AND scope_id='global' AND config_key=%s",
                (CONFIG_KEY,),
            ).fetchone()
    except Exception:
        raise ExportError("Database read failed; check connection, TLS, permissions and current schema. Details suppressed to protect credentials.") from None
    if row is None:
        raise ExportError("No active Maps credential in platform Secret Store.")
    return decrypt_credential(master, row, os.environ.get("OPEN_WEB_CODEX_MASTER_KEY_VERSION", "v1"))


def main():
    args = arguments()
    if args.credential_file is not None:
        if args.database_url_file is not None or args.master_key_file is not None:
            raise ExportError("Use either --credential-file or database configuration options.")
        provider, key = parse_credential(read_file(args.credential_file), memory=True)
    else:
        if missing_dependencies():
            if args.no_install:
                raise ExportError("Missing dependencies: psycopg[binary], cryptography.")
            return temporary_environment()
        provider, key = export_database(args)
    print("Maps provider: " + provider, file=sys.stderr)
    print(key)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ExportError as error:
        print("error: " + str(error), file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
