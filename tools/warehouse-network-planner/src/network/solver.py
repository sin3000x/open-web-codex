"""Small deterministic assignment and finite-candidate facility solvers."""

from __future__ import annotations

import math
from collections import defaultdict
from collections.abc import Iterable
from dataclasses import dataclass
from functools import lru_cache
from typing import Any, Literal

from supply_chain_planner.network.matrix_models import CostMatrix, RouteMatrix
from supply_chain_planner.network.models import (
    CurrentAssignmentRecord,
    DemandCityRecord,
    WarehouseRecord,
)
from supply_chain_planner.network.optimization_models import (
    AssignmentComparison,
    AssignmentObjective,
    AssignmentResult,
    AssignmentRow,
    CityAssignmentChange,
    CostSummary,
    CoverageComparison,
    CoverageMetricDelta,
    CoverageMetricSummary,
    FacilityLocationResult,
    ServiceMetric,
)


class SolverUnavailable(RuntimeError):
    """The declared deterministic solver dependency is not installed."""


@dataclass(frozen=True)
class SolverStageOutcome:
    kind: Literal["minimum_openings", "minimum_cost"]
    status: Literal["optimal", "feasible", "timeout", "infeasible", "unavailable"]
    optimality: Literal["proven", "feasible_only", "not_available"]
    result: FacilityLocationResult | None
    selected_number_to_open: int | None
    objective_value: float | None
    best_bound: float | None
    branches: int


@lru_cache(maxsize=1)
def _load_cp_model():
    """Load the native solver only when a facility-location solve needs it."""

    try:
        from ortools.sat.python import cp_model
    except ImportError:  # pragma: no cover - exercised by the explicit unavailable path
        return None
    return cp_model


def preload_solver() -> bool:
    """Load the native solver ahead of a solve and report whether it is available.

    The first load of freshly materialized OR-Tools native libraries can take
    minutes on some hosts, so callers load it outside any solver time budget.
    """

    return _load_cp_model() is not None


def _route_index(matrix: RouteMatrix):
    return {
        (row.origin_id, row.destination_id, row.layer): row
        for row in matrix.rows
        if row.status == "ready"
    }


def _cost_index(matrix: CostMatrix):
    return {
        (row.origin_id, row.destination_id, row.layer): row.cost_per_demand_unit
        for row in matrix.rows
    }


def _total_unit_cost(
    costs: dict[tuple[str, str, str], float],
    warehouse: WarehouseRecord,
    demand_city_id: str,
    upstream_center_id: str | None = None,
) -> float | None:
    last_mile = costs.get((warehouse.warehouse_id, demand_city_id, "last_mile"))
    if last_mile is None:
        return None
    if warehouse.warehouse_type != "cross_docking":
        return last_mile
    upstream = upstream_center_id or warehouse.upstream_center_id
    if upstream is None:
        return None
    linehaul = costs.get((upstream, warehouse.warehouse_id, "linehaul"))
    return None if linehaul is None else last_mile + linehaul


def _validate_warehouse_upstreams(warehouses: Iterable[WarehouseRecord]) -> None:
    rows = list(warehouses)
    by_id = {warehouse.warehouse_id: warehouse for warehouse in rows}
    for warehouse in rows:
        if warehouse.warehouse_type != "cross_docking":
            continue
        if warehouse.upstream_center_id is None:
            raise ValueError(f"warehouse_upstream_center_required:{warehouse.warehouse_id}")
        upstream = by_id.get(warehouse.upstream_center_id)
        if upstream is None:
            raise ValueError(f"warehouse_upstream_center_missing:{warehouse.warehouse_id}")
        if upstream.warehouse_type != "center":
            raise ValueError(f"warehouse_upstream_must_be_center:{warehouse.warehouse_id}")


