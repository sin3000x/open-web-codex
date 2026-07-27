"""Persistent GeoJSON resources addressed by MCP Resource URIs."""

from __future__ import annotations

import json
import os
import re
import tempfile
import uuid
from dataclasses import dataclass
from pathlib import Path

_RESOURCE_ID = re.compile(r"^[A-Za-z0-9_.-]{1,128}$")


@dataclass(frozen=True)
class PublishedGeoJson:
    resource_id: str
    uri: str
    size: int


class GeoJsonResourceStore:
    def __init__(self, workspace_root: Path) -> None:
        self.root = workspace_root.resolve() / ".codex" / "maps-data"

    def publish(self, geojson: dict[str, object]) -> PublishedGeoJson:
        payload = json.dumps(
            geojson,
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
        resource_id = f"map-data-{uuid.uuid4().hex}"
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(self.root, 0o700)
        with tempfile.NamedTemporaryFile(dir=self.root, delete=False) as temporary:
            temporary.write(payload)
            temporary.flush()
            os.fsync(temporary.fileno())
            temporary_path = Path(temporary.name)
        target = self.root / f"{resource_id}.geojson"
        os.chmod(temporary_path, 0o600)
        temporary_path.replace(target)
        return PublishedGeoJson(
            resource_id=resource_id,
            uri=f"maps-data://geojson/{resource_id}",
            size=len(payload),
        )

    def read(self, resource_id: str) -> str:
        if not _RESOURCE_ID.fullmatch(resource_id):
            raise ValueError("GeoJSON Resource identifier is invalid")
        path = self.root / f"{resource_id}.geojson"
        return path.read_text(encoding="utf-8")
