"""Owner module for facility tools."""

import time
from typing import (
    Annotated,
    Literal,
)

from mcp.server.fastmcp import (
    Context,
)
from mcp.types import (
    CallToolResult,
    TextContent,
)
from open_web_codex_provider import (
    ResourceRef,
)
from pydantic import (
    Field,
)
from supply_chain_planner.network.matrix import (
    validate_route_matrix as _validate_route_matrix_model,
)
from supply_chain_planner.network.matrix_models import (
    CostMatrix,
    ExistingOnlyWarehouseScope,
    ExistingPlusCandidatesWarehouseScope,
)
from supply_chain_planner.network.matrix_models import (
    RouteMatrix as ComposableRouteMatrix,
)
from supply_chain_planner.network.models import (
    PlanningInputIdentity,
)
from supply_chain_planner.network.optimization_models import (
    CoverageMetricSummary,
    ExactOpeningPolicy,
    ExistingWarehousePolicy,
    MinimumFeasibleOpeningPolicy,
    OpeningPolicySelection,
    PMedianSolution,
    PMedianSolverStage,
    ScenarioResult,
    ScenarioSpec,
    ServiceCoverageConstraint,
    WarehouseChanges,
)
from supply_chain_planner.network.solver import (
    SolverStageOutcome,
    SolverUnavailable,
    compare_assignments,
    coverage_metrics,
    service_metrics,
    solve_assignment,
    solve_p_median_stage,
    summarize_assignment_cost,
)
from supply_chain_planner.shared.models import (
    ComparableNetworkResultRef,
    FacilityChangeAssessmentToolResult,
    FacilityChangeCostComparison,
    NetworkPlanComparisonResource,
    NetworkPlanComparisonResourceRef,
    NetworkScenarioResourceRef,
    PMedianSolutionToolResult,
    PreparedNetworkResource,
)
from supply_chain_planner.shared.planning_input import (
    require_matching_input,
)

from .tool_runtime import (
    BOUNDED_LOCAL_COMPUTE_TOOL,
    CONTENT_ADDRESSED_RESOURCE_TOOL,
    McpResourceContractError,
    _bounded_id_summary,
    _cost_metric_summary,
    _coverage_metric_summary,
    _load_comparable_resource,
    _load_ready_network,
    _runtime,
)


def _load_facility_scenario_inputs(
    prepared_input_relative_path: str,
    route_matrix_ref: ResourceRef,
    cost_matrix_ref: ResourceRef | None,
    scenario: ScenarioSpec,
    ctx: Context,
) -> tuple[
    PreparedNetworkResource,
    PlanningInputIdentity,
    ComposableRouteMatrix,
    CostMatrix | None,
    list[float],
    set[str],
    set[str],
]:
    """Load and validate immutable inputs shared by both scenario tools."""
    if (
        not scenario.service_targets
        or len(scenario.service_targets) > 32
        or any(target <= 0 for target in scenario.service_targets)
    ):
        raise McpResourceContractError("scenario_service_targets_invalid")
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
        raise McpResourceContractError("scenario_input_identity_mismatch") from error
    if scenario.objective == "min_cost" and costs is None:
        raise McpResourceContractError("min_cost_scenario_requires_cost_matrix")
    route_validation = _validate_route_matrix_model(
        prepared.demand_cities,
        prepared.warehouses,
        routes,
    )
    if not route_validation.valid:
        raise McpResourceContractError("scenario_route_matrix_incomplete")
    if costs is not None and (costs.missing_routes or not costs.stats.complete):
        raise McpResourceContractError("scenario_cost_matrix_incomplete")
    warehouse_by_id = {warehouse.warehouse_id: warehouse for warehouse in prepared.warehouses}
    add_ids = set(scenario.add_warehouse_ids)
    remove_ids = set(scenario.remove_warehouse_ids)
    for relocation in scenario.relocations:
        remove_ids.add(relocation.remove_warehouse_id)
        add_ids.add(relocation.add_warehouse_id)
    if len(add_ids) > 256 or len(remove_ids) > 256:
        raise McpResourceContractError("scenario_warehouse_change_limit_exceeded")
    unknown = (add_ids | remove_ids) - set(warehouse_by_id)
    if unknown:
        raise McpResourceContractError("scenario_unknown_warehouses")
    if any(warehouse_by_id[item].is_existing for item in add_ids):
        raise McpResourceContractError("scenario_add_requires_candidate_warehouse")
    if any(not warehouse_by_id[item].is_existing for item in remove_ids):
        raise McpResourceContractError("scenario_remove_requires_existing_warehouse")
    if add_ids & remove_ids:
        raise McpResourceContractError("scenario_add_remove_conflict")
    return (
        prepared,
        input_identity,
        routes,
        costs,
        sorted(set(scenario.service_targets)),
        add_ids,
        remove_ids,
    )