def solve_assignment(
    demand: Iterable[DemandCityRecord],
    warehouses: Iterable[WarehouseRecord],
    route_matrix: RouteMatrix,
    cost_matrix: CostMatrix | None,
    objective: AssignmentObjective,
    active_warehouse_ids: set[str],
) -> AssignmentResult:
    demand_rows = sorted(demand, key=lambda row: row.city_id)
    warehouse_rows = sorted(
        (warehouse for warehouse in warehouses if warehouse.warehouse_id in active_warehouse_ids),
        key=lambda row: row.warehouse_id,
    )
    if not warehouse_rows:
        raise ValueError("assignment_requires_active_warehouse")
    _validate_warehouse_upstreams(warehouse_rows)
    routes = _route_index(route_matrix)
    costs = _cost_index(cost_matrix) if cost_matrix is not None else {}
    rows: list[AssignmentRow] = []
    unassigned = 0
    for city in demand_rows:
        choices = []
        for warehouse in warehouse_rows:
            if (
                warehouse.upstream_center_id
                and warehouse.upstream_center_id not in active_warehouse_ids
            ):
                continue
            route = routes.get((warehouse.warehouse_id, city.city_id, "last_mile"))
            if route is None or route.status != "ready":
                continue
            cost = _total_unit_cost(costs, warehouse, city.city_id)
            if objective == "min_cost" and cost is None:
                continue
            choices.append(
                (
                    cost if objective == "min_cost" else route.duration_hours,
                    route.duration_hours,
                    warehouse.warehouse_id,
                    route.distance_km,
                    cost,
                )
            )
        if not choices:
            rows.append(
                AssignmentRow(
                    demand_city_id=city.city_id,
                    warehouse_id=None,
                    demand_quantity=city.demand_quantity,
                    reason="no_ready_route_or_cost",
                )
            )
            unassigned += city.demand_quantity
            continue
        _, duration, warehouse_id, distance, cost = min(choices)
        rows.append(
            AssignmentRow(
                demand_city_id=city.city_id,
                warehouse_id=warehouse_id,
                upstream_center_id=next(
                    warehouse.upstream_center_id
                    for warehouse in warehouse_rows
                    if warehouse.warehouse_id == warehouse_id
                ),
                demand_quantity=city.demand_quantity,
                distance_km=distance,
                duration_hours=duration,
                cost=cost,
            )
        )
    total = sum(city.demand_quantity for city in demand_rows)
    return AssignmentResult(
        objective=objective,
        rows=rows,
        total_demand=total,
        unassigned_demand=unassigned,
    )


def solve_current_assignment(
    demand: Iterable[DemandCityRecord],
    warehouses: Iterable[WarehouseRecord],
    current_assignments: Iterable[CurrentAssignmentRecord],
    route_matrix: RouteMatrix,
    cost_matrix: CostMatrix | None,
    objective: AssignmentObjective,
) -> AssignmentResult:
    """Evaluate the recorded assignment without silently re-optimizing it."""
    demand_rows = sorted(demand, key=lambda row: row.city_id)
    warehouses_by_id = {warehouse.warehouse_id: warehouse for warehouse in warehouses}
    _validate_warehouse_upstreams(warehouses_by_id.values())
    assignments_by_city = {
        assignment.demand_city_id: assignment for assignment in current_assignments
    }
    routes = _route_index(route_matrix)
    costs = _cost_index(cost_matrix) if cost_matrix is not None else {}
    rows: list[AssignmentRow] = []
    unassigned = 0
    for city in demand_rows:
        current = assignments_by_city.get(city.city_id)
        warehouse = warehouses_by_id.get(current.serving_warehouse_id) if current else None
        route = (
            routes.get((warehouse.warehouse_id, city.city_id, "last_mile"))
            if warehouse is not None
            else None
        )
        current_upstream = current.upstream_center_id if current else None
        if (
            warehouse is not None
            and current_upstream is not None
            and current_upstream != warehouse.upstream_center_id
        ):
            raise ValueError(
                f"current_assignment_upstream_mismatch:{city.city_id}:"
                f"{current_upstream}:{warehouse.upstream_center_id}"
            )
        cost = (
            _total_unit_cost(costs, warehouse, city.city_id, warehouse.upstream_center_id)
            if warehouse is not None
            else None
        )
        if current is None:
            reason = "current_assignment_missing"
        elif warehouse is None:
            reason = "current_warehouse_missing"
        elif route is None or route.status != "ready":
            reason = "current_route_missing"
        elif objective == "min_cost" and cost is None:
            reason = "current_cost_missing"
        else:
            rows.append(
                AssignmentRow(
                    demand_city_id=city.city_id,
                    warehouse_id=warehouse.warehouse_id,
                    upstream_center_id=warehouse.upstream_center_id,
                    demand_quantity=city.demand_quantity,
                    distance_km=route.distance_km,
                    duration_hours=route.duration_hours,
                    cost=cost,
                )
            )
            continue
        rows.append(
            AssignmentRow(
                demand_city_id=city.city_id,
                warehouse_id=warehouse.warehouse_id if warehouse is not None else None,
                upstream_center_id=(
                    warehouse.upstream_center_id if warehouse is not None else None
                ),
                demand_quantity=city.demand_quantity,
                reason=reason,
            )
        )
        unassigned += city.demand_quantity
    return AssignmentResult(
        objective=objective,
        rows=rows,
        total_demand=sum(city.demand_quantity for city in demand_rows),
        unassigned_demand=unassigned,
    )


