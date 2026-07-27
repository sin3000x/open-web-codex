"""Read-only supply-chain data MCP for the Data Agent."""

from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path
from typing import Annotated

from mcp.server.fastmcp import FastMCP
from mcp.types import CallToolResult, ResourceLink, TextContent

from .data_core import build_planning_dataset as aggregate_planning_dataset
from .data_core import inspect_source
from .models import (
    DataAgentRef,
    DataAgentResourceToolResult,
    PlanningDataset,
    PlanningSource,
    PlanningSourceInspection,
    ValidationResult,
)
from .resource_store import PublishedResource, ResourceStore

MCP_SERVER_NAME = "supply_chain_data"
RESOURCE_URI_PREFIX = "supply-chain-data://resources/"
MAX_SOURCE_BYTES = 20 * 1024 * 1024
SOURCE_ID_PATTERN = re.compile(r"^[a-z0-9][a-z0-9_-]{0,127}$")

mcp = FastMCP(
    "Supply Chain Data",
    instructions=(
        "This is a read-only enterprise data boundary for the supply-chain Data Agent. "
        "Inputs are bounded source IDs resolved under a deployment-configured data root; "
        "never request or accept organization IDs, Profile IDs, credentials, arbitrary SQL, "
        "filesystem paths, or write statements. Call inspect_planning_source before "
        "build_planning_dataset. The build tool publishes planning-dataset.v1 as an immutable "
        "MCP Resource with source range, units, row counts, promotion share, delivery baseline, "
        "and data-quality limitations. Copy data_ref unchanged. Validate the Resource before "
        "handing it to the Network Planning Agent. Do not paste unbounded source rows into "
        "messages and do not choose a warehouse-network solution."
    ),
    json_response=True,
)

_workspace_root = Path.cwd().resolve()
_data_root = Path(
    os.environ.get(
        "SUPPLY_CHAIN_READONLY_DATA_ROOT",
        _workspace_root / "examples" / "data-sources",
    )
).resolve()
_profile_state_root = Path(
    os.environ.get("CODEX_HOME", _workspace_root / ".codex")
).resolve()
_resource_store: ResourceStore | None = None


def _store() -> ResourceStore:
    global _resource_store
    if _resource_store is None:
        resource_root = Path(
            os.environ.get(
                "SUPPLY_CHAIN_DATA_RESOURCE_DIR",
                _profile_state_root
                / "mcp-state"
                / "supply-chain-data"
                / "resources",
            )
        ).resolve()
        _resource_store = ResourceStore(
            resource_root,
            uri_prefix=RESOURCE_URI_PREFIX,
        )
    return _resource_store


@mcp.resource(
    "supply-chain-data://resources/{resource_id}",
    name="supply_chain_planning_dataset",
    title="Supply-chain planning dataset",
    mime_type="application/json",
)
def read_planning_dataset_resource(resource_id: str) -> str:
    """Read a planning dataset previously published by the read-only data MCP."""
    return _store().read(resource_id)


def _load_source(source_id: str) -> PlanningSource:
    if not SOURCE_ID_PATTERN.fullmatch(source_id):
        raise ValueError("source_id must contain only lowercase letters, digits, _ or -")
    path = (_data_root / f"{source_id}.json").resolve()
    if path.parent != _data_root:
        raise ValueError("source_id escapes the configured read-only data root")
    size = path.stat().st_size
    if size > MAX_SOURCE_BYTES:
        raise ValueError(
            f"source file is {size} bytes; maximum supported size is {MAX_SOURCE_BYTES}"
        )
    payload = json.loads(path.read_text(encoding="utf-8"))
    source = PlanningSource.model_validate(payload)
    if source.source_id != source_id:
        raise ValueError(
            f"requested source_id {source_id!r} does not match payload "
            f"{source.source_id!r}"
        )
    return source


def _data_ref(published: PublishedResource) -> DataAgentRef:
    return DataAgentRef(
        server=MCP_SERVER_NAME,
        uri=published.uri,
        resource_schema=published.schema,
    )


def _resource_link(
    published: PublishedResource,
    description: str,
) -> ResourceLink:
    return ResourceLink(
        type="resource_link",
        name=published.resource_id,
        title=published.schema,
        uri=published.uri,
        description=description,
        mimeType="application/json",
        size=published.size,
    )


@mcp.tool(structured_output=True)
def inspect_planning_source(source_id: str) -> PlanningSourceInspection:
    """Inspect bounded metadata and quality signals without exposing source rows."""
    return inspect_source(_load_source(source_id))


