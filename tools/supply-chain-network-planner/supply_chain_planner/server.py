"""FastMCP entry point for supply-chain network planning."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Annotated, Literal

from mcp.server.fastmcp import FastMCP
from mcp.types import CallToolResult, ResourceLink, TextContent
from pydantic import ValidationError

from .core import (
    compare_scenarios,
    create_route_matrix,
    create_snapshot,
    evaluate_current_assignment,
    evaluate_optimized_network,
    solve_location_candidates,
)
from .models import (
    ComparisonToolResult,
    CurrentCoverageResult,
    CurrentCoverageToolResult,
    DataRef,
    FacilityLocationSolution,
    FacilityLocationToolResult,
    NetworkInput,
    NetworkScenarioResult,
    NetworkSnapshot,
    ResourceToolResult,
    RouteEntry,
    RouteMatrix,
    ScenarioComparison,
    ValidationResult,
)
from .resource_store import PublishedResource, ResourceStore, data_ref

MAX_SOURCE_BYTES = 20 * 1024 * 1024

mcp = FastMCP(
    "Supply Chain Network Planner",
    instructions=(
        "Use prepare_network_snapshot before any calculation, then register one immutable "
        "navigation route matrix for that snapshot. Carry every returned data_ref unchanged; "
        "its server is supply_chain_planner and its URI is the durable MCP Resource identity. "
        "Do not describe a result as current coverage unless evaluate_current_coverage was "
        "used: it separates actual assignments from optimized assignments on the same existing "
        "footprint. Scenario and location tools use integer, splittable planning units and an "
        "explicit end-to-end service policy. Location results are exact only over the candidate "
        "facilities contained in the snapshot. Call validate_network_resource before presenting "
        "a decision."
    ),
    json_response=True,
)

_workspace_root = Path.cwd().resolve()
_data_root = Path(os.environ.get("SUPPLY_CHAIN_DATA_ROOT", _workspace_root)).resolve()
_profile_state_root = Path(
    os.environ.get("CODEX_HOME", _workspace_root / ".codex")
).resolve()
_resource_store: ResourceStore | None = None


def _store() -> ResourceStore:
    global _resource_store
    if _resource_store is None:
        resource_root = Path(
            os.environ.get(
                "SUPPLY_CHAIN_RESOURCE_DIR",
                _profile_state_root
                / "mcp-state"
                / "supply-chain-network-planner"
                / "resources",
            )
        ).resolve()
        _resource_store = ResourceStore(resource_root)
    return _resource_store


@mcp.resource(
    "supply-chain://resources/{resource_id}",
    name="supply_chain_resource",
    title="Supply-chain planning resource",
    mime_type="application/json",
)
def read_supply_chain_resource(resource_id: str) -> str:
    """Read an immutable JSON planning resource created by this MCP server."""
    return _store().read(resource_id)


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


def _resource_call_result(
    published: PublishedResource,
    *,
    summary: str,
    structured: dict[str, object],
) -> CallToolResult:
    return CallToolResult(
        content=[
            TextContent(type="text", text=summary),
            _resource_link(published, summary),
        ],
        structuredContent=structured,
    )


def _load_ref(ref: DataRef, model_type):
    if ref.server != "supply_chain_planner":
        raise ValueError("data_ref.server must be supply_chain_planner")
    payload = _store().load(ref)
    value = model_type.model_validate(payload)
    actual_schema = getattr(value, "schema_version", None)
    if actual_schema != ref.resource_schema:
        raise ValueError(
            f"data_ref schema {ref.resource_schema!r} does not match resource schema "
            f"{actual_schema!r}"
        )
    return value


def _load_network_source(source_path: str) -> tuple[NetworkInput, str]:
    candidate = Path(source_path)
    path = candidate.resolve() if candidate.is_absolute() else (_data_root / candidate).resolve()
    if path != _data_root and _data_root not in path.parents:
        raise ValueError("source_path escapes the configured supply-chain data root")
    if path.suffix.lower() != ".json":
        raise ValueError("source_path must point to a JSON file")
    size = path.stat().st_size
    if size > MAX_SOURCE_BYTES:
        raise ValueError(
            f"source file is {size} bytes; maximum supported size is {MAX_SOURCE_BYTES}"
        )
    payload = json.loads(path.read_text(encoding="utf-8"))
    return NetworkInput.model_validate(payload), str(path.relative_to(_data_root))


@mcp.tool(structured_output=True)
def prepare_network_snapshot(
    network: NetworkInput | None = None,
    source_path: str | None = None,
) -> Annotated[CallToolResult, ResourceToolResult]:
    """Validate network data and publish an immutable network_snapshot.v1 Resource.

    Provide exactly one input: typed inline `network` data, or a JSON `source_path`
    resolved within the server-configured data root.
    """
    if (network is None) == (source_path is None):
        raise ValueError("provide exactly one of network or source_path")
    source_name = "inline"
    if source_path is not None:
        network, source_name = _load_network_source(source_path)
    assert network is not None
    snapshot = create_snapshot(network, source_name=source_name)
    published = _store().publish(snapshot.schema_version, snapshot)
    summary = (
        f"Prepared snapshot {snapshot.snapshot_id} with {len(snapshot.demand_points)} "
        f"demand points, {len(snapshot.facilities)} facilities, "
        f"{sum(item.demand_units for item in snapshot.demand_points)} demand units, "
        f"currency {snapshot.currency}, and policy "
        f"{snapshot.service_policy.policy_id}."
    )
    structured = ResourceToolResult(
        summary=summary,
        data_ref=data_ref(published),
    ).model_dump(mode="json")
    return _resource_call_result(published, summary=summary, structured=structured)


@mcp.tool(structured_output=True)
def register_route_matrix(
    snapshot_ref: DataRef,
    provider: str,
    method: Literal["navigation", "quoted", "haversine_estimate"],
    entries: list[RouteEntry],
    require_complete: bool = True,
) -> Annotated[CallToolResult, ResourceToolResult]:
    """Validate and publish route distance/time rows for a prepared snapshot.

    Use `navigation` for provider navigation results, `quoted` for carrier-provided
    lane metrics, and `haversine_estimate` only for explicitly labeled rough estimates.
    Complete matrices are required by default; set `require_complete=false` only for
    explicit diagnostic work, where missing pairs remain visible during evaluation.
    """
    snapshot = _load_ref(snapshot_ref, NetworkSnapshot)
    expected_pairs = {
        (facility.facility_id, demand.demand_id)
        for facility in snapshot.facilities
        for demand in snapshot.demand_points
    }
    supplied_pairs = {
        (entry.origin_facility_id, entry.destination_demand_id) for entry in entries
    }
    missing_pairs = sorted(expected_pairs - supplied_pairs)
    if require_complete and missing_pairs:
        preview = ", ".join(
            f"{origin}->{destination}" for origin, destination in missing_pairs[:5]
        )
        suffix = (
            "" if len(missing_pairs) <= 5 else f" and {len(missing_pairs) - 5} more"
        )
        raise ValueError(
            f"route matrix is missing {len(missing_pairs)} required pairs: "
            f"{preview}{suffix}"
        )
    matrix = create_route_matrix(
        snapshot,
        provider=provider,
        method=method,
        entries=entries,
    )
    published = _store().publish(matrix.schema_version, matrix)
    expected = len(expected_pairs)
    ready = sum(entry.status == "ready" for entry in matrix.entries)
    summary = (
        f"Registered route matrix {matrix.route_matrix_id}: {len(entries)} of {expected} "
        f"facility-demand pairs supplied, {ready} ready, method {matrix.method}, "
        f"provider {matrix.provider}."
    )
    structured = ResourceToolResult(
        summary=summary,
        data_ref=data_ref(published),
    ).model_dump(mode="json")
    return _resource_call_result(published, summary=summary, structured=structured)


@mcp.tool(structured_output=True)
def evaluate_current_coverage(
    snapshot_ref: DataRef,
    route_matrix_ref: DataRef,
) -> Annotated[CallToolResult, CurrentCoverageToolResult]:
    """Calculate actual current coverage and optimized existing-footprint coverage.

    Actual coverage preserves each demand point's `current_facility_id`. Optimized
    coverage reallocates splittable integer demand within existing facility capacities.
    """
    snapshot = _load_ref(snapshot_ref, NetworkSnapshot)
    matrix = _load_ref(route_matrix_ref, RouteMatrix)
    actual = evaluate_current_assignment(snapshot, matrix)
    existing_ids = [
        facility.facility_id for facility in snapshot.facilities if facility.is_existing
    ]
    optimized = evaluate_optimized_network(
        snapshot,
        matrix,
        active_facility_ids=existing_ids,
        scenario_id="optimized_existing_footprint",
    )
    actual_published = _store().publish(actual.schema_version, actual)
    optimized_published = _store().publish(optimized.schema_version, optimized)
    interpretation = (
        "Actual keeps recorded customer-to-warehouse relationships; optimized shows the "
        "best coverage and variable cost available after reallocation on the same footprint."
    )
    aggregate = CurrentCoverageResult(
        snapshot_id=snapshot.snapshot_id,
        route_matrix_id=matrix.route_matrix_id,
        actual_result_ref=data_ref(actual_published),
        optimized_result_ref=data_ref(optimized_published),
        actual_metrics=actual.metrics,
        optimized_metrics=optimized.metrics,
        interpretation=interpretation,
    )
    aggregate_published = _store().publish(aggregate.schema_version, aggregate)
    summary = (
        f"Current actual coverage is {actual.metrics.coverage_ratio:.2%} "
        f"({actual.metrics.covered_demand_units}/{actual.metrics.total_demand_units}); "
        f"optimized coverage on the same existing footprint is "
        f"{optimized.metrics.coverage_ratio:.2%} "
        f"({optimized.metrics.covered_demand_units}/"
        f"{optimized.metrics.total_demand_units})."
    )
    structured = CurrentCoverageToolResult(
        **aggregate.model_dump(),
        summary=summary,
        data_ref=data_ref(aggregate_published),
    ).model_dump(mode="json")
    return _resource_call_result(
        aggregate_published,
        summary=summary,
        structured=structured,
    )


@mcp.tool(structured_output=True)
def evaluate_network_scenario(
    snapshot_ref: DataRef,
    route_matrix_ref: DataRef,
    scenario_id: str,
    active_facility_ids: list[str],
) -> Annotated[CallToolResult, ResourceToolResult]:
    """Optimize coverage and variable cost for an explicit set of active facilities."""
    snapshot = _load_ref(snapshot_ref, NetworkSnapshot)
    matrix = _load_ref(route_matrix_ref, RouteMatrix)
    result = evaluate_optimized_network(
        snapshot,
        matrix,
        active_facility_ids=active_facility_ids,
        scenario_id=scenario_id,
    )
    published = _store().publish(result.schema_version, result)
    summary = (
        f"Scenario {scenario_id}: coverage {result.metrics.coverage_ratio:.2%}, "
        f"covered demand {result.metrics.covered_demand_units}/"
        f"{result.metrics.total_demand_units}, total cost "
        f"{result.metrics.total_cost} {snapshot.currency}, "
        f"{len(result.active_facility_ids)} active facilities."
    )
    structured = ResourceToolResult(
        summary=summary,
        data_ref=data_ref(published),
    ).model_dump(mode="json")
    return _resource_call_result(published, summary=summary, structured=structured)


@mcp.tool(structured_output=True)
def compare_network_scenarios(
    baseline_result_ref: DataRef,
    candidate_result_ref: DataRef,
) -> Annotated[CallToolResult, ComparisonToolResult]:
    """Compare two scenario results built from the same snapshot, routes, and policy."""
    baseline = _load_ref(baseline_result_ref, NetworkScenarioResult)
    candidate = _load_ref(candidate_result_ref, NetworkScenarioResult)
    comparison = compare_scenarios(baseline, candidate)
    published = _store().publish(comparison.schema_version, comparison)
    summary = (
        f"Candidate versus baseline: coverage "
        f"{comparison.coverage_ratio_delta:+.2%}, covered demand "
        f"{comparison.covered_demand_units_delta:+d}, total cost "
        f"{comparison.total_cost_delta:+f}."
    )
    structured = ComparisonToolResult(
        **comparison.model_dump(),
        summary=summary,
        data_ref=data_ref(published),
    ).model_dump(mode="json")
    return _resource_call_result(published, summary=summary, structured=structured)


@mcp.tool(structured_output=True)
def solve_facility_location(
    snapshot_ref: DataRef,
    route_matrix_ref: DataRef,
    target_coverage_ratio: float,
) -> Annotated[CallToolResult, FacilityLocationToolResult]:
    """Find the fewest candidate warehouses that reach a target coverage ratio.

    The solver enumerates every subset of at most 14 candidate facilities. Within
    each subset it solves capacity-constrained, splittable integer allocation by
    min-cost flow. It minimizes added facility count first, then total cost.
    """
    snapshot = _load_ref(snapshot_ref, NetworkSnapshot)
    matrix = _load_ref(route_matrix_ref, RouteMatrix)
    result, selected, evaluated, feasible = solve_location_candidates(
        snapshot,
        matrix,
        target_coverage_ratio=target_coverage_ratio,
    )
    result_published = _store().publish(result.schema_version, result)
    solution = FacilityLocationSolution(
        solution_id=f"solution_{result.result_id.removeprefix('result_')}",
        snapshot_id=snapshot.snapshot_id,
        route_matrix_id=matrix.route_matrix_id,
        target_coverage_ratio=target_coverage_ratio,
        status="optimal" if feasible else "infeasible",
        selected_candidate_facility_ids=selected,
        active_facility_ids=result.active_facility_ids,
        evaluated_subset_count=evaluated,
        result_ref=data_ref(result_published),
        metrics=result.metrics,
        assumptions=[
            "Demand units are nonnegative integers and may split across facilities.",
            "Existing facilities remain open; only snapshot candidate facilities are selectable.",
            "Coverage requires end-to-end seconds at or below the snapshot policy threshold.",
            "Optimality applies only to the finite candidate set and registered route matrix.",
        ],
    )
    solution_published = _store().publish(solution.schema_version, solution)
    status_text = (
        "Found the exact minimum-facility solution"
        if feasible
        else "Target is infeasible for the supplied candidates; returning the best evaluated plan"
    )
    summary = (
        f"{status_text}: add {len(selected)} facilities {selected}, coverage "
        f"{result.metrics.coverage_ratio:.2%}, total cost "
        f"{result.metrics.total_cost} {snapshot.currency}; evaluated {evaluated} subsets."
    )
    structured = FacilityLocationToolResult(
        **solution.model_dump(),
        summary=summary,
        solution_ref=data_ref(solution_published),
    ).model_dump(mode="json")
    return _resource_call_result(
        solution_published,
        summary=summary,
        structured=structured,
    )


@mcp.tool(structured_output=True)
def validate_network_resource(
    resource_ref: DataRef,
) -> ValidationResult:
    """Validate a snapshot, route matrix, scenario result, comparison, or solution."""
    errors: list[str] = []
    warnings: list[str] = []
    checks: list[str] = []
    payload = _store().load(resource_ref)
    schema = str(payload.get("schema_version", "unknown"))
    model_by_schema = {
        "network_snapshot.v1": NetworkSnapshot,
        "route_matrix.v1": RouteMatrix,
        "network_scenario_result.v1": NetworkScenarioResult,
        "current_coverage_result.v1": CurrentCoverageResult,
        "scenario_comparison.v1": ScenarioComparison,
        "facility_location_solution.v1": FacilityLocationSolution,
    }
    model_type = model_by_schema.get(schema)
    if model_type is None:
        errors.append(f"unsupported resource schema {schema!r}")
    else:
        try:
            value = model_type.model_validate(payload)
            checks.append(f"resource conforms to {schema}")
            if isinstance(value, NetworkScenarioResult):
                allocation_total = sum(item.units for item in value.allocations)
                if allocation_total != value.metrics.total_demand_units:
                    errors.append(
                        "allocation units do not equal metrics.total_demand_units"
                    )
                covered = sum(
                    item.units for item in value.allocations if item.covered
                )
                if covered != value.metrics.covered_demand_units:
                    errors.append(
                        "covered allocation units do not equal metrics.covered_demand_units"
                    )
                expected_ratio = (
                    covered / value.metrics.total_demand_units
                    if value.metrics.total_demand_units
                    else 0
                )
                if abs(expected_ratio - value.metrics.coverage_ratio) > 1e-12:
                    errors.append("coverage_ratio is inconsistent with allocation units")
                checks.append("scenario allocation totals and coverage ratio are consistent")
                warnings.extend(value.issues)
            if isinstance(value, RouteMatrix):
                pairs = {
                    (entry.origin_facility_id, entry.destination_demand_id)
                    for entry in value.entries
                }
                if len(pairs) != len(value.entries):
                    errors.append("route matrix contains duplicate origin-destination pairs")
                checks.append("route matrix origin-destination pairs are unique")
            if isinstance(value, FacilityLocationSolution):
                if value.status == "optimal" and (
                    value.metrics.coverage_ratio + 1e-12
                    < value.target_coverage_ratio
                ):
                    errors.append("optimal solution does not reach its coverage target")
                checks.append("facility-location status is consistent with target coverage")
        except ValidationError as error:
            errors.extend(
                f"{'.'.join(str(part) for part in item['loc'])}: {item['msg']}"
                for item in error.errors()
            )
    return ValidationResult(
        valid=not errors,
        resource_schema=schema,
        errors=errors,
        warnings=warnings,
        checks=checks,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Supply-chain network planning MCP server")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path.cwd(),
        help="Plugin root used for default data and Resource directories",
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
        os.environ.get("SUPPLY_CHAIN_DATA_ROOT", _workspace_root)
    ).resolve()
    _profile_state_root = Path(
        os.environ.get("CODEX_HOME", _workspace_root / ".codex")
    ).resolve()
    resource_root = Path(
        os.environ.get(
            "SUPPLY_CHAIN_RESOURCE_DIR",
            _profile_state_root
            / "mcp-state"
            / "supply-chain-network-planner"
            / "resources",
        )
    ).resolve()
    _resource_store = ResourceStore(resource_root)
    mcp.run(transport=args.transport)


if __name__ == "__main__":
    main()
