"""Workspace-backed planning-input contract shared by Data and Network tools."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath

from supply_chain_planner.data.mapping import REQUIRED_FIELDS, PlanningSourceRole
from supply_chain_planner.data.workspace_intake import (
    inspect,
    read_json_document_with_sha256,
    source_content_sha256,
)
from supply_chain_planner.network.models import PlanningInputIdentity
from supply_chain_planner.shared.models import (
    PreparedAdministrativeCatalog,
    PreparedNetworkResource,
    PreparedSourceSelection,
    SourceSelection,
)

PREPARED_PARENT = PurePosixPath("outputs/warehouse-network/prepared")


def load_prepared_network_input(
    workspace_root: Path,
    prepared_input_relative_path: str,
) -> tuple[PreparedNetworkResource, PlanningInputIdentity]:
    """Load one exact Tool-created planning input from the authorized Workspace."""
    _validate_prepared_path(prepared_input_relative_path)
    payload, content_sha256 = read_json_document_with_sha256(
        workspace_root,
        prepared_input_relative_path,
    )
    try:
        prepared = PreparedNetworkResource.model_validate(payload)
    except ValueError as error:
        raise ValueError("prepared_network_input_invalid") from error
    if prepared.selected_source_identity != derive_selected_source_identity(
        prepared.country_code,
        prepared.source_selections,
        prepared.administrative_catalog,
    ):
        raise ValueError("prepared_selected_source_identity_invalid")
    for selection in prepared.source_selections:
        _validate_provenance_path(selection.relative_path, allowed_suffixes={".csv", ".json", ".xlsx"})
    if prepared.administrative_catalog is not None:
        _validate_provenance_path(
            prepared.administrative_catalog.relative_path,
            allowed_suffixes={".json"},
        )
    return prepared, PlanningInputIdentity(content_sha256=content_sha256)


def derive_selected_source_identity(
    country_code: str,
    source_selections: list[PreparedSourceSelection],
    administrative_catalog: PreparedAdministrativeCatalog | None,
) -> str:
    canonical = {
        "country_code": country_code.upper(),
        "source_selections": [
            _canonical_selection_payload(item)
            for item in sorted(
                source_selections,
                key=lambda value: (value.relative_path, value.unit_ref, value.role.value),
            )
        ],
        "administrative_catalog": (
            administrative_catalog.model_dump(mode="json", by_alias=True)
            if administrative_catalog is not None
            else None
        ),
    }
    encoded = json.dumps(canonical, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _canonical_selection_payload(item: PreparedSourceSelection) -> dict[str, object]:
    payload = item.model_dump(mode="json", by_alias=True)
    payload["mappings"] = [
        mapping.model_dump(mode="json", by_alias=True)
        for mapping in sorted(
            item.mappings,
            key=lambda mapping: (
                mapping.target_field,
                mapping.source_field,
                mapping.transform.value,
                str(mapping.factor),
            ),
        )
    ]
    return payload


def validate_prepared_freshness(
    workspace_root: Path,
    prepared: PreparedNetworkResource,
    *,
    required_roles: list[PlanningSourceRole],
    country_code: str,
    allow_needs_geography: bool = False,
    check_administrative_catalog: bool = True,
) -> tuple[bool, str]:
    """Validate a prepared candidate's selected raw sources and unit mappings."""

    if prepared.state != "ready" and not (
        allow_needs_geography and prepared.state == "needs_geography"
    ):
        return False, "state_not_ready"
    if prepared.country_code != country_code.upper():
        return False, "country_mismatch"
    selected_roles = {item.role.value for item in prepared.source_selections}
    if not {role.value for role in required_roles} <= selected_roles:
        return False, "required_roles_missing"
    if prepared.selected_source_identity != derive_selected_source_identity(
        prepared.country_code,
        prepared.source_selections,
        prepared.administrative_catalog,
    ):
        return False, "selected_identity_mismatch"
    hash_before_cache: dict[str, str] = {}
    hash_after_cache: dict[str, str] = {}
    structure_cache: dict[str, dict[str, object]] = {}
    for selection in prepared.source_selections:
        try:
            if selection.relative_path not in structure_cache:
                hash_before = source_content_sha256(workspace_root, selection.relative_path)
                hash_before_cache[selection.relative_path] = hash_before
                structure_cache[selection.relative_path] = inspect(
                    workspace_root, selection.relative_path
                )["structure"]
                hash_after = source_content_sha256(workspace_root, selection.relative_path)
                hash_after_cache[selection.relative_path] = hash_after
                if hash_before != selection.raw_content_sha256 or hash_after != hash_before:
                    return False, "source_changed"
            elif (
                hash_before_cache[selection.relative_path] != selection.raw_content_sha256
                or hash_after_cache[selection.relative_path]
                != hash_before_cache[selection.relative_path]
            ):
                return False, "source_changed"
            structure = structure_cache[selection.relative_path]
            unit = _find_inspected_unit(structure, selection.unit_ref)
            fields = {
                str(field.get("name"))
                for field in unit.get("fields", [])
                if isinstance(field, dict) and field.get("name")
            }
            source_selection = SourceSelection.model_validate(
                selection.model_dump(mode="json", exclude={"raw_content_sha256"})
            )
            if not set(mapping.source_field for mapping in source_selection.mappings) <= fields:
                return False, "mapping_source_missing"
            if not REQUIRED_FIELDS[source_selection.role] <= {
                mapping.target_field for mapping in source_selection.mappings
            }:
                return False, "mapping_target_missing"
        except (OSError, ValueError):
            return False, "source_invalid"
    if check_administrative_catalog and prepared.administrative_catalog is not None:
        try:
            if source_content_sha256(
                workspace_root, prepared.administrative_catalog.relative_path
            ) != prepared.administrative_catalog.content_sha256:
                return False, "administrative_catalog_changed"
        except (OSError, ValueError):
            return False, "administrative_catalog_invalid"
    # A file prepared before this rule was enforced must not be reused: every
    # cross-dock needs an upstream center for its linehaul route.
    if any(
        warehouse.warehouse_type == "cross_docking" and warehouse.upstream_center_id is None
        for warehouse in prepared.warehouses
    ):
        return False, "warehouse_upstream_center_required"
    return True, "fresh"