def service_metrics(
    assignment: AssignmentResult,
    targets: Iterable[float],
) -> list[ServiceMetric]:
    total = assignment.total_demand
    return [
        ServiceMetric(
            target_hours=target,
            covered_demand=sum(
                row.demand_quantity
                for row in assignment.rows
                if row.duration_hours is not None and row.duration_hours <= target
            ),
            total_demand=total,
            coverage_rate=(
                sum(
                    row.demand_quantity
                    for row in assignment.rows
                    if row.duration_hours is not None and row.duration_hours <= target
                )
                / total
                if total
                else 0
            ),
        )
        for target in targets
    ]


def coverage_metrics(
    assignment: AssignmentResult,
    targets: Iterable[float],
) -> list[CoverageMetricSummary]:
    """Compute both supported service-coverage denominators from one assignment."""

    target_values = list(targets)
    total_city_count = len(assignment.rows)
    demand_by_target = {
        metric.target_hours: metric for metric in service_metrics(assignment, target_values)
    }
    result: list[CoverageMetricSummary] = []
    for target in target_values:
        covered_city_count = sum(
            row.duration_hours is not None and row.duration_hours <= target
            for row in assignment.rows
        )
        result.append(
            CoverageMetricSummary(
                target_hours=target,
                covered_city_count=covered_city_count,
                total_city_count=total_city_count,
                city_coverage_rate=(
                    covered_city_count / total_city_count if total_city_count else 0
                ),
                covered_demand=demand_by_target[target].covered_demand,
                total_demand=demand_by_target[target].total_demand,
                demand_weighted_coverage_rate=demand_by_target[target].coverage_rate,
            )
        )
    return result