@mcp.tool(structured_output=True)
def build_planning_dataset(
    source_id: str,
) -> Annotated[CallToolResult, DataAgentResourceToolResult]:
    """Aggregate an authorized source into an immutable planning-dataset.v1 Resource."""
    dataset = build_planning_dataset_from_source_id(source_id)
    published = _store().publish(dataset.schema_version, dataset)
    quality = "valid" if dataset.data_quality.valid else "has blocking data gaps"
    summary = (
        f"Built {dataset.dataset_id} from {dataset.source_summary.order_row_count} "
        f"source rows and {dataset.source_summary.demand_units} demand units across "
        f"{dataset.source_summary.demand_node_count} demand nodes; data quality {quality}, "
        f"{len(dataset.data_quality.warnings)} warnings."
    )
    structured = DataAgentResourceToolResult(
        summary=summary,
        data_ref=_data_ref(published),
    ).model_dump(mode="json")
    return CallToolResult(
        content=[
            TextContent(type="text", text=summary),
            _resource_link(published, summary),
        ],
        structuredContent=structured,
    )


def build_planning_dataset_from_source_id(source_id: str) -> PlanningDataset:
    """Build without publishing; kept separate for deterministic tests."""
    return aggregate_planning_dataset(_load_source(source_id))


@mcp.tool(structured_output=True)
def validate_planning_dataset(
    resource_ref: DataAgentRef,
) -> ValidationResult:
    """Validate planning-dataset structure, totals, handoff projection, and quality state."""
    if resource_ref.server != MCP_SERVER_NAME:
        raise ValueError(f"data_ref.server must be {MCP_SERVER_NAME}")
    if resource_ref.resource_schema != "planning-dataset.v1":
        raise ValueError("resource_ref must identify planning-dataset.v1")
    dataset = PlanningDataset.model_validate(_store().load_uri(resource_ref.uri))
    errors: list[str] = []
    warnings = list(dataset.data_quality.warnings)
    checks: list[str] = ["resource conforms to planning-dataset.v1"]

    distribution_units = sum(
        item.demand_units for item in dataset.demand_distribution
    )
    projected_units = sum(
        item.demand_units for item in dataset.network_input.demand_points
    )
    if distribution_units != dataset.source_summary.demand_units:
        errors.append("distribution demand units do not match source summary")
    if projected_units != dataset.source_summary.demand_units:
        errors.append("network_input demand units do not match source summary")
    if (
        dataset.delivery_baseline.observed_demand_units
        + dataset.delivery_baseline.unobserved_demand_units
        != dataset.source_summary.demand_units
    ):
        errors.append("delivery baseline units do not match source summary")
    distribution_ids = {item.demand_id for item in dataset.demand_distribution}
    projection_ids = {item.demand_id for item in dataset.network_input.demand_points}
    if distribution_ids != projection_ids:
        errors.append("distribution and network_input demand identifiers differ")
    if dataset.data_quality.errors:
        errors.extend(dataset.data_quality.errors)
    if not errors:
        checks.extend(
            [
                "demand totals reconcile across source, distribution, and network projection",
                "delivery observed and unobserved units reconcile to total demand",
                "demand identifiers match the network handoff projection",
            ]
        )
    return ValidationResult(
        valid=not errors,
        resource_schema=dataset.schema_version,
        errors=errors,
        warnings=warnings,
        checks=checks,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Read-only supply-chain data MCP server")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path.cwd(),
        help="Plugin root used for default fixture and Profile Resource locations",
    )
    parser.add_argument(
        "--transport",
        choices=("stdio", "streamable-http"),
        default="stdio",
    )
    args = parser.parse_args()

    global _workspace_root, _data_root, _profile_state_root, _resource_store
    _workspace_root = args.workspace_root.resolve()
    _data_root = Path(
        os.environ.get(
            "SUPPLY_CHAIN_READONLY_DATA_ROOT",
            _workspace_root / "examples" / "data-sources",
        )
    ).resolve()
    _profile_state_root = Path(
        os.environ.get("CODEX_HOME", _workspace_root / ".codex")
    ).resolve()
    resource_root = Path(
        os.environ.get(
            "SUPPLY_CHAIN_DATA_RESOURCE_DIR",
            _profile_state_root
            / "mcp-state"
            / "supply-chain-data"
            / "resources",
        )
    ).resolve()
    _resource_store = ResourceStore(
        resource_root,
        uri_prefix=RESOURCE_URI_PREFIX,
    )
    mcp.run(transport=args.transport)


if __name__ == "__main__":
    main()
