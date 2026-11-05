"""Owner module for analysis tools."""

from typing import (
    Annotated,
    Literal,
)

from mcp.server.fastmcp import (
    Context,
)
from mcp.types import (
    CallToolResult,
)
from open_web_codex_provider import (
    ResourceRef,
)
from pydantic import (
    Field,
    ValidationError,
)
from supply_chain_planner.network.matrix import (
    validate_route_matrix as _validate_route_matrix_model,
)
from supply_chain_planner.network.matrix_models import (
    CostMatrix,
)
from supply_chain_planner.network.matrix_models import (
    RouteMatrix as ComposableRouteMatrix,
)
from supply_chain_planner.network.models import (
    DemandCityRecord,
    WarehouseRecord,
)
from supply_chain_planner.network.optimization_models import (
    AssignmentComparison,
    AssignmentResult,
    BaselineResult,
    CoverageMetricSummary,
)
from supply_chain_planner.network.solver import (
    compare_assignments,
    coverage_metrics,
    service_metrics,
    solve_assignment,
    solve_current_assignment,
    summarize_assignment_cost,
)
from supply_chain_planner.shared.models import (
    ComparableNetworkResultRef,
    NetworkBaselineResourceToolResult,
    NetworkPlanComparisonResource,
    UncoveredCitySummary,
)
from supply_chain_planner.shared.planning_input import (
    require_matching_input,
)

from .tool_runtime import (
    CONTENT_ADDRESSED_RESOURCE_TOOL,
    McpResourceContractError,
    _bounded_id_summary,
    _cost_metric_summary,
    _coverage_metric_summary,
    _load_comparable_resource,
    _load_ready_network,
    _runtime,
)


def _baseline_coverage_projection(
    assignment: AssignmentResult,
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    targets: list[float],
) -> tuple[list[CoverageMetricSummary], float, list[UncoveredCitySummary]]:
    coverage = coverage_metrics(assignment, targets)
    detail_target = max(targets)
    demand_by_id = {item.city_id: item for item in demand_cities}
    warehouse_by_id = {item.warehouse_id: item for item in warehouses}
    uncovered: list[UncoveredCitySummary] = []
    for row in sorted(assignment.rows, key=lambda item: item.demand_city_id):
        if row.duration_hours is not None and row.duration_hours <= detail_target:
            continue
        demand = demand_by_id[row.demand_city_id]
        warehouse = warehouse_by_id.get(row.warehouse_id) if row.warehouse_id else None
        uncovered.append(
            UncoveredCitySummary(
                demand_city_id=row.demand_city_id,
                demand_city_name=demand.city_name,
                warehouse_id=row.warehouse_id,
                warehouse_name=warehouse.warehouse_name if warehouse else None,
                duration_hours=row.duration_hours,
                demand_quantity=row.demand_quantity,
            )
        )
    return coverage, detail_target, uncovered


def compare_network_scenarios(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    before_ref: ComparableNetworkResultRef,
    after_ref: ComparableNetworkResultRef,
    service_targets: Annotated[list[float], Field(min_length=1, max_length=32)],
    ctx: Context,
) -> CallToolResult:
    """Compare two typed network results in either direction."""
    _prepared, input_identity = _load_ready_network(prepared_input_relative_path, ctx)
    try:
        before_ref = ComparableNetworkResultRef.model_validate(before_ref.model_dump(mode="json"))
        after_ref = ComparableNetworkResultRef.model_validate(after_ref.model_dump(mode="json"))
    except ValidationError as error:
        raise McpResourceContractError("comparison_subject_schema_invalid") from error
    if any(target <= 0 for target in service_targets):
        raise McpResourceContractError("comparison_service_targets_invalid")
    before_view = _load_comparable_resource(before_ref)
    after_view = _load_comparable_resource(after_ref)
    try:
        require_matching_input(input_identity, before_view.input_identity)
        require_matching_input(input_identity, after_view.input_identity)
    except ValueError as error:
        raise McpResourceContractError("comparison_input_identity_mismatch") from error
    comparison: AssignmentComparison = compare_assignments(
        before_view.assignment,
        after_view.assignment,
        service_targets,
        set(before_view.active_warehouse_ids),
        set(after_view.active_warehouse_ids),
    )
    coverage_summary = (
        ", ".join(
            f"{metric.target_hours:g}h city-count "
            f"{metric.before.city_coverage_rate:.1%}→"
            f"{metric.after.city_coverage_rate:.1%} "
            f"({metric.delta.city_coverage_rate:+.1%}), demand-weighted "
            f"{metric.before.demand_weighted_coverage_rate:.1%}→"
            f"{metric.after.demand_weighted_coverage_rate:.1%} "
            f"({metric.delta.demand_weighted_coverage_rate:+.1%})"
            for metric in comparison.coverage[:8]
        )
        or "none"
    )
    if len(comparison.coverage) > 8:
        coverage_summary = f"{coverage_summary}, +{len(comparison.coverage) - 8} more"
    cost_summary = (
        "unavailable"
        if comparison.before_cost is None or comparison.after_cost is None
        else f"{comparison.before_cost:.2f}→{comparison.after_cost:.2f} "
        f"({comparison.cost_delta or 0:+.2f})"
    )
    plan_comparison = NetworkPlanComparisonResource(
        prepared_input_relative_path=prepared_input_relative_path,
        input_identity=input_identity,
        before_ref=before_ref,
        after_ref=after_ref,
        comparison=comparison,
    )
    return _runtime().publish(
        plan_comparison.schema_version,
        plan_comparison,
        f"Compared {len(comparison.city_changes)} city assignments; selected "
        f"[{_bounded_id_summary(comparison.selected_warehouse_ids)}], removed "
        f"[{_bounded_id_summary(comparison.removed_warehouse_ids)}], affected "
        f"{len(comparison.affected_city_ids)}, reassigned "
        f"{len(comparison.reassigned_city_ids)}; cost {cost_summary}; coverage "
        f"{coverage_summary}.",
    )