def compare_assignments(
    before: AssignmentResult,
    after: AssignmentResult,
    requested_service_targets: Iterable[float],
    before_active_warehouse_ids: set[str],
    after_active_warehouse_ids: set[str],
) -> AssignmentComparison:
    """Compare two complete or partial assignments without workflow state."""

    targets = sorted(set(requested_service_targets))
    if not targets or any(target <= 0 for target in targets):
        raise ValueError("comparison_requires_positive_service_targets")
    before_city_ids = [row.demand_city_id for row in before.rows]
    after_city_ids = [row.demand_city_id for row in after.rows]
    if len(before_city_ids) != len(set(before_city_ids)) or len(after_city_ids) != len(
        set(after_city_ids)
    ):
        raise ValueError("comparison_assignment_city_duplicate")
    before_quantities = {row.demand_city_id: row.demand_quantity for row in before.rows}
    after_quantities = {row.demand_city_id: row.demand_quantity for row in after.rows}
    if before_quantities != after_quantities or before.total_demand != after.total_demand:
        raise ValueError("comparison_assignment_domain_mismatch")
    before_coverage = {metric.target_hours: metric for metric in coverage_metrics(before, targets)}
    after_coverage = {metric.target_hours: metric for metric in coverage_metrics(after, targets)}
    before_rows = {row.demand_city_id: row for row in before.rows}
    after_rows = {row.demand_city_id: row for row in after.rows}
    changes: list[CityAssignmentChange] = []
    for city_id in sorted(set(before_rows) | set(after_rows)):
        before_row = before_rows.get(city_id)
        after_row = after_rows.get(city_id)
        reassigned = (
            before_row is not None
            and after_row is not None
            and before_row.warehouse_id != after_row.warehouse_id
        )
        affected = before_row != after_row
        changes.append(
            CityAssignmentChange(
                demand_city_id=city_id,
                before_warehouse_id=(before_row.warehouse_id if before_row else None),
                after_warehouse_id=(after_row.warehouse_id if after_row else None),
                affected=affected,
                reassigned=reassigned,
                duration_hours_delta=_optional_delta(
                    before_row.duration_hours if before_row else None,
                    after_row.duration_hours if after_row else None,
                ),
                cost_per_unit_delta=_optional_delta(
                    before_row.cost if before_row else None,
                    after_row.cost if after_row else None,
                ),
            )
        )
    before_cost = _complete_assignment_cost(before)
    after_cost = _complete_assignment_cost(after)
    return AssignmentComparison(
        requested_service_targets=targets,
        coverage=[
            CoverageComparison(
                target_hours=target,
                before=before_coverage[target],
                after=after_coverage[target],
                delta=CoverageMetricDelta(
                    covered_city_count=(
                        after_coverage[target].covered_city_count
                        - before_coverage[target].covered_city_count
                    ),
                    total_city_count=(
                        after_coverage[target].total_city_count
                        - before_coverage[target].total_city_count
                    ),
                    city_coverage_rate=(
                        after_coverage[target].city_coverage_rate
                        - before_coverage[target].city_coverage_rate
                    ),
                    covered_demand=(
                        after_coverage[target].covered_demand
                        - before_coverage[target].covered_demand
                    ),
                    total_demand=(
                        after_coverage[target].total_demand - before_coverage[target].total_demand
                    ),
                    demand_weighted_coverage_rate=(
                        after_coverage[target].demand_weighted_coverage_rate
                        - before_coverage[target].demand_weighted_coverage_rate
                    ),
                ),
            )
            for target in targets
        ],
        before_cost=before_cost,
        after_cost=after_cost,
        cost_delta=(
            after_cost - before_cost if before_cost is not None and after_cost is not None else None
        ),
        selected_warehouse_ids=sorted(after_active_warehouse_ids - before_active_warehouse_ids),
        removed_warehouse_ids=sorted(before_active_warehouse_ids - after_active_warehouse_ids),
        affected_city_ids=[item.demand_city_id for item in changes if item.affected],
        reassigned_city_ids=[item.demand_city_id for item in changes if item.reassigned],
        city_changes=changes,
    )


def _complete_assignment_cost(assignment: AssignmentResult) -> float | None:
    if assignment.unassigned_demand or any(
        row.warehouse_id is not None and row.cost is None for row in assignment.rows
    ):
        return None
    return sum(
        float(row.demand_quantity) * float(row.cost)
        for row in assignment.rows
        if row.warehouse_id is not None and row.cost is not None
    )


def _optional_delta(before: float | None, after: float | None) -> float | None:
    return after - before if before is not None and after is not None else None


def _assignment_objective_value(assignment: AssignmentResult) -> float:
    return sum(
        (
            row.cost
            if assignment.objective == "min_cost" and row.cost is not None
            else row.duration_hours or 0
        )
        * float(row.demand_quantity)
        for row in assignment.rows
        if row.warehouse_id is not None
    )


