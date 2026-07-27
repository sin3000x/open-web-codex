"""Content-addressed storage for MCP planning resources."""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from .models import DataRef, MCP_SERVER_NAME

RESOURCE_ID_PATTERN = re.compile(r"^[a-z0-9_.-]{1,160}$")
RESOURCE_URI_PREFIX = "supply-chain://resources/"


@dataclass(frozen=True)
class PublishedResource:
    resource_id: str
    uri: str
    schema: str
    size: int


class ResourceStore:
    def __init__(self, root: Path, *, uri_prefix: str = RESOURCE_URI_PREFIX):
        self.root = root.resolve()
        self.uri_prefix = uri_prefix
        self.root.mkdir(parents=True, exist_ok=True)

    def publish(self, schema: str, value: BaseModel | dict[str, Any]) -> PublishedResource:
        payload = value.model_dump(mode="json") if isinstance(value, BaseModel) else value
        encoded = json.dumps(
            payload,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        digest = hashlib.sha256(encoded).hexdigest()[:24]
        safe_schema = re.sub(r"[^a-z0-9_.-]", "-", schema.lower())
        resource_id = f"{safe_schema}-{digest}"
        path = self._path(resource_id)
        if not path.exists():
            path.write_bytes(encoded)
        return PublishedResource(
            resource_id=resource_id,
            uri=f"{self.uri_prefix}{resource_id}",
            schema=schema,
            size=len(encoded),
        )

    def read(self, resource_id: str) -> str:
        return self._path(resource_id).read_text(encoding="utf-8")

    def load(self, data_ref: DataRef | str) -> dict[str, Any]:
        ref = parse_data_ref(data_ref)
        return self.load_uri(ref.uri)

    def load_uri(self, uri: str) -> dict[str, Any]:
        if not uri.startswith(self.uri_prefix):
            raise ValueError(f"resource URI must start with {self.uri_prefix!r}")
        resource_id = uri.removeprefix(self.uri_prefix)
        payload = json.loads(self.read(resource_id))
        if not isinstance(payload, dict):
            raise ValueError(f"resource {resource_id!r} does not contain a JSON object")
        return payload

    def _path(self, resource_id: str) -> Path:
        if not RESOURCE_ID_PATTERN.fullmatch(resource_id):
            raise ValueError("invalid supply-chain resource identifier")
        path = (self.root / f"{resource_id}.json").resolve()
        if path.parent != self.root:
            raise ValueError("resource path escapes the configured resource directory")
        return path


def parse_data_ref(value: DataRef | str) -> DataRef:
    if isinstance(value, DataRef):
        return value
    if value.startswith(RESOURCE_URI_PREFIX):
        resource_id = value.removeprefix(RESOURCE_URI_PREFIX)
        if not RESOURCE_ID_PATTERN.fullmatch(resource_id):
            raise ValueError("invalid supply-chain resource URI")
        schema = resource_id.rsplit("-", maxsplit=1)[0]
        return DataRef(
            server=MCP_SERVER_NAME,
            uri=value,
            resource_schema=schema,
        )
    raise ValueError(
        "expected a supply-chain MCP resource URI or the data_ref returned by an earlier tool"
    )


def data_ref(published: PublishedResource) -> DataRef:
    return DataRef(
        server=MCP_SERVER_NAME,
        uri=published.uri,
        resource_schema=published.schema,
    )
