"""Resolve explicitly given prepared inputs for the requested planning roles.

Raw-source inspection and prepared-input reuse are separate Data contracts.
This module owns reuse only: it validates each explicitly given prepared file
for freshness, then compares the normalized records of the requested roles.
Files that hold identical records for those roles are interchangeable for the
goal, so one is reused without asking; only files whose requested-role records
differ require a user choice.
"""

from __future__ import annotations

__all__ = ["MAX_PREPARED_CANDIDATES", "resolve_prepared_inputs"]

import hashlib
import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

from supply_chain_planner.data.mapping import PlanningSourceRole
from supply_chain_planner.data.workspace_intake import discover_prepared_candidates
from supply_chain_planner.network.models import NetworkRecord, PlanningInputIdentity
from supply_chain_planner.shared.models import (
    PreparedCandidateSummary,
    PreparedNetworkResource,
    PreparedResolutionReady,
    PreparedResolutionSelectionRequired,
    PreparedResolutionUnavailable,
)
from supply_chain_planner.shared.planning_input import (
    load_prepared_network_input,
    validate_prepared_freshness,
)

MAX_PREPARED_CANDIDATES = 64

_Candidate = tuple[str, PreparedNetworkResource, PlanningInputIdentity]

_ROLE_RECORDS: dict[
    PlanningSourceRole, Callable[[PreparedNetworkResource], list[NetworkRecord]]
] = {
    PlanningSourceRole.DEMAND: lambda prepared: list(prepared.demand_cities),
    PlanningSourceRole.EXISTING_WAREHOUSE: lambda prepared: [
        item for item in prepared.warehouses if item.is_existing
    ],
    PlanningSourceRole.CANDIDATE_WAREHOUSE: lambda prepared: [
        item for item in prepared.warehouses if not item.is_existing
    ],
    PlanningSourceRole.CURRENT_ASSIGNMENT: lambda prepared: list(prepared.current_assignments),
    PlanningSourceRole.ROUTE_QUOTE: lambda prepared: [
        *prepared.route_quotes,
        *prepared.provided_route_facts,
    ],
}


def resolve_prepared_inputs(
    root: Path,
    prepared_relative_paths: list[str],
    required_roles: list[PlanningSourceRole],
    country_code: str | None,
) -> PreparedResolutionReady | PreparedResolutionSelectionRequired | PreparedResolutionUnavailable:
    """Reuse one fresh prepared input or report why the given files cannot be reused."""
    if not prepared_relative_paths:
        raise ValueError("prepared_relative_paths_required")
    if len(prepared_relative_paths) > MAX_PREPARED_CANDIDATES:
        raise ValueError("prepared_candidate_limit_exceeded")
    if len(set(prepared_relative_paths)) != len(prepared_relative_paths):
        raise ValueError("prepared_relative_paths_duplicate")
    discovered = {str(item["relative_path"]) for item in discover_prepared_candidates(root)}
    if not set(prepared_relative_paths) <= discovered:
        raise ValueError("prepared_candidate_not_found")
    fresh, reasons = _fresh_candidates(
        root, sorted(prepared_relative_paths), required_roles, country_code
    )
    if not fresh:
        return PreparedResolutionUnavailable(
            outcome="prepared_unavailable",
            summary=(
                "No given prepared input is fresh for the requested roles; inspect the raw "
                "sources and prepare a new input."
            ),
            next_action="inspect_sources",
            retryable=False,
            reasons=reasons[:64],
            reason_count=len(reasons),
            reasons_truncated=len(reasons) > 64,
        )
    groups: dict[str, list[_Candidate]] = {}
    for candidate in fresh:
        groups.setdefault(_requested_records_key(candidate[1], required_roles), []).append(
            candidate
        )
    if len(groups) == 1:
        group = next(iter(groups.values()))
        path, prepared, identity = _representative(group)
        persisted = [
            issue.business_message for issue in prepared.issues if issue.severity == "warning"
        ]
        warnings = persisted + reasons
        warning_count = prepared.issue_count + len(reasons)
        return PreparedResolutionReady(
            outcome="prepared_ready",
            operation="reused",
            summary=(
                f"Reused fresh prepared input {path}; {len(group)} given file(s) hold identical "
                "records for the requested roles."
            ),
            next_action="handoff",
            retryable=False,
            prepared_input_relative_path=path,
            input_identity=identity,
            roles=prepared.roles,
            role_counts=prepared.role_counts,
            equivalent_count=len(group),
            warnings=warnings[:64],
            warning_count=warning_count,
            warnings_truncated=warning_count > 64,
        )
    summaries = sorted(
        (_summary(group) for group in groups.values()),
        key=lambda item: item.prepared_input_relative_path,
    )
    return PreparedResolutionSelectionRequired(
        outcome="prepared_selection_required",
        summary=(
            f"{len(summaries)} fresh prepared inputs hold different records for the requested "
            "roles; choose one."
        ),
        next_action="request_user_input",
        retryable=False,
        candidates=summaries[:64],
        candidate_count=len(summaries),
        candidates_truncated=len(summaries) > 64,
    )


def _fresh_candidates(
    root: Path,
    candidate_paths: list[str],
    required_roles: list[PlanningSourceRole],
    country_code: str | None,
) -> tuple[list[_Candidate], list[str]]:
    """Validate candidates against an explicit country or each file's recorded one."""
    fresh: list[_Candidate] = []
    reasons: list[str] = []
    for path in candidate_paths:
        try:
            prepared, identity = load_prepared_network_input(root, path)
            valid, reason = validate_prepared_freshness(
                root,
                prepared,
                required_roles=required_roles,
                country_code=country_code or prepared.country_code,
            )
        except ValueError as error:
            valid, reason = False, str(error).splitlines()[0] or "candidate_invalid"
        except OSError:
            valid, reason = False, "candidate_unavailable"
        if valid:
            fresh.append((path, prepared, identity))
        else:
            reasons.append(f"prepared candidate {path} ignored: {reason}")
    return fresh, reasons


def _requested_records_key(
    prepared: PreparedNetworkResource,
    required_roles: list[PlanningSourceRole],
) -> str:
    canonical: dict[str, Any] = {"country_code": prepared.country_code}
    for role in required_roles:
        records = [
            json.dumps(record.model_dump(mode="json"), ensure_ascii=False, sort_keys=True)
            for record in _ROLE_RECORDS[role](prepared)
        ]
        canonical[role.value] = sorted(records)
    encoded = json.dumps(canonical, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _representative(group: list[_Candidate]) -> _Candidate:
    """Prefer the file with the most prepared roles, then the canonical path."""
    return min(group, key=lambda item: (-len(item[1].roles), item[0]))


def _summary(group: list[_Candidate]) -> PreparedCandidateSummary:
    path, prepared, identity = _representative(group)
    return PreparedCandidateSummary(
        prepared_input_relative_path=path,
        input_identity=identity,
        roles=prepared.roles,
        role_counts=prepared.role_counts,
        equivalent_count=len(group),
    )