def solve_p_median_stage(
    demand: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    route_matrix: RouteMatrix,
    cost_matrix: CostMatrix,
    stage_kind: Literal["minimum_openings", "minimum_cost"],
    number_to_open: int | None,
    maximum_number_to_open: int | None,
    fixed_existing_ids: set[str],
    optional_existing_ids: set[str],
    time_limit_seconds: float,
    service_constraints: list[tuple[float, float]] | None = None,
):
    cp_model = _load_cp_model()
    if cp_model is None:
        raise SolverUnavailable("ortools is not installed")
    if time_limit_seconds <= 0 or (number_to_open is not None and number_to_open < 0):
        raise ValueError("invalid_solver_parameters")
    if stage_kind == "minimum_openings" and (
        maximum_number_to_open is None or maximum_number_to_open < 0
    ):
        raise ValueError("minimum_openings_bound_required")
    if stage_kind == "minimum_cost" and number_to_open is None:
        raise ValueError("minimum_cost_number_required")

    demand_rows = sorted(demand, key=lambda row: row.city_id)
    warehouse_rows = sorted(warehouses, key=lambda row: row.warehouse_id)
    _validate_warehouse_upstreams(warehouse_rows)
    warehouse_ids = {warehouse.warehouse_id for warehouse in warehouse_rows}
    existing_ids = {warehouse.warehouse_id for warehouse in warehouse_rows if warehouse.is_existing}
    policy_ids = fixed_existing_ids | optional_existing_ids
    unknown_policy_ids = policy_ids - warehouse_ids
    if unknown_policy_ids:
        raise ValueError(
            "existing_policy_unknown_warehouses:" + ",".join(sorted(unknown_policy_ids))
        )
    non_existing_policy_ids = policy_ids - existing_ids
    if non_existing_policy_ids:
        raise ValueError(
            "existing_policy_requires_existing_warehouses:"
            + ",".join(sorted(non_existing_policy_ids))
        )
    overlap = fixed_existing_ids & optional_existing_ids
    if overlap:
        raise ValueError("existing_policy_overlap:" + ",".join(sorted(overlap)))
    if policy_ids != existing_ids:
        missing = existing_ids - policy_ids
        raise ValueError("existing_policy_incomplete:" + ",".join(sorted(missing)))
    fixed = set(fixed_existing_ids)
    candidates = [
        warehouse
        for warehouse in warehouse_rows
        if not warehouse.is_existing and warehouse.warehouse_id not in fixed
    ]
    if stage_kind == "minimum_cost" and number_to_open > len(candidates):
        return SolverStageOutcome(
            stage_kind, "infeasible", "proven", None, None, None, None, 0
        )
    if stage_kind == "minimum_openings" and maximum_number_to_open > len(candidates):
        maximum_number_to_open = len(candidates)

    routes = _route_index(route_matrix)
    costs = _cost_index(cost_matrix)
    model = cp_model.CpModel()
    open_variables: dict[str, Any] = {}
    for warehouse in warehouse_rows:
        warehouse_id = warehouse.warehouse_id
        variable = model.NewBoolVar(f"open_{warehouse_id}")
        open_variables[warehouse_id] = variable
        if warehouse_id in fixed:
            model.Add(variable == 1)
    for warehouse in warehouse_rows:
        if warehouse.upstream_center_id:
            upstream = open_variables.get(warehouse.upstream_center_id)
            if upstream is None:
                raise ValueError(f"warehouse_upstream_center_missing:{warehouse.warehouse_id}")
            model.Add(open_variables[warehouse.warehouse_id] <= upstream)
    candidate_open_count = sum(
        open_variables[warehouse.warehouse_id] for warehouse in candidates
    )
    if stage_kind == "minimum_openings":
        model.Add(candidate_open_count <= maximum_number_to_open)
    else:
        model.Add(candidate_open_count == number_to_open)

    assignment_variables: dict[tuple[str, str], Any] = {}
    objective_terms = []
    for city in demand_rows:
        choices = []
        for warehouse in warehouse_rows:
            route = routes.get((warehouse.warehouse_id, city.city_id, "last_mile"))
            cost = _total_unit_cost(costs, warehouse, city.city_id)
            if route is None or route.status != "ready" or cost is None:
                continue
            variable = model.NewBoolVar(f"assign_{city.city_id}_{warehouse.warehouse_id}")
            assignment_variables[(city.city_id, warehouse.warehouse_id)] = variable
            model.Add(variable <= open_variables[warehouse.warehouse_id])
            objective_terms.append(int(round(cost * 1000 * float(city.demand_quantity))) * variable)
            choices.append((warehouse, route))
        if not choices:
            return SolverStageOutcome(
                stage_kind, "infeasible", "proven", None, None, None, None, 0
            )
        model.Add(
            sum(
                assignment_variables[(city.city_id, warehouse.warehouse_id)]
                for warehouse, _ in choices
            )
            == 1
        )

    for target_hours, minimum_coverage in service_constraints or []:
        if not 0 < minimum_coverage <= 1 or target_hours <= 0:
            raise ValueError("invalid_service_constraint")
        eligible = []
        for city in demand_rows:
            for warehouse in warehouse_rows:
                variable = assignment_variables.get((city.city_id, warehouse.warehouse_id))
                route = routes.get((warehouse.warehouse_id, city.city_id, "last_mile"))
                if (
                    variable is not None
                    and route is not None
                    and route.duration_hours <= target_hours
                ):
                    eligible.append(int(city.demand_quantity) * variable)
        required = math.ceil(
            float(sum(city.demand_quantity for city in demand_rows)) * minimum_coverage
        )
        model.Add(sum(eligible) >= required)

    model.Minimize(candidate_open_count if stage_kind == "minimum_openings" else sum(objective_terms))
    solver = cp_model.CpSolver()
    solver.parameters.max_time_in_seconds = time_limit_seconds
    solver.parameters.num_search_workers = 1
    solver.parameters.random_seed = 0
    status = solver.Solve(model)
    branches = int(solver.NumBranches())
    if status == cp_model.UNKNOWN:
        return SolverStageOutcome(stage_kind, "timeout", "not_available", None, None, None, None, branches)
    if status == cp_model.INFEASIBLE:
        return SolverStageOutcome(stage_kind, "infeasible", "proven", None, None, None, None, branches)
    if status != cp_model.OPTIMAL and status != cp_model.FEASIBLE:
        return SolverStageOutcome(stage_kind, "unavailable", "not_available", None, None, None, None, branches)

    active = {
        warehouse_id
        for warehouse_id, variable in open_variables.items()
        if solver.Value(variable) == 1
    }
    rows: list[AssignmentRow] = []
    unassigned = 0
    for city in demand_rows:
        selected = next(
            (
                warehouse
                for warehouse in warehouse_rows
                if assignment_variables.get((city.city_id, warehouse.warehouse_id)) is not None
                and solver.Value(assignment_variables[(city.city_id, warehouse.warehouse_id)]) == 1
            ),
            None,
        )
        if selected is None:
            rows.append(
                AssignmentRow(
                    demand_city_id=city.city_id,
                    warehouse_id=None,
                    demand_quantity=city.demand_quantity,
                    reason="no_ready_route_or_cost",
                )
            )
            unassigned += city.demand_quantity
            continue
        route = routes[(selected.warehouse_id, city.city_id, "last_mile")]
        cost = _total_unit_cost(costs, selected, city.city_id)
        if cost is None:
            raise ValueError(f"selected_total_cost_missing:{selected.warehouse_id}:{city.city_id}")
        rows.append(
            AssignmentRow(
                demand_city_id=city.city_id,
                warehouse_id=selected.warehouse_id,
                upstream_center_id=selected.upstream_center_id,
                demand_quantity=city.demand_quantity,
                distance_km=route.distance_km,
                duration_hours=route.duration_hours,
                cost=cost,
            )
        )
    assignment = AssignmentResult(
        objective="min_cost",
        rows=rows,
        total_demand=sum(city.demand_quantity for city in demand_rows),
        unassigned_demand=unassigned,
    )
    value = _assignment_objective_value(assignment)
    result = FacilityLocationResult(
        objective_value=value,
        active_warehouse_ids=sorted(active),
        opened_candidate_ids=sorted(
            warehouse_id for warehouse_id in active if warehouse_id not in existing_ids
        ),
        closed_existing_ids=sorted(existing_ids - active),
        assignment=assignment,
    )
    selected_number_to_open = len(result.opened_candidate_ids)
    objective_value = (
        float(selected_number_to_open)
        if stage_kind == "minimum_openings"
        else result.objective_value
    )
    raw_bound = float(solver.BestObjectiveBound())
    best_bound = raw_bound if stage_kind == "minimum_openings" else raw_bound / 1000
    return SolverStageOutcome(
        stage_kind,
        "optimal" if status == cp_model.OPTIMAL else "feasible",
        "proven" if status == cp_model.OPTIMAL else "feasible_only",
        result,
        selected_number_to_open,
        objective_value,
        best_bound,
        branches,
    )


