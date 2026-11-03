"""Typed supply-chain inspection and preparation MCP for the Data Agent."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Annotated, Any

from mcp.server.fastmcp import Context, FastMCP
from mcp.server.stdio import stdio_server
from mcp.types import (
    CallToolResult,
    TextContent,
    ToolAnnotations,
)
from open_web_codex_provider import (
    MAX_WORKSPACE_FILE_BYTES,
    McpResourceRuntime,
    ProviderContractError,
)
from pydantic import Field
from supply_chain_planner.data import workspace_intake as _workspace_intake
from supply_chain_planner.data.geography import enrich_network_geography
from supply_chain_planner.data.geography import (
    load_administrative_catalog as _load_administrative_catalog,
)
from supply_chain_planner.data.mapping import (
    REQUIRED_FIELDS,
    FieldObservation,
    PlanningSourceRole,
    SourceRole,
    SuggestedRoleAssessment,
    TransformSpec,
    assess_role_mappings,
)
from supply_chain_planner.data.normalization import (
    ConfirmedFieldMapping,
    ConfirmedSourceRows,
    normalize_confirmed_rows,
)
from supply_chain_planner.data.prepared_query import (
    MAX_QUERY_ROWS,
    field_catalog_description,
    query_prepared_input,
)
from supply_chain_planner.data.prepared_resolution import (
    MAX_PREPARED_CANDIDATES,
    resolve_prepared_inputs,
)
from supply_chain_planner.data.workspace_intake import (
    discover,
    inspect,
    read_json_document_with_sha256,
    read_unit_rows,
    source_inspection_identity,
    source_inspection_snapshot,
    workspace_source_metadata,
)
from supply_chain_planner.network.models import (
    DataQualityIssue,
    NormalizedInputBatch,
)
from supply_chain_planner.shared.models import (
    CandidateWarehouseSummary,
    ConfirmedFieldDecision,
    DataInspectionInspected,
    DataInspectionSelectionRequired,
    DataInspectionToolResult,
    DataPreparationNeedsInput,
    DataPreparationReady,
    DataPreparationSourceChanged,
    DataPreparationToolResult,
    DataSourceRequirement,
    GeographyOverride,
    InspectionLimitCounts,
    PreparationRoleCounts,
    PreparedAdministrativeCatalog,
    PreparedNetworkResource,
    PreparedQueryDataset,
    PreparedQueryFilter,
    PreparedQueryMeasure,
    PreparedQueryOrder,
    PreparedQueryStale,
    PreparedQueryToolResult,
    PreparedResolutionToolResult,
    PreparedSourceSelection,
    SourceInspectionIdentity,
    SourceSelection,
)
from supply_chain_planner.shared.planning_input import (
    derive_selected_source_identity,
    load_prepared_network_input,
    validate_prepared_freshness,
)
from supply_chain_planner.shared.resources import SupplyChainResources
from supply_chain_planner.shared.workspace_outputs import (
    WorkspaceOutputKind,
    prepare_workspace_output_path,
)

SANDBOX_STATE_META_CAPABILITY = "codex/sandbox-state-meta"

READ_ONLY_LOCAL_TOOL = ToolAnnotations(
    readOnlyHint=True,
    destructiveHint=False,
    idempotentHint=True,
    openWorldHint=False,
)
WORKSPACE_PREPARATION_TOOL = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=False,
    idempotentHint=False,
    openWorldHint=False,
)


mcp = FastMCP(
    "Supply Chain Data",
    instructions=(
        "This is a typed enterprise data boundary for the supply-chain Data Agent. Inspection "
        "tools are read-only; preparation tools may only create new files under the declared "
        "warehouse-network output directory. "
        "Inputs are validated Workspace-relative paths resolved under the trusted Turn Workspace; "
        "never request or accept organization IDs, Profile IDs, credentials, arbitrary SQL, "
        "filesystem paths, or write statements. Discover and inspect the complete authorized "
        "Workspace before confirming mappings. Inspection returns one bounded inline profile and "
        "workspace_source_inspection.v2 identity; Data does not publish a source Resource. "
        "Inspection accepts raw sources only; reuse prepared inputs through "
        "resolve_prepared_network_input. Each unit reports its exact record_count and, per "
        "field, bounded statistics over every record: value types, null and distinct counts, "
        "numeric min/max, value counts for low-cardinality fields and a few representative "
        "values. No raw rows are returned. "
        "Inspection reports independent source-unit role assessments; partial or ambiguous units "
        "do not block inspection. Only the selected sources are validated during preparation. "
        "The initial normalization tool rereads "
        "the complete explicitly selected source units and preserves every selected candidate "
        "warehouse in a user-visible prepared_network_input.v2 Workspace JSON file. Source facts "
        "such as demand, existing warehouses, assignments, routes, costs, or candidates always "
        "require a complete new preparation. Return the exact Workspace-relative output path and "
        "its content identity to the Network Planning Agent. Do not paste unbounded source rows "
        "into messages and do not choose a warehouse-network solution."
    ),
    json_response=True,
)

_workspace_root = Path.cwd().resolve()
_profile_state_root = Path(os.environ.get("CODEX_HOME", _workspace_root / ".codex")).resolve()
_supply_chain_resources: SupplyChainResources | None = None


def _runtime() -> McpResourceRuntime:
    global _supply_chain_resources
    if _supply_chain_resources is None:
        _supply_chain_resources = SupplyChainResources(_workspace_root, _profile_state_root)
    return _supply_chain_resources.data


def _workspace(ctx: Context) -> Path:
    return _runtime().require_workspace(ctx)


def _write_prepared_input(
    prepared: PreparedNetworkResource,
    output_relative_path: str,
    ctx: Context,
    summary: str,
) -> DataPreparationToolResult:
    output_relative_path = prepare_workspace_output_path(
        _workspace(ctx),
        output_relative_path,
        WorkspaceOutputKind.PREPARED_INPUT,
    )
    created = _runtime().create_workspace_model(
        ctx,
        output_relative_path,
        prepared,
        max_bytes=MAX_WORKSPACE_FILE_BYTES,
    )
    persisted, identity = load_prepared_network_input(
        _workspace(ctx),
        created.relative_path,
    )
    candidates = sorted(
        (warehouse for warehouse in persisted.warehouses if not warehouse.is_existing),
        key=lambda warehouse: warehouse.warehouse_id,
    )
    warnings = [
        issue.business_message
        for issue in persisted.issues
        if issue.severity == "warning"
    ]
    warning_count = persisted.issue_count
    result = DataPreparationReady(
        outcome="ready",
        operation="created",
        summary=summary,
        next_action=("prepare_geography" if persisted.state == "needs_geography" else "handoff"),
        retryable=False,
        prepared_input_relative_path=created.relative_path,
        input_identity=identity,
        state=persisted.state,
        role_counts=PreparationRoleCounts(
            demand=len(persisted.demand_cities),
            existing_warehouse=sum(item.is_existing for item in persisted.warehouses),
            candidate_warehouse=sum(not item.is_existing for item in persisted.warehouses),
            current_assignment=len(persisted.current_assignments),
            route_quote=len(persisted.route_quotes),
            provided_route_fact=len(persisted.provided_route_facts),
        ),
        warnings=warnings[:64],
        warning_count=warning_count,
        warnings_truncated=persisted.issues_truncated,
        candidate_warehouse_count=len(candidates),
        candidate_warehouses=[
            CandidateWarehouseSummary(
                warehouse_id=warehouse.warehouse_id,
                warehouse_name=warehouse.warehouse_name,
                city_name=warehouse.city_name,
            )
            for warehouse in candidates[:64]
        ],
        candidate_warehouses_truncated=len(candidates) > 64,
    )
    return DataPreparationToolResult(root=result)


def _needs_input_result(
    requirements: list[DataSourceRequirement],
) -> DataPreparationToolResult:
    complete = _dedupe_requirements(requirements)
    if not complete:
        raise ProviderContractError("preparation_blocker_untyped")
    bounded = complete[:64]
    summary = _requirements_summary(complete)
    return DataPreparationToolResult(
        root=DataPreparationNeedsInput(
            outcome="needs_input",
            summary=summary,
            next_action="request_user_input",
            retryable=False,
            requirements=bounded,
            requirement_count=len(complete),
            requirements_truncated=len(complete) > 64,
        )
    )


def _requirements_summary(requirements: list[DataSourceRequirement]) -> str:
    preview = " ".join(item.question for item in requirements[:3])
    suffix = "结构化结果列出前64项。" if len(requirements) > 64 else ""
    if len(requirements) > 3:
        return f"发现 {len(requirements)} 项选中来源问题。{preview} {suffix}".strip()
    return f"发现 {len(requirements)} 项选中来源问题。{preview} {suffix}".strip()


def _source_changed_result() -> DataPreparationToolResult:
    return DataPreparationToolResult(
        root=DataPreparationSourceChanged(
            outcome="source_changed",
            summary="The inspected Workspace sources changed; inspect them again before preparing.",
            next_action="reinspect",
            retryable=False,
        )
    )


def _is_source_lifecycle_error(error: ValueError) -> bool:
    code = str(error).split(":", 1)[0]
    return code in {
        "workspace_source_not_found",
        "workspace_source_symlink_rejected",
        "workspace_source_escape_rejected",
        "workspace_source_not_regular_file",
        "workspace_source_size_limit",
        "unsupported_source_format",
    }


@mcp.tool(structured_output=True, annotations=READ_ONLY_LOCAL_TOOL)
def discover_workspace_sources(ctx: Context) -> dict[str, Any]:
    """Discover bounded Excel/CSV/JSON metadata across the trusted Workspace."""
    root = _workspace(ctx)
    sources = discover(root)
    return {
        "schema": "workspace_source_catalog.v1",
        "workspace_scope": "authorized_workspace",
        "sources": sources,
        "truncated": False,
        **workspace_source_metadata(root),
    }


@mcp.tool(structured_output=True, annotations=READ_ONLY_LOCAL_TOOL)
def inspect_workspace_sources(
    relative_paths: list[str],
    ctx: Context,
    country_code: Annotated[
        str | None,
        Field(
            pattern=r"^[A-Za-z]{2}$",
            description=(
                "Only a country the user stated. Omit it otherwise; a unique administrative "
                "catalog among the inspected files may supply it."
            ),
        ),
    ] = None,
) -> Annotated[CallToolResult, DataInspectionToolResult]:
    """Inspect selected raw Workspace sources and return one bounded inline profile.

    Prepared inputs are not raw sources; reuse them with
    resolve_prepared_network_input. Preview rows are examples for schema
    inspection only. They are never a complete source snapshot and must not be
    used as the source row count.
    """
    country = country_code.strip().upper() if country_code is not None else None
    root = _workspace(ctx)
    if not relative_paths:
        raise ValueError("relative_paths must contain at least one Workspace-relative path")
    if len(set(relative_paths)) != len(relative_paths):
        raise ValueError("relative_paths must not contain duplicates")
    if any(_is_prepared_candidate_path(path) for path in relative_paths):
        raise ProviderContractError("prepared_input_not_raw_source")
    inspected = _inspect_workspace_sources(relative_paths, ctx)
    if isinstance(inspected, DataInspectionSelectionRequired):
        summary = inspected.summary
        result = DataInspectionToolResult(root=inspected)
        return CallToolResult(
            content=[TextContent(type="text", text=summary)],
            structuredContent=result.model_dump(mode="json", by_alias=True),
        )
    profile = inspected
    derived_country, country_warnings = _derive_inspected_country(profile, country)
    content_sha256, source_count = source_inspection_identity(root, relative_paths)
    identity = SourceInspectionIdentity(
        content_sha256=content_sha256,
        source_count=source_count,
    )
    summary = (
        f"Inspected {source_count} authorized Workspace sources into "
        f"{sum(len(source.get('units', [])) for source in profile.get('sources', []))} "
        "source units. Select the units required for the planning goal; partial or "
        "ambiguous role assessments are not global blockers."
    )
    result = DataInspectionToolResult(
        root=DataInspectionInspected(
            outcome="inspected",
            schemaVersion="workspace_source_profile.v3",
            summary=summary,
            next_action="confirm_sources",
            retryable=False,
            country_code=derived_country,
            source_profile=profile,
            inspection_identity=identity,
            inspected_relative_paths=sorted(relative_paths),
            warnings=country_warnings[:64],
            warning_count=len(country_warnings),
            warnings_truncated=len(country_warnings) > 64,
        )
    )
    return CallToolResult(
        content=[TextContent(type="text", text=summary)],
        structuredContent=result.model_dump(mode="json", by_alias=True),
    )


def _derive_inspected_country(
    profile: dict[str, Any], explicit_country: str | None
) -> tuple[str | None, list[str]]:
    if explicit_country is not None:
        return explicit_country, []
    codes: list[str] = []
    metadata_seen = False
    invalid_metadata = False
    for source in profile.get("sources", []):
        units = source.get("units", [])
        if not any(
            assessment.get("role") == SourceRole.ADMINISTRATIVE_CATALOG.value
            for unit in units
            if isinstance(unit, dict)
            for assessment in unit.get("role_assessments", [])
            if isinstance(assessment, dict)
        ):
            continue
        metadata = source.get("administrative_metadata")
        if not isinstance(metadata, dict):
            continue
        metadata_seen = True
        value = metadata.get("country_code")
        if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z]{2}", value.strip()):
            invalid_metadata = True
            continue
        codes.append(value.strip().upper())
    if invalid_metadata:
        return None, ["country_code_metadata_invalid"]
    unique_codes = sorted(set(codes))
    if len(unique_codes) == 1:
        return unique_codes[0], []
    if len(unique_codes) > 1:
        return None, ["country_code_metadata_conflict"]
    return None, ["country_code_metadata_missing" if metadata_seen else "country_code_not_found"]


def _canonical_required_roles(
    required_roles: list[PlanningSourceRole],
) -> list[PlanningSourceRole]:
    if len(required_roles) != len(set(required_roles)):
        raise ValueError("required_roles_must_be_unique")
    return sorted(required_roles, key=lambda role: role.value)


def _is_prepared_candidate_path(relative_path: str) -> bool:
    candidate = PurePosixPath(relative_path)
    return (
        candidate.parent == PurePosixPath("outputs/warehouse-network/prepared")
        and candidate.suffix.lower() == ".json"
    )


@mcp.tool(structured_output=True, annotations=READ_ONLY_LOCAL_TOOL)
def resolve_prepared_network_input(
    prepared_relative_paths: Annotated[
        list[str],
        Field(
            min_length=1,
            max_length=MAX_PREPARED_CANDIDATES,
            description="Exact prepared_candidate paths from discover_workspace_sources.",
        ),
    ],
    required_roles: Annotated[list[PlanningSourceRole], Field(min_length=1, max_length=5)],
    ctx: Context,
    country_code: Annotated[
        str | None,
        Field(
            pattern=r"^[A-Za-z]{2}$",
            description=(
                "Only a country the user stated. Omit it otherwise: each prepared file is "
                "checked against the country it records."
            ),
        ),
    ] = None,
) -> Annotated[CallToolResult, PreparedResolutionToolResult]:
    """Reuse one fresh prepared input for the requested roles.

    Files whose records for the requested roles are identical are interchangeable
    and resolve to one prepared_ready result; only differing records require a
    user choice. prepared_unavailable means inspect the raw sources and prepare.
    """
    outcome = resolve_prepared_inputs(
        _workspace(ctx),
        prepared_relative_paths,
        _canonical_required_roles(required_roles),
        country_code.strip().upper() if country_code is not None else None,
    )
    result = PreparedResolutionToolResult(root=outcome)
    return CallToolResult(
        content=[TextContent(type="text", text=outcome.summary)],
        structuredContent=result.model_dump(mode="json", by_alias=True),
    )


def _inspect_workspace_sources(
    relative_paths: list[str],
    ctx: Context,
) -> dict[str, Any] | DataInspectionSelectionRequired:
    if not relative_paths:
        raise ValueError("relative_paths must contain at least one Workspace-relative path")
    if len(relative_paths) > _workspace_intake.MAX_INSPECTION_FILES:
        return _selection_required_result(
            summary="Select fewer Workspace source files before inspecting them.",
            observed=InspectionLimitCounts(
                files=min(len(relative_paths), 500), units=0, bytes=0
            ),
        )
    if len(set(relative_paths)) != len(relative_paths):
        raise ValueError("relative_paths must not contain duplicates")
    sources: list[dict[str, Any]] = []
    unit_count = 0
    for relative_path in relative_paths:
        source = inspect(_workspace(ctx), relative_path)
        structure = source.pop("structure")
        metadata = structure.get("administrative_metadata")
        if isinstance(metadata, dict) and metadata:
            source["administrative_metadata"] = metadata
        units = _mapping_analysis(str(source["relative_path"]), structure)
        source["units"] = units
        unit_count += len(units)
        if unit_count > _workspace_intake.MAX_INSPECTION_UNITS:
            return _selection_required_result(
                summary="Select files with fewer source units before inspecting them.",
                observed=InspectionLimitCounts(
                    files=len(sources) + 1, units=unit_count, bytes=0
                ),
            )
        sources.append(source)
    profile = {
        "schemaVersion": "workspace_source_profile.v3",
        "sources": sources,
        "source_count": len(sources),
        "unit_count": unit_count,
    }
    profile_bytes = len(json.dumps(profile, ensure_ascii=False, separators=(",", ":")).encode())
    if profile_bytes > _workspace_intake.MAX_INSPECTION_BYTES:
        return _selection_required_result(
            summary="Select fewer Workspace sources; the bounded inspection profile is too large.",
            observed=InspectionLimitCounts(
                files=len(sources), units=unit_count, bytes=profile_bytes
            ),
        )
    return profile


def _selection_required_result(
    *, summary: str, observed: InspectionLimitCounts
) -> DataInspectionSelectionRequired:
    return DataInspectionSelectionRequired(
        outcome="selection_required",
        schemaVersion="workspace_source_profile.v3",
        summary=summary,
        code="inspection_selection_required",
        next_action="select_fewer_sources",
        retryable=False,
        observed=observed,
        limit=InspectionLimitCounts(
            files=_workspace_intake.MAX_INSPECTION_FILES,
            units=_workspace_intake.MAX_INSPECTION_UNITS,
            bytes=_workspace_intake.MAX_INSPECTION_BYTES,
        ),
    )


def _unit_definitions(structure: dict[str, Any]) -> list[dict[str, Any]]:
    kind = structure.get("kind")
    if kind == "table":
        return [{"unit_ref": "table", **structure}]
    if kind == "workbook":
        return [
            {"unit_ref": sheet.get("unit_ref", f"sheet:{sheet.get('sheet', '')}"), **sheet}
            for sheet in structure.get("sheets", [])
        ]
    if kind == "json":
        return [
            {"unit_ref": array.get("unit_ref", array.get("path", "$")), **array}
            for array in structure.get("arrays", [])
        ]
    return []


def _unit_observations(unit: dict[str, Any]) -> list[FieldObservation]:
    return [
        FieldObservation(
            name=str(field["name"]).strip(),
            sample_values=tuple(str(value)[:256] for value in _field_sample_values(field)[:3]),
        )
        for field in unit.get("fields", [])
        if isinstance(field, dict) and str(field.get("name", "")).strip()
    ]


def _field_sample_values(field: dict[str, Any]) -> list[Any]:
    if "value_counts" in field:
        return [item["value"] for item in field["value_counts"]]
    return list(field.get("representative_values", []))


def _role_assessment_payload(assessment: SuggestedRoleAssessment) -> dict[str, Any]:
    return {
        "role": assessment.role.value,
        "state": assessment.state,
        "confidence": assessment.confidence,
        "ambiguous": assessment.ambiguous,
        "matched_required_fields": list(assessment.matched_required_fields),
        "missing_required_fields": list(assessment.missing_required_fields),
        "field_mappings": [
            {
                "target_field": mapping.target_field,
                "source_fields": list(mapping.source_fields),
                "transform": mapping.transform.model_dump(mode="json"),
                "score": mapping.score,
                "reason_code": mapping.reason_code,
            }
            for mapping in assessment.field_mappings
        ],
    }


def _administrative_assessment(structure: dict[str, Any]) -> dict[str, Any] | None:
    if structure.get("kind") != "json":
        return None
    keys = {
        key
        for values in structure.get("object_keys", {}).values()
        for key in values
    }
    if "admin_level" not in keys:
        return None
    return {
        "role": SourceRole.ADMINISTRATIVE_CATALOG.value,
        "state": "complete",
        "confidence": 1.0,
        "ambiguous": False,
        "matched_required_fields": [],
        "missing_required_fields": [],
        "field_mappings": [],
    }


def _mapping_analysis(
    relative_path: str,
    structure: dict[str, Any],
) -> list[dict[str, Any]]:
    del relative_path
    units: list[dict[str, Any]] = []
    unit_definitions = _unit_definitions(structure)
    administrative = _administrative_assessment(structure)
    for index, unit in enumerate(unit_definitions):
        observations = _unit_observations(unit)
        assessments = [
            _role_assessment_payload(item) for item in assess_role_mappings(observations)
        ]
        if administrative is not None and index == 0:
            assessments.append(administrative)
        locator = {
            key: unit[key]
            for key in ("sheet", "path", "array_prefix")
            if unit.get(key) is not None
        }
        unit_kind = {
            "table": "table",
            "workbook": "sheet",
            "json": "json_array",
        }.get(structure.get("kind"), structure.get("kind"))
        unit_profile = {
            "unit_ref": str(unit.get("unit_ref", "table")),
            "kind": unit_kind,
            "locator": locator,
            "fields": unit.get("fields", []),
            "record_count": unit.get("record_count", 0),
            "record_count_exact": unit.get("record_count_exact", True),
            "role_assessments": assessments,
        }
        for count_key in ("formula_cell_count", "non_object_item_count"):
            if unit.get(count_key):
                unit_profile[count_key] = unit[count_key]
        units.append(unit_profile)
    return units


def _missing_fields_question(relative_path: str, missing_fields: tuple[str, ...]) -> str:
    if missing_fields == ("warehouse_type",):
        return (
            f"文件 {relative_path} 缺少仓型字段 warehouse_type。请在源数据中补充该列，"
            "每行使用 center 或 cross_docking，然后再继续。"
        )
    fields = ", ".join(missing_fields)
    return f"文件 {relative_path} 缺少必需字段：{fields}。请补充源数据后再继续。"


@mcp.tool(structured_output=True, annotations=WORKSPACE_PREPARATION_TOOL)
def prepare_network_input(
    inspection_identity: SourceInspectionIdentity,
    inspected_relative_paths: list[str],
    source_selections: Annotated[
        list[SourceSelection],
        Field(
            min_length=1,
            max_length=640,
            description=(
                "Select exact units from the latest workspace_source_profile.v3. Each item must "
                "include relative_path, unit_ref, role, and optional explicit mappings. "
                "Never include an administrative catalog here."
            )
        ),
    ],
    country_code: Annotated[str, Field(pattern=r"^[A-Za-z]{2}$")],
    output_relative_path: Annotated[
        str,
        Field(
            min_length=1,
            max_length=1024,
            description=(
                "Create-new JSON path directly under outputs/warehouse-network/prepared/."
            ),
        ),
    ],
    ctx: Context,
    administrative_catalog_relative_path: Annotated[
        str | None,
        Field(
            description=(
                "Optional exact Workspace-relative administrative JSON path used atomically "
                "for geography enrichment. The same file must not appear in source_selections."
            )
        ),
    ] = None,
    overrides: list[GeographyOverride] | None = None,
) -> DataPreparationToolResult:
    """Prepare one complete, validated Workspace input from exact source units."""
    inspection_identity = SourceInspectionIdentity.model_validate(inspection_identity)
    source_selections = [SourceSelection.model_validate(selection) for selection in source_selections]
    if not source_selections:
        raise ProviderContractError("source_selections_required")
    if len(source_selections) > 640:
        raise ProviderContractError("source_selection_limit_exceeded")
    selection_keys = [
        (selection.relative_path, selection.unit_ref, selection.role.value)
        for selection in source_selections
    ]
    if len(selection_keys) != len(set(selection_keys)):
        raise ProviderContractError("source_selection_duplicate")
    try:
        inspected = _inspect_workspace_sources(inspected_relative_paths, ctx)
    except ValueError as error:
        if _is_source_lifecycle_error(error):
            return _source_changed_result()
        raise
    if isinstance(inspected, DataInspectionSelectionRequired):
        raise ProviderContractError("inspection_selection_required")
    try:
        inspection_snapshot = source_inspection_snapshot(
            _workspace(ctx), inspected_relative_paths
        )
    except ValueError as error:
        if _is_source_lifecycle_error(error):
            return _source_changed_result()
        raise
    if (
        inspection_identity.schema_version != "workspace_source_inspection.v2"
        or inspection_identity.content_sha256 != inspection_snapshot.content_sha256
        or inspection_identity.source_count != inspection_snapshot.source_count
    ):
        return _source_changed_result()
    country = country_code.strip().upper()
    if not re.fullmatch(r"[A-Z]{2}", country):
        raise ValueError("country_code_required_iso_alpha2")
    prepare_workspace_output_path(
        _workspace(ctx), output_relative_path, WorkspaceOutputKind.PREPARED_INPUT
    )
    available = {str(source["relative_path"]): source for source in inspected["sources"]}
    inspected_set = set(inspected_relative_paths)
    if administrative_catalog_relative_path is not None:
        if administrative_catalog_relative_path not in inspected_set:
            raise ProviderContractError("administrative_catalog_not_in_inspected_paths")
        if any(
            selection.relative_path == administrative_catalog_relative_path
            for selection in source_selections
        ):
            raise ProviderContractError("administrative_catalog_source_overlap")
    resolved: list[_ResolvedSelection] = []
    requirements: list[DataSourceRequirement] = []
    for selection in source_selections:
        if selection.relative_path not in inspected_set or selection.relative_path not in available:
            raise ProviderContractError("source_selection_path_invalid")
        resolution = _resolve_source_selection(selection, available[selection.relative_path])
        if isinstance(resolution, DataSourceRequirement):
            requirements.append(resolution)
        else:
            resolved.append(resolution)
    if requirements:
        return _needs_input_result(requirements)

    normalized_sources: list[ConfirmedSourceRows] = []
    rows_cache: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for selection_index, item in enumerate(resolved):
        cache_key = (item.selection.relative_path, item.selection.unit_ref)
        if cache_key not in rows_cache:
            try:
                rows_cache[cache_key] = read_unit_rows(
                    _workspace(ctx),
                    item.selection.relative_path,
                    item.selection.unit_ref,
                    locator=item.unit["locator"],
                )
            except ValueError as error:
                if _is_source_lifecycle_error(error):
                    return _source_changed_result()
                if str(error) == "source_unit_item_not_object":
                    return _needs_input_result(
                        [
                            _mapping_requirement(
                                item.selection,
                                f"文件 {item.selection.relative_path} 的单元 {item.selection.unit_ref} 包含无法映射的非对象记录。",
                                code="source_data_invalid",
                            )
                        ]
                    )
                if str(error) == "source_unit_duplicate_columns":
                    return _needs_input_result(
                        [
                            _mapping_requirement(
                                item.selection,
                                f"文件 {item.selection.relative_path} 的单元 {item.selection.unit_ref} 存在重复列名，无法安全映射。",
                                code="source_data_invalid",
                            )
                        ]
                    )
                raise ProviderContractError("source_unit_read_invalid") from error
        mappings = [
            ConfirmedFieldMapping(
                target_field=mapping.target_field,
                source_field=mapping.source_field,
                transform=TransformSpec(kind=mapping.transform, factor=mapping.factor),
            )
            for mapping in item.decision.mappings
        ]
        normalized_sources.append(
            ConfirmedSourceRows(
                role=item.decision.role,
                rows=rows_cache[cache_key],
                mappings=mappings,
                relative_path=item.selection.relative_path,
                unit_ref=item.selection.unit_ref,
                selection_key=f"selection:{selection_index}",
            )
        )
    try:
        fresh_snapshot = source_inspection_snapshot(
            _workspace(ctx), inspected_relative_paths
        )
    except ValueError as error:
        if _is_source_lifecycle_error(error):
            return _source_changed_result()
        raise
    if (
        fresh_snapshot.content_sha256 != inspection_snapshot.content_sha256
        or fresh_snapshot.source_count != inspection_snapshot.source_count
    ):
        return _source_changed_result()
    state, batch = normalize_confirmed_rows(normalized_sources)
    if state == "needs_input":
        return _needs_input_result(_batch_requirements(normalized_sources, batch))
    try:
        raw_hashes: dict[str, str] = {}
        prepared_selections: list[PreparedSourceSelection] = []
        for item in resolved:
            if item.selection.relative_path not in raw_hashes:
                raw_hashes[item.selection.relative_path] = fresh_snapshot.file_sha256[
                    item.selection.relative_path
                ]
            canonical_mappings = sorted(
                item.decision.mappings,
                key=lambda mapping: (
                    mapping.target_field,
                    mapping.source_field,
                    mapping.transform.value,
                    str(mapping.factor),
                ),
            )
            prepared_selections.append(
                PreparedSourceSelection(
                    relative_path=item.selection.relative_path,
                    unit_ref=item.selection.unit_ref,
                    role=item.decision.role,
                    mappings=canonical_mappings,
                    raw_content_sha256=raw_hashes[item.selection.relative_path],
                )
            )
        administrative_provenance = (
            PreparedAdministrativeCatalog(
                relative_path=administrative_catalog_relative_path,
                content_sha256=fresh_snapshot.file_sha256[administrative_catalog_relative_path],
            )
            if administrative_catalog_relative_path is not None
            else None
        )
    except ValueError as error:
        if _is_source_lifecycle_error(error):
            return _source_changed_result()
        raise
    prepared_selections = sorted(
        prepared_selections,
        key=lambda item: (item.relative_path, item.unit_ref, item.role.value),
    )
    role_counts = PreparationRoleCounts(
        demand=len(batch.demand_cities),
        existing_warehouse=sum(item.is_existing for item in batch.warehouses),
        candidate_warehouse=sum(not item.is_existing for item in batch.warehouses),
        current_assignment=len(batch.current_assignments),
        route_quote=len(batch.route_quotes),
        provided_route_fact=len(batch.provided_route_facts),
    )
    payload = PreparedNetworkResource(
        country_code=country,
        state=state,
        source_selections=prepared_selections,
        administrative_catalog=administrative_provenance,
        selected_source_identity=derive_selected_source_identity(
            country, prepared_selections, administrative_provenance
        ),
        issue_count=len(batch.issues),
        issues_truncated=len(batch.issues) > 64,
        roles=sorted({item.role for item in prepared_selections}, key=lambda role: role.value),
        role_counts=role_counts,
        **{
            **batch.model_dump(mode="json"),
            "issues": batch.issues[:64],
        },
    )
    if administrative_catalog_relative_path is not None:
        try:
            geography = _enrich_prepared_geography(
                payload,
                administrative_catalog_relative_path,
                ctx,
                overrides,
                expected_administrative_sha256=payload.administrative_catalog.content_sha256
                if payload.administrative_catalog is not None
                else None,
            )
        except (ValueError, KeyError) as error:
            if str(error) == "source_changed":
                return _source_changed_result()
            return _needs_input_result(
                [_administrative_requirement(administrative_catalog_relative_path, error)]
            )
        if geography.prepared is None:
            return _needs_input_result(
                _geography_requirements(
                    normalized_sources,
                    _batch_from_payload_with_issues(payload, geography.issues),
                    administrative_catalog_relative_path,
                )
            )
        payload = geography.prepared
        return _write_prepared_input(
            payload,
            output_relative_path,
            ctx,
            f"Normalized {len(resolved)} selected source units and {geography.summary}",
        )
    return _write_prepared_input(
        payload,
        output_relative_path,
        ctx,
        f"Normalized {len(resolved)} selected source units; state is {state}.",
    )


@dataclass(frozen=True)
class _ResolvedSelection:
    selection: SourceSelection
    decision: SourceSelection
    unit: dict[str, Any]


@dataclass(frozen=True)
class _GeographyEnrichment:
    prepared: PreparedNetworkResource | None
    summary: str
    issues: list[DataQualityIssue]


def _resolve_source_selection(
    selection: SourceSelection,
    source: dict[str, Any],
) -> _ResolvedSelection | DataSourceRequirement:
    units = [unit for unit in source.get("units", []) if unit.get("unit_ref") == selection.unit_ref]
    if len(units) != 1:
        raise ProviderContractError("source_unit_ref_invalid")
    unit = units[0]
    field_names = {
        str(field.get("name"))
        for field in unit.get("fields", [])
        if isinstance(field, dict) and field.get("name")
    }
    if selection.mappings:
        unknown = sorted({mapping.source_field for mapping in selection.mappings} - field_names)
        if unknown:
            raise ProviderContractError("source_selection_field_invalid")
        missing = sorted(
            REQUIRED_FIELDS[selection.role]
            - {mapping.target_field for mapping in selection.mappings}
        )
        if missing:
            raise ProviderContractError("source_selection_required_mapping_missing")
        decision = selection
        return _ResolvedSelection(selection=selection, decision=decision, unit=unit)

    assessments = [
        item
        for item in unit.get("role_assessments", [])
        if item.get("role") == selection.role.value
    ]
    complete = [item for item in assessments if item.get("state") == "complete"]
    if len(complete) == 1:
        mappings: list[ConfirmedFieldDecision] = []
        for mapping in complete[0].get("field_mappings", []):
            source_fields = mapping.get("source_fields")
            transform = mapping.get("transform")
            if not isinstance(source_fields, list) or len(source_fields) != 1:
                return _mapping_requirement(selection, "字段映射存在多义，无法自动选择。")
            if not isinstance(transform, dict) or not isinstance(transform.get("kind"), str):
                raise ProviderContractError("source_selection_mapping_invalid")
            mappings.append(
                ConfirmedFieldDecision(
                    source_field=str(source_fields[0]),
                    target_field=str(mapping["target_field"]),
                    transform=transform["kind"],
                    factor=transform.get("factor"),
                )
            )
        decision = SourceSelection(
            relative_path=selection.relative_path,
            unit_ref=selection.unit_ref,
            role=selection.role,
            mappings=mappings,
        )
        return _ResolvedSelection(selection=selection, decision=decision, unit=unit)
    for assessment in assessments:
        missing = tuple(sorted(str(value) for value in assessment.get("missing_required_fields", [])))
        if assessment.get("state") == "partial" and missing:
            return _mapping_requirement(
                selection,
                _missing_fields_question(selection.relative_path, selection.unit_ref, missing),
                missing,
                code="required_fields_missing",
            )
    if any(item.get("state") == "ambiguous" for item in assessments):
        return _mapping_requirement(selection, f"文件 {selection.relative_path} 的单元 {selection.unit_ref} 业务角色或字段映射存在歧义。")
    return _mapping_requirement(selection, f"文件 {selection.relative_path} 的单元 {selection.unit_ref} 无法完整映射为 {selection.role.value}。")


def _mapping_requirement(
    selection: SourceSelection,
    question: str,
    missing: tuple[str, ...] = (),
    *,
    code: str = "field_mapping_confirmation_required",
) -> DataSourceRequirement:
    return DataSourceRequirement(
        code=code,
        relative_path=selection.relative_path,
        unit_ref=selection.unit_ref,
        candidate_roles=[selection.role],
        missing_required_fields=list(missing),
        question=question,
    )


def _missing_fields_question(
    relative_path: str, unit_ref: str, missing_fields: tuple[str, ...]
) -> str:
    if missing_fields == ("warehouse_type",):
        return (
            f"文件 {relative_path} 的单元 {unit_ref} 缺少仓型字段 warehouse_type。"
            "请补充该列，每行使用 center 或 cross_docking，然后再继续。"
        )
    fields = ", ".join(missing_fields)
    return f"文件 {relative_path} 的单元 {unit_ref} 缺少必需字段：{fields}。请补充后再继续。"


def _batch_requirements(
    sources: list[ConfirmedSourceRows], batch: NormalizedInputBatch
) -> list[DataSourceRequirement]:
    requirements: list[DataSourceRequirement] = []
    for issue in batch.issues:
        if issue.severity != "error":
            continue
        source = next(
            (item for item in sources if _source_id(item) == issue.source_id),
            _source_for_issue(sources, issue.code),
        )
        if source is None:
            continue
        code = (
            "source_duplicate_conflict"
            if "duplicate_conflict" in issue.code
            else "source_data_invalid"
        )
        requirements.append(
            DataSourceRequirement(
                code=code,
                relative_path=source.relative_path,
                unit_ref=source.unit_ref,
                candidate_roles=[SourceRole(source.role)],
                missing_required_fields=[],
                field_name=issue.field_name,
                question=(
                    f"文件 {source.relative_path} 的单元 {source.unit_ref} 存在"
                    f" {issue.code}，请修正该选中来源后再继续。"
                ),
            )
        )
    return requirements


def _geography_requirements(
    sources: list[ConfirmedSourceRows],
    batch: NormalizedInputBatch,
    administrative_path: str,
) -> list[DataSourceRequirement]:
    requirements: list[DataSourceRequirement] = []
    for issue in batch.issues:
        if issue.severity != "error" or not issue.code.startswith("geography_"):
            continue
        field_name = issue.field_name or ""
        role = (
            SourceRole.DEMAND
            if field_name.startswith("demand:")
            else SourceRole.EXISTING_WAREHOUSE
            if field_name.startswith("existing_warehouse:")
            else SourceRole.CANDIDATE_WAREHOUSE
            if field_name.startswith("candidate_warehouse:")
            else None
        )
        source = next((item for item in sources if item.role == role), None) if role else None
        if source is None:
            requirements.append(
                _administrative_requirement(
                    administrative_path,
                    ValueError(issue.code),
                    field_name=field_name,
                )
            )
            continue
        requirements.append(
            DataSourceRequirement(
                code="source_data_invalid",
                relative_path=source.relative_path,
                unit_ref=source.unit_ref,
                candidate_roles=[role],
                missing_required_fields=[],
                field_name=field_name,
                question=(
                    f"文件 {source.relative_path} 的单元 {source.unit_ref} 的地理字段"
                    f" {field_name} 无法匹配行政区目录，请修正选中来源后再继续。"
                ),
            )
        )
    return requirements


def _source_for_issue(
    sources: list[ConfirmedSourceRows], code: str
) -> ConfirmedSourceRows | None:
    prefixes = (
        ("demand", {SourceRole.DEMAND}),
        ("warehouse", {SourceRole.EXISTING_WAREHOUSE, SourceRole.CANDIDATE_WAREHOUSE}),
        ("assignment", {SourceRole.CURRENT_ASSIGNMENT}),
        ("route", {SourceRole.ROUTE_QUOTE}),
        ("provided_route", {SourceRole.ROUTE_QUOTE}),
    )
    for prefix, roles in prefixes:
        if code.startswith(prefix):
            match = next((source for source in sources if source.role in roles), None)
            if match is not None:
                return match
    return sources[0] if sources else None


def _source_id(source: ConfirmedSourceRows) -> str | None:
    return source.selection_key[:64] or None


def _administrative_requirement(
    path: str, error: Exception, *, field_name: str = "administrative_catalog"
) -> DataSourceRequirement:
    return DataSourceRequirement(
        code="business_rule_unknown",
        relative_path=path,
        unit_ref="document",
        candidate_roles=[SourceRole.ADMINISTRATIVE_CATALOG],
        missing_required_fields=[],
        field_name=field_name,
        question=f"行政区目录 {path} 无法完成地理补全，请修正该目录后再继续。",
    )


def _batch_from_payload(payload: PreparedNetworkResource) -> NormalizedInputBatch:
    return NormalizedInputBatch.model_validate(
        payload.model_dump(
            include={
                "demand_cities",
                "warehouses",
                "current_assignments",
                "route_quotes",
                "provided_route_facts",
                "issues",
            }
        )
    )


def _batch_from_payload_with_issues(
    payload: PreparedNetworkResource, issues: list[DataQualityIssue]
) -> NormalizedInputBatch:
    return NormalizedInputBatch.model_validate(
        {
            **_batch_from_payload(payload).model_dump(mode="json"),
            "issues": issues,
        }
    )


def _dedupe_requirements(
    requirements: list[DataSourceRequirement],
) -> list[DataSourceRequirement]:
    result: list[DataSourceRequirement] = []
    keys: set[tuple[Any, ...]] = set()
    for requirement in requirements:
        key = (
            requirement.code,
            requirement.relative_path,
            requirement.unit_ref,
            requirement.field_name,
            tuple(requirement.missing_required_fields),
        )
        if key not in keys:
            keys.add(key)
            result.append(requirement)
    return result


def _enrich_prepared_geography(
    payload: PreparedNetworkResource,
    administrative_catalog_relative_path: str,
    ctx: Context,
    overrides: list[GeographyOverride] | None,
    *,
    expected_administrative_sha256: str | None = None,
) -> _GeographyEnrichment:
    country_code = payload.country_code
    batch = NormalizedInputBatch.model_validate(
        payload.model_dump(
            include={
                "demand_cities",
                "warehouses",
                "current_assignments",
                "route_quotes",
                "provided_route_facts",
                "issues",
            }
        )
    )
    try:
        catalog_document, catalog_sha256 = read_json_document_with_sha256(
            _workspace(ctx), administrative_catalog_relative_path
        )
    except ValueError:
        if expected_administrative_sha256 is not None:
            raise ValueError("source_changed") from None
        raise
    if (
        expected_administrative_sha256 is not None
        and catalog_sha256 != expected_administrative_sha256
    ):
        raise ValueError("source_changed")
    admin_level = catalog_document.get("admin_level")
    if not isinstance(admin_level, str) or not admin_level.strip():
        raise ValueError("administrative_catalog_level_missing")
    catalog = _load_administrative_catalog(
        country_code,
        admin_level.strip(),
        catalog_document,
    )
    override_map = {
        (override.entity, override.entity_id): override.catalog_city_id
        for override in overrides or []
    }
    if len(override_map) != len(overrides or []):
        raise ValueError("geography_overrides_must_be_unique")
    demands, warehouses, _candidates, issues = enrich_network_geography(
        batch.demand_cities,
        batch.warehouses,
        catalog,
        overrides=override_map,
    )
    combined_issues = [*batch.issues, *issues]
    missing_coordinates = sum(
        item.longitude is None or item.latitude is None
        for item in [*demands, *warehouses]
    )
    state = (
        "needs_input"
        if any(issue.severity == "error" for issue in combined_issues)
        else "needs_geography"
        if missing_coordinates
        else "ready"
    )
    summary = (
        f"enriched geography from the catalog's {admin_level.strip()} level; "
        f"{len(demands)} demand cities and {len(warehouses)} warehouses; "
        f"missing-coordinate records {missing_coordinates}; state is {state}"
    )
    if state == "needs_input":
        return _GeographyEnrichment(None, summary, combined_issues)
    prepared_batch = batch.model_copy(
        update={
            "demand_cities": demands,
            "warehouses": warehouses,
            "issues": combined_issues,
        }
    )
    admin_provenance = PreparedAdministrativeCatalog(
        relative_path=administrative_catalog_relative_path,
        content_sha256=catalog_sha256,
    )
    prepared_payload = PreparedNetworkResource(
        country_code=payload.country_code,
        state=state,
        source_selections=payload.source_selections,
        administrative_catalog=admin_provenance,
        selected_source_identity=derive_selected_source_identity(
            payload.country_code, payload.source_selections, admin_provenance
        ),
        roles=payload.roles,
        role_counts=payload.role_counts,
        issue_count=len(combined_issues),
        issues_truncated=len(combined_issues) > 64,
        **{
            **prepared_batch.model_dump(mode="json"),
            "issues": combined_issues[:64],
        },
    )
    return _GeographyEnrichment(prepared_payload, summary, combined_issues)


@mcp.tool(structured_output=True, annotations=WORKSPACE_PREPARATION_TOOL)
def prepare_network_geography(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    administrative_catalog_relative_path: str,
    output_relative_path: Annotated[
        str,
        Field(
            min_length=1,
            max_length=1024,
            description=(
                "Create-new JSON path directly under outputs/warehouse-network/prepared/."
            ),
        ),
    ],
    ctx: Context,
    overrides: list[GeographyOverride] | None = None,
) -> DataPreparationToolResult:
    """Create a new prepared Workspace input enriched by one exact boundary catalog."""
    payload, _input_identity = load_prepared_network_input(
        _workspace(ctx), prepared_input_relative_path
    )
    fresh, _reason = validate_prepared_freshness(
        _workspace(ctx),
        payload,
        required_roles=payload.roles,
        country_code=payload.country_code,
        allow_needs_geography=True,
        check_administrative_catalog=False,
    )
    if not fresh:
        return _source_changed_result()
    if any(
        selection.relative_path == administrative_catalog_relative_path
        for selection in payload.source_selections
    ):
        raise ProviderContractError("administrative_catalog_source_overlap")
    try:
        geography = _enrich_prepared_geography(
            payload,
            administrative_catalog_relative_path,
            ctx,
            overrides,
        )
    except (ValueError, KeyError) as error:
        return _needs_input_result(
            [_administrative_requirement(administrative_catalog_relative_path, error)]
        )
    if geography.prepared is None:
        pseudo_sources = [
            ConfirmedSourceRows(
                role=selection.role,
                rows=[],
                mappings=[],
                relative_path=selection.relative_path,
                unit_ref=selection.unit_ref,
                selection_key=f"prepared:{index}",
            )
            for index, selection in enumerate(payload.source_selections)
        ]
        return _needs_input_result(
            _geography_requirements(
                pseudo_sources,
                _batch_from_payload_with_issues(payload, geography.issues),
                administrative_catalog_relative_path,
            )
        )
    return _write_prepared_input(
        geography.prepared,
        output_relative_path,
        ctx,
        f"Prepared network geography and {geography.summary}.",
    )


@mcp.tool(
    structured_output=True,
    annotations=READ_ONLY_LOCAL_TOOL,
    description=(
        "Answer business fact questions exactly over every normalized record of one prepared "
        "input: counts, group-by breakdowns, sum/min/max/mean and bounded filtered row lists. "
        "Use it for questions such as how many warehouses of each type exist, which cities "
        "have the most demand, or which candidates lie in a province; never answer such "
        "questions from inspection statistics. Each group carries its exact record count; "
        "measures add sums or extremes. roles and role_counts show which roles the "
        "prepared file contains; a role it did not prepare is not evidence of zero. group_by returns groups and no rows; without "
        "group_by it returns at most `limit` rows plus the exact matched_count. A changed "
        "selected source returns prepared_input_stale instead of facts. Fields by dataset: "
        f"{field_catalog_description()}."
    ),
)
def query_prepared_network_input(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    dataset: PreparedQueryDataset,
    ctx: Context,
    filters: Annotated[list[PreparedQueryFilter], Field(max_length=8)] | None = None,
    group_by: Annotated[list[str], Field(max_length=2)] | None = None,
    measures: Annotated[list[PreparedQueryMeasure], Field(max_length=4)] | None = None,
    select: Annotated[list[str], Field(max_length=16)] | None = None,
    order_by: PreparedQueryOrder | None = None,
    limit: Annotated[int, Field(ge=0, le=MAX_QUERY_ROWS)] = 20,
) -> Annotated[CallToolResult, PreparedQueryToolResult]:
    root = _workspace(ctx)
    prepared, identity = load_prepared_network_input(root, prepared_input_relative_path)
    fresh, reason = validate_prepared_freshness(
        root,
        prepared,
        required_roles=prepared.roles,
        country_code=prepared.country_code,
        allow_needs_geography=True,
    )
    if fresh:
        outcome = query_prepared_input(
            prepared,
            identity,
            prepared_input_relative_path,
            dataset,
            filters=filters or (),
            group_by=group_by or (),
            measures=measures or (),
            select=select or (),
            order_by=order_by,
            limit=limit,
        )
    else:
        outcome = PreparedQueryStale(
            outcome="prepared_input_stale",
            summary=(
                f"Prepared input {prepared_input_relative_path} no longer matches its selected "
                f"sources ({reason}); reinspect and prepare again before answering."
            ),
            reason=reason,
            prepared_input_relative_path=prepared_input_relative_path,
            next_action="reinspect",
            retryable=False,
        )
    result = PreparedQueryToolResult(root=outcome)
    return CallToolResult(
        content=[TextContent(type="text", text=outcome.summary)],
        structuredContent=result.model_dump(mode="json", by_alias=True),
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Typed supply-chain inspection and preparation MCP server"
    )
    parser.add_argument("--transport", choices=("stdio",), default="stdio")
    parser.parse_args()

    global _workspace_root, _profile_state_root, _supply_chain_resources
    _workspace_root = Path.cwd().resolve(strict=True)
    _profile_state_root = Path(os.environ.get("CODEX_HOME", _workspace_root / ".codex")).resolve()
    _supply_chain_resources = SupplyChainResources(_workspace_root, _profile_state_root)
    asyncio.run(run_stdio())


async def run_stdio() -> None:
    initialization_options = mcp._mcp_server.create_initialization_options(
        experimental_capabilities={SANDBOX_STATE_META_CAPABILITY: {}},
    )
    async with stdio_server() as streams:
        await mcp._mcp_server.run(streams[0], streams[1], initialization_options)


if __name__ == "__main__":
    main()