def _find_inspected_unit(structure: dict[str, object], unit_ref: str) -> dict[str, object]:
    kind = structure.get("kind")
    if kind == "table":
        units = [{"unit_ref": "table", **structure}]
    elif kind == "workbook":
        units = [
            {"unit_ref": sheet.get("unit_ref", f"sheet:{sheet.get('sheet', '')}"), **sheet}
            for sheet in structure.get("sheets", [])
        ]
    elif kind == "json":
        units = [
            {"unit_ref": array.get("unit_ref", array.get("path", "$")), **array}
            for array in structure.get("arrays", [])
        ]
    else:
        units = []
    matches = [unit for unit in units if unit.get("unit_ref") == unit_ref]
    if len(matches) != 1:
        raise ValueError("prepared_unit_invalid")
    return matches[0]


def _validate_prepared_path(relative_path: str) -> None:
    candidate = PurePosixPath(relative_path)
    if (
        candidate.is_absolute()
        or candidate.as_posix() != relative_path
        or candidate.parent != PREPARED_PARENT
        or candidate.suffix.lower() != ".json"
        or candidate.name in {"", ".", ".."}
    ):
        raise ValueError("prepared_input_path_invalid")


def _validate_provenance_path(relative_path: str, *, allowed_suffixes: set[str]) -> None:
    candidate = PurePosixPath(relative_path)
    if (
        candidate.is_absolute()
        or candidate.as_posix() != relative_path
        or not candidate.parts
        or any(part in {"", ".", ".."} for part in candidate.parts)
        or candidate.suffix.lower() not in allowed_suffixes
    ):
        raise ValueError("prepared_provenance_path_invalid")


def require_matching_input(
    expected: PlanningInputIdentity,
    actual: PlanningInputIdentity,
) -> None:
    """Reject a valid-looking matrix or result derived from another input."""
    if expected != actual:
        raise ValueError("planning_input_identity_mismatch")