def by_warehouse_cost(assignment: AssignmentResult) -> dict[str, float]:
    result: dict[str, float] = defaultdict(float)
    for row in assignment.rows:
        if row.warehouse_id is not None and row.cost is not None:
            result[row.warehouse_id] += row.cost * float(row.demand_quantity)
    return dict(sorted(result.items()))


def summarize_assignment_cost(
    assignment: AssignmentResult,
    cost_matrix: CostMatrix,
) -> CostSummary:
    costs = _cost_index(cost_matrix)
    last_mile_by_warehouse: dict[str, float] = defaultdict(float)
    linehaul_by_warehouse: dict[str, float] = defaultdict(float)
    missing_routes: list[tuple[str, str, str]] = []
    for row in assignment.rows:
        if row.warehouse_id is None:
            continue
        demand_quantity = float(row.demand_quantity)
        last_mile_rate = costs.get((row.warehouse_id, row.demand_city_id, "last_mile"))
        if last_mile_rate is None:
            missing_routes.append((row.warehouse_id, row.demand_city_id, "last_mile"))
        else:
            last_mile_by_warehouse[row.warehouse_id] += last_mile_rate * demand_quantity
        if row.upstream_center_id and row.upstream_center_id != row.warehouse_id:
            linehaul_key = (
                row.upstream_center_id,
                row.warehouse_id,
                "linehaul",
            )
            linehaul_rate = costs.get(linehaul_key)
            if linehaul_rate is None:
                missing_routes.append(linehaul_key)
            else:
                linehaul_by_warehouse[row.upstream_center_id] += linehaul_rate * demand_quantity
    warehouse_ids = set(last_mile_by_warehouse) | set(linehaul_by_warehouse)
    by_warehouse = {
        warehouse_id: last_mile_by_warehouse[warehouse_id] + linehaul_by_warehouse[warehouse_id]
        for warehouse_id in sorted(warehouse_ids)
    }
    linehaul = sum(linehaul_by_warehouse.values())
    last_mile = sum(last_mile_by_warehouse.values())
    return CostSummary(
        currency=cost_matrix.currency,
        total=linehaul + last_mile,
        linehaul=linehaul,
        last_mile=last_mile,
        by_warehouse=by_warehouse,
        linehaul_by_warehouse=dict(sorted(linehaul_by_warehouse.items())),
        last_mile_by_warehouse=dict(sorted(last_mile_by_warehouse.items())),
        complete=not missing_routes and assignment.unassigned_demand == 0,
        missing_routes=sorted(set(missing_routes)),
    )