def _evaluate_facility_change(
    prepared: PreparedNetworkResource,
    input_identity: PlanningInputIdentity,
    routes: ComposableRouteMatrix,
    costs: CostMatrix | None,
    scenario: ScenarioSpec,
    base_active_ids: set[str],
    add_ids: set[str],
    remove_ids: set[str],
    ordered_targets: list[float],
) -> ScenarioResult:
    """Apply one validated change to an explicit active warehouse footprint."""
    active_ids = set(base_active_ids)
    active_ids.difference_update(remove_ids)
    active_ids.update(add_ids)
    invalid_upstreams = sorted(
        warehouse.warehouse_id
        for warehouse in prepared.warehouses
        if warehouse.warehouse_id in active_ids
        and warehouse.upstream_center_id is not None
        and warehouse.upstream_center_id not in active_ids
    )
    if invalid_upstreams:
        raise McpResourceContractError("scenario_active_upstream_required")
    assignment = solve_assignment(
        prepared.demand_cities,
        prepared.warehouses,
        routes,
        costs,
        scenario.objective,
        active_ids,
    )
    return ScenarioResult(
        active_warehouse_ids=sorted(active_ids),
        assignment=assignment,
        cost=summarize_assignment_cost(assignment, costs) if costs is not None else None,
        service=service_metrics(assignment, ordered_targets),
        warehouse_changes=WarehouseChanges(
            added=sorted(add_ids),
            removed=sorted(remove_ids),
        ),
        input_identity=input_identity,
    )