def evaluate_network_baseline(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    route_matrix_ref: ResourceRef,
    objective: Literal["min_time", "min_cost"],
    service_targets: Annotated[list[float], Field(min_length=1, max_length=32)],
    ctx: Context,
    coverage_mode: Literal["auto", "actual_current", "optimized_existing_footprint"] = "auto",
    cost_matrix_ref: ResourceRef | None = None,
) -> Annotated[CallToolResult, NetworkBaselineResourceToolResult]:
    """Evaluate an actual or optimized-existing baseline.

    Pass the same route and cost matrices the follow-up analysis uses, in any
    warehouse scope; only existing warehouses are evaluated. The default auto mode selects actual_current only when the prepared input
    contains current assignments; otherwise it selects the optimized existing
    footprint without making the model inspect or guess input contents.
    """
    if any(target <= 0 for target in service_targets):
        raise McpResourceContractError("baseline_service_targets_invalid")
    prepared, input_identity = _load_ready_network(prepared_input_relative_path, ctx)
    routes = _runtime().load_model(
        route_matrix_ref,
        "route_matrix.v3",
        ComposableRouteMatrix,
    )
    costs = (
        _runtime().load_model(cost_matrix_ref, "cost_matrix.v3", CostMatrix)
        if cost_matrix_ref is not None
        else None
    )
    try:
        require_matching_input(input_identity, routes.input_identity)
        if costs is not None:
            require_matching_input(input_identity, costs.input_identity)
    except ValueError as error:
        raise McpResourceContractError("baseline_input_identity_mismatch") from error
    existing_ids = sorted(
        warehouse.warehouse_id for warehouse in prepared.warehouses if warehouse.is_existing
    )
    # Every warehouse scope contains all existing warehouses and a baseline reads
    # only their lanes, so it reuses the exact matrices of the follow-up siting or
    # facility-change run; baseline and plan then share identical lane facts.
    if costs is not None and (
        costs.warehouse_scope != routes.warehouse_scope
        or costs.warehouse_ids != routes.warehouse_ids
    ):
        raise McpResourceContractError("baseline_cost_scope_mismatch")
    route_validation = _validate_route_matrix_model(
        prepared.demand_cities,
        prepared.warehouses,
        routes,
    )
    if not route_validation.valid:
        raise McpResourceContractError("baseline_route_matrix_incomplete")
    if objective == "min_cost" and costs is None:
        raise McpResourceContractError("min_cost_baseline_requires_cost_matrix")
    if objective == "min_cost" and costs is not None and (
        costs.missing_routes or not costs.stats.complete
    ):
        raise McpResourceContractError("baseline_cost_matrix_incomplete")
    active_ids = set(existing_ids)
    resolved_coverage_mode = (
        "actual_current"
        if coverage_mode == "auto" and prepared.current_assignments
        else "optimized_existing_footprint"
        if coverage_mode == "auto"
        else coverage_mode
    )
    if resolved_coverage_mode == "actual_current":
        if not prepared.current_assignments:
            raise McpResourceContractError("current_assignments_required")
        assignment = solve_current_assignment(
            prepared.demand_cities,
            prepared.warehouses,
            prepared.current_assignments,
            routes,
            costs,
            objective,
        )
        label: Literal["actual_current", "optimized_existing_footprint"] = "actual_current"
    else:
        assignment = solve_assignment(
            prepared.demand_cities,
            prepared.warehouses,
            routes,
            costs,
            objective,
            active_ids,
        )
        label = "optimized_existing_footprint"
    ordered_targets = sorted(set(service_targets))
    coverage, detail_target, uncovered = _baseline_coverage_projection(
        assignment,
        prepared.demand_cities,
        prepared.warehouses,
        ordered_targets,
    )
    baseline = BaselineResult(
        label=label,
        active_warehouse_ids=sorted(active_ids),
        assignment=assignment,
        service=service_metrics(assignment, ordered_targets),
        coverage=coverage,
        cost=summarize_assignment_cost(assignment, costs) if costs is not None else None,
        notice_code=None,
        input_identity=input_identity,
    )
    label_text = (
        "actual current assignment"
        if label == "actual_current"
        else "optimized existing-warehouse footprint"
    )
    message = (
        f"Evaluated the {label_text}; active warehouses "
        f"{len(baseline.active_warehouse_ids)}, cost "
        f"{_cost_metric_summary(baseline.cost)}, service "
        f"{_coverage_metric_summary(coverage)}; uncovered at "
        f"{detail_target:g}h: {len(uncovered)} cities."
    )
    result = _runtime().publish(baseline.schema_version, baseline, message)
    if result.structuredContent is None:
        raise McpResourceContractError("baseline_result_missing")
    result.structuredContent.update(
        {
            "coverage_metrics": [metric.model_dump(mode="json") for metric in coverage],
            "detail_target_hours": detail_target,
            "uncovered_city_count": len(uncovered),
            "uncovered_cities": [item.model_dump(mode="json") for item in uncovered[:10]],
            "uncovered_cities_truncated": len(uncovered) > 10,
        }
    )
    return result


def register_tools(mcp, *, phase: str = "all") -> None:
    if phase in ("all", "compare"):
        mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(compare_network_scenarios)
    if phase in ("all", "baseline"):
        mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(evaluate_network_baseline)