def assess_facility_change(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    route_matrix_ref: ResourceRef,
    before_ref: ComparableNetworkResultRef,
    scenario: ScenarioSpec,
    ctx: Context,
    cost_matrix_ref: ResourceRef | None = None,
) -> Annotated[CallToolResult, FacilityChangeAssessmentToolResult]:
    """Apply and compare one bounded facility change in a single call.

    Use this tool when a user adds, removes, or relocates facilities and the
    exact normalized input, route matrix, prior result, scenario, and optional
    cost matrix are already available. It returns both the changed scenario and
    its comparison without requiring a second model-selected Tool call.
    """
    prepared, input_identity, routes, costs, targets, add_ids, remove_ids = (
        _load_facility_scenario_inputs(
            prepared_input_relative_path,
            route_matrix_ref,
            cost_matrix_ref,
            scenario,
            ctx,
        )
    )
    before_view = _load_comparable_resource(before_ref)
    before_active_ids = set(before_view.active_warehouse_ids)
    try:
        require_matching_input(input_identity, before_view.input_identity)
    except ValueError as error:
        raise McpResourceContractError("scenario_before_input_identity_mismatch") from error
    known_warehouse_ids = {warehouse.warehouse_id for warehouse in prepared.warehouses}
    if before_active_ids - known_warehouse_ids:
        raise McpResourceContractError("scenario_before_warehouse_unknown")
    if remove_ids - before_active_ids:
        raise McpResourceContractError("scenario_remove_requires_active_warehouse")
    if add_ids & before_active_ids:
        raise McpResourceContractError("scenario_add_requires_inactive_warehouse")
    existing_ids = {
        warehouse.warehouse_id for warehouse in prepared.warehouses if warehouse.is_existing
    }
    scoped_candidate_ids = sorted((before_active_ids - existing_ids) | add_ids)
    expected_scope = (
        ExistingPlusCandidatesWarehouseScope(candidate_ids=scoped_candidate_ids)
        if scoped_candidate_ids
        else ExistingOnlyWarehouseScope()
    )
    expected_warehouse_ids = sorted(existing_ids | set(scoped_candidate_ids))
    if routes.warehouse_scope != expected_scope or routes.warehouse_ids != expected_warehouse_ids:
        raise McpResourceContractError("scenario_route_scope_mismatch")
    if costs is not None and (
        costs.warehouse_scope != expected_scope or costs.warehouse_ids != expected_warehouse_ids
    ):
        raise McpResourceContractError("scenario_cost_scope_mismatch")
    after = _evaluate_facility_change(
        prepared,
        input_identity,
        routes,
        costs,
        scenario,
        before_active_ids,
        add_ids,
        remove_ids,
        targets,
    )
    comparison = compare_assignments(
        before_view.assignment,
        after.assignment,
        targets,
        before_active_ids,
        set(after.active_warehouse_ids),
    )
    coverage_summary = (
        ", ".join(
            f"{metric.target_hours:g}h city-count "
            f"{metric.before.city_coverage_rate:.1%}→{metric.after.city_coverage_rate:.1%} "
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
    cost_complete = comparison.before_cost is not None and comparison.after_cost is not None
    cost_summary = (
        f"{comparison.before_cost:.2f}→{comparison.after_cost:.2f} "
        f"({comparison.cost_delta or 0:+.2f})"
        if cost_complete
        else "unavailable"
    )
    summary = (
        f"Assessed one facility change against the exact before result; active warehouses "
        f"{len(after.active_warehouse_ids)}, added [{_bounded_id_summary(sorted(add_ids))}], "
        f"removed [{_bounded_id_summary(sorted(remove_ids))}], affected "
        f"{len(comparison.affected_city_ids)}, reassigned "
        f"{len(comparison.reassigned_city_ids)}; cost {cost_summary}; coverage "
        f"{coverage_summary}."
    )

    scenario_published = _runtime().publish(after.schema_version, after, summary)
    if scenario_published.structuredContent is None:
        raise McpResourceContractError("facility_change_result_missing")
    scenario_ref = NetworkScenarioResourceRef.model_validate(
        scenario_published.structuredContent["resource_ref"]
    )
    plan_comparison = NetworkPlanComparisonResource(
        prepared_input_relative_path=prepared_input_relative_path,
        input_identity=input_identity,
        before_ref=before_ref,
        after_ref=ComparableNetworkResultRef.model_validate(scenario_ref.model_dump(mode="json")),
        comparison=comparison,
    )
    comparison_published = _runtime().publish(
        plan_comparison.schema_version,
        plan_comparison,
        summary,
    )
    if comparison_published.structuredContent is None:
        raise McpResourceContractError("facility_change_result_missing")
    affected_city_changes = [change for change in comparison.city_changes if change.affected]
    result = FacilityChangeAssessmentToolResult(
        summary=summary,
        scenario_ref=scenario_ref,
        plan_comparison_ref=NetworkPlanComparisonResourceRef.model_validate(
            comparison_published.structuredContent["resource_ref"]
        ),
        active_warehouse_count=len(after.active_warehouse_ids),
        added_warehouse_ids=sorted(add_ids),
        removed_warehouse_ids=sorted(remove_ids),
        cost=FacilityChangeCostComparison(
            currency=costs.currency if costs is not None else None,
            before_total=comparison.before_cost,
            after_total=comparison.after_cost,
            delta=comparison.cost_delta,
            complete=cost_complete,
        ),
        coverage=comparison.coverage,
        affected_city_count=len(comparison.affected_city_ids),
        affected_city_ids=comparison.affected_city_ids[:10],
        affected_city_ids_truncated=len(comparison.affected_city_ids) > 10,
        affected_city_changes=affected_city_changes[:10],
        affected_city_changes_truncated=len(affected_city_changes) > 10,
        reassigned_city_count=len(comparison.reassigned_city_ids),
        reassigned_city_ids=comparison.reassigned_city_ids[:10],
        reassigned_city_ids_truncated=len(comparison.reassigned_city_ids) > 10,
    )
    return CallToolResult(
        content=[
            TextContent(type="text", text=summary),
            *scenario_published.content[1:],
            *comparison_published.content[1:],
        ],
        structuredContent=result.model_dump(mode="json"),
    )


def _solver_stage_record(
    outcome: SolverStageOutcome,
    service_targets: list[float],
) -> PMedianSolverStage:
    coverage = (
        coverage_metrics(outcome.result.assignment, sorted(set(service_targets)))
        if outcome.result is not None
        else []
    )
    return PMedianSolverStage(
        kind=outcome.kind,
        status=outcome.status,
        optimality=outcome.optimality,
        selected_number_to_open=outcome.selected_number_to_open,
        objective_value=outcome.objective_value,
        best_bound=outcome.best_bound,
        coverage=coverage,
    )


def solve_p_median(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    route_matrix_ref: ResourceRef,
    cost_matrix_ref: ResourceRef,
    opening_policy: Annotated[
        OpeningPolicySelection,
        Field(
            description=(
                "Use exact with number_to_open for one candidate count, or "
                "minimum_feasible for one bounded minimum-openings stage followed by "
                "a fixed-count minimum-cost stage."
            )
        ),
    ],
    existing_warehouse_policy: Annotated[
        ExistingWarehousePolicy,
        Field(
            description=(
                "Explicit existing-site policy. Use keep_all_existing to keep every "
                "existing warehouse open. Use allow_closure only with the exact existing "
                "warehouse IDs the user authorized to close; all other existing "
                "warehouses remain fixed."
            )
        ),
    ],
    service_targets: Annotated[list[float], Field(min_length=1, max_length=32)],
    time_limit_seconds: Annotated[float, Field(gt=0, le=300)],
    ctx: Context,
    service_constraints: list[ServiceCoverageConstraint] | None = None,
) -> Annotated[CallToolResult, PMedianSolutionToolResult]:
    """Solve finite-candidate p-median with one bounded exact/search execution."""
    if any(target <= 0 for target in service_targets):
        raise McpResourceContractError("p_median_service_targets_invalid")
    prepared, input_identity = _load_ready_network(prepared_input_relative_path, ctx)
    routes = _runtime().load_model(
        route_matrix_ref,
        "route_matrix.v3",
        ComposableRouteMatrix,
    )
    costs = _runtime().load_model(
        cost_matrix_ref,
        "cost_matrix.v3",
        CostMatrix,
    )
    try:
        require_matching_input(input_identity, routes.input_identity)
        require_matching_input(input_identity, costs.input_identity)
    except ValueError as error:
        raise McpResourceContractError("p_median_input_identity_mismatch") from error
    route_validation = _validate_route_matrix_model(
        prepared.demand_cities,
        prepared.warehouses,
        routes,
    )
    if not route_validation.valid:
        raise McpResourceContractError("p_median_route_matrix_incomplete")
    all_warehouse_ids = sorted(warehouse.warehouse_id for warehouse in prepared.warehouses)
    if routes.warehouse_ids != all_warehouse_ids:
        raise McpResourceContractError("p_median_route_matrix_requires_all_warehouses_scope")
    if costs.warehouse_ids != all_warehouse_ids:
        raise McpResourceContractError("p_median_cost_matrix_requires_all_warehouses_scope")
    if costs.missing_routes:
        raise McpResourceContractError("p_median_cost_matrix_incomplete")
    constraints = [
        (constraint.target_hours, constraint.minimum_coverage)
        for constraint in service_constraints or []
    ]
    existing_ids = {
        warehouse.warehouse_id for warehouse in prepared.warehouses if warehouse.is_existing
    }
    if existing_warehouse_policy.mode == "keep_all_existing":
        fixed_existing_ids = existing_ids
        optional_existing_ids: set[str] = set()
    else:
        if len(existing_warehouse_policy.closable_existing_ids) != len(
            set(existing_warehouse_policy.closable_existing_ids)
        ):
            raise McpResourceContractError("existing_policy_closable_ids_duplicate")
        optional_existing_ids = set(existing_warehouse_policy.closable_existing_ids)
        fixed_existing_ids = existing_ids - optional_existing_ids
    candidate_count = sum(1 for warehouse in prepared.warehouses if not warehouse.is_existing)
    if isinstance(opening_policy, MinimumFeasibleOpeningPolicy):
        if not constraints:
            raise McpResourceContractError(
                "p_median_minimum_feasible_requires_service_constraints"
            )
        maximum = (
            candidate_count
            if opening_policy.maximum_number_to_open is None
            else opening_policy.maximum_number_to_open
        )
        if maximum > candidate_count or maximum > 64:
            raise McpResourceContractError("p_median_opening_policy_bound_invalid")
    started_at = time.monotonic()
    solver_stages: list[PMedianSolverStage] = []
    stage_outcomes: list[SolverStageOutcome] = []

    def remaining_seconds() -> float:
        return time_limit_seconds - (time.monotonic() - started_at)

    def run_stage(
        kind: Literal["minimum_openings", "minimum_cost"],
        number_to_open: int | None,
        maximum_number_to_open: int | None,
    ) -> SolverStageOutcome:
        def record(outcome: SolverStageOutcome) -> SolverStageOutcome:
            solver_stages.append(_solver_stage_record(outcome, service_targets))
            stage_outcomes.append(outcome)
            return outcome

        remaining = remaining_seconds()
        if remaining <= 0:
            return record(
                SolverStageOutcome(kind, "timeout", "not_available", None, None, None, None, 0)
            )
        try:
            outcome = solve_p_median_stage(
                prepared.demand_cities,
                prepared.warehouses,
                routes,
                costs,
                kind,
                number_to_open,
                maximum_number_to_open,
                fixed_existing_ids,
                optional_existing_ids,
                remaining,
                constraints,
            )
        except SolverUnavailable:
            return record(
                SolverStageOutcome(
                    kind, "unavailable", "not_available", None, None, None, None, 0
                )
            )
        return record(outcome)

    selected_solution = None
    selected_number_to_open: int | None = None
    minimum_number_to_open_proven = False
    final_best_bound: float | None = None
    if isinstance(opening_policy, ExactOpeningPolicy):
        stage = run_stage("minimum_cost", opening_policy.number_to_open, None)
        selected_solution = stage.result
        selected_number_to_open = stage.selected_number_to_open
        final_best_bound = stage.best_bound
    else:
        first_stage = run_stage("minimum_openings", None, maximum)
        if first_stage.result is not None:
            selected_solution = first_stage.result
            selected_number_to_open = first_stage.selected_number_to_open
            minimum_number_to_open_proven = first_stage.optimality == "proven"
            second_stage = run_stage("minimum_cost", selected_number_to_open, None)
            if second_stage.result is not None:
                selected_solution = second_stage.result
                selected_number_to_open = second_stage.selected_number_to_open
                final_best_bound = second_stage.best_bound

    if selected_solution is None:
        terminal_stage = stage_outcomes[-1] if stage_outcomes else None
        status = terminal_stage.status if terminal_stage is not None else "unavailable"
        solution = PMedianSolution(
            status=status,
            active_warehouse_ids=[],
            opened_candidate_ids=[],
            closed_existing_ids=[],
            optimality="not_available",
            opening_policy=opening_policy,
            selected_number_to_open=None,
            minimum_number_to_open_proven=False,
            solver_stages=solver_stages,
            message="No feasible p-median solution was found.",
            input_identity=input_identity,
        )
        solution_coverage: list[CoverageMetricSummary] = []
    else:
        stage_statuses = [stage.status for stage in stage_outcomes]
        all_optimal = all(status == "optimal" for status in stage_statuses)
        proven = (
            all_optimal
            and (isinstance(opening_policy, ExactOpeningPolicy) or minimum_number_to_open_proven)
        )
        solution = PMedianSolution(
            status="optimal" if proven else "feasible",
            active_warehouse_ids=selected_solution.active_warehouse_ids,
            opened_candidate_ids=selected_solution.opened_candidate_ids,
            closed_existing_ids=selected_solution.closed_existing_ids,
            assignment=selected_solution.assignment,
            objective_value=selected_solution.objective_value,
            cost=summarize_assignment_cost(selected_solution.assignment, costs),
            service=service_metrics(selected_solution.assignment, sorted(set(service_targets))),
            optimality="proven" if proven else "feasible_only",
            opening_policy=opening_policy,
            selected_number_to_open=selected_number_to_open,
            minimum_number_to_open_proven=minimum_number_to_open_proven,
            solver_stages=solver_stages,
            best_bound=final_best_bound,
            message=(
                None
                if proven
                else "A feasible solution was retained; one or more solver stages were not proven optimal."
            ),
            input_identity=input_identity,
        )
        solution_coverage = coverage_metrics(selected_solution.assignment, sorted(set(service_targets)))
    summary = (
        f"p-median status is {solution.status}; active warehouses "
        f"{len(solution.active_warehouse_ids)}, opened "
        f"[{_bounded_id_summary(solution.opened_candidate_ids)}], closed "
        f"[{_bounded_id_summary(solution.closed_existing_ids)}]; cost "
        f"{_cost_metric_summary(solution.cost)}, coverage "
        f"{_coverage_metric_summary(solution_coverage) if solution.assignment is not None else 'none'}."
    )
    published = _runtime().publish(
        solution.schema_version,
        solution,
        summary,
    )
    if published.structuredContent is None:
        raise McpResourceContractError("p_median_result_missing")
    resource_ref = ResourceRef.model_validate(published.structuredContent["resource_ref"])
    result = PMedianSolutionToolResult(
        summary=summary,
        resource_ref=resource_ref,
        input_identity=input_identity,
        status=solution.status,
        optimality=solution.optimality,
        opening_policy=solution.opening_policy,
        selected_number_to_open=solution.selected_number_to_open,
        minimum_number_to_open_proven=solution.minimum_number_to_open_proven,
        solver_stages=solution.solver_stages,
        active_warehouse_count=len(solution.active_warehouse_ids),
        opened_candidate_ids=solution.opened_candidate_ids,
        closed_existing_ids=solution.closed_existing_ids,
        cost=solution.cost,
        coverage=solution_coverage,
    )
    published.structuredContent = result.model_dump(mode="json")
    return published


def register_tools(mcp, *, phase: str = "all") -> None:
    if phase in ("all", "all"):
        mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(assess_facility_change)
    if phase in ("all", "all"):
        mcp.tool(structured_output=True, annotations=BOUNDED_LOCAL_COMPUTE_TOOL)(solve_p_median)
