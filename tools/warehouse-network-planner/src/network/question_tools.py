"""Owner module for the business-question analysis tools.

Each tool answers one business question in a single call. Route matrices, cost
matrices and plan comparisons are internal steps built with the owner modules'
functions; their Resource references never pass through the model.
"""

__all__ = [
    "evaluate_baseline",
    "evaluate_scenario",
    "find_sites_for_service_target",
    "register_tools",
]

from decimal import Decimal
from typing import Annotated, Literal

from mcp.server.fastmcp import Context
from mcp.types import CallToolResult, ContentBlock, TextContent
from open_web_codex_provider import ResourceRef
from pydantic import Field, RootModel
from supply_chain_planner.data.mapping import PlanningSourceRole
from supply_chain_planner.data.prepared_resolution import (
    MAX_PREPARED_CANDIDATES,
    resolve_prepared_inputs,
)
from supply_chain_planner.data.workspace_intake import discover_prepared_candidates
from supply_chain_planner.network.matrix import canonical_warehouse_scope
from supply_chain_planner.network.matrix_models import (
    AllWarehousesScope,
    CostMatrix,
    CostPolicySelection,
    ExistingOnlyWarehouseScope,
    ExistingPlusCandidatesWarehouseScope,
    HaversineRouteMatrixStats,
    ObservedQuoteMeanCostEvidence,
    WarehouseScope,
)
from supply_chain_planner.network.matrix_models import RouteMatrix as ComposableRouteMatrix
from supply_chain_planner.network.models import PlanningInputIdentity, WarehouseRecord
from supply_chain_planner.network.optimization_models import (
    AllowExistingWarehouseClosurePolicy,
    AssignmentObjective,
    CityAssignmentChange,
    ComparableNetworkView,
    CoverageComparison,
    CoverageMetricSummary,
    ExactOpeningPolicy,
    KeepAllExistingWarehousePolicy,
    MinimumFeasibleOpeningPolicy,
    PMedianSolverStage,
    ScenarioSpec,
    ServiceCoverageConstraint,
)
from supply_chain_planner.shared.models import (
    AssignmentResultResourceRef,
    ComparableNetworkResultRef,
    FacilityChangeAssessmentToolResult,
    FacilityChangeCostComparison,
    NetworkPlanComparisonResource,
    NetworkPlanComparisonResourceRef,
    PlanningInputQuestion,
    PMedianSolutionToolResult,
    PreparedCandidateSummary,
    PreparedNetworkResource,
    PreparedResolutionReady,
    PreparedResolutionSelectionRequired,
    StrictModel,
    UncoveredCitySummary,
)

from . import analysis_tools, cost_tools, facility_tools, route_tools
from .tool_runtime import (
    BOUNDED_LOCAL_COMPUTE_TOOL,
    CONTENT_ADDRESSED_RESOURCE_TOOL,
    McpResourceContractError,
    _cost_metric_summary,
    _coverage_metric_summary,
    _load_comparable_resource,
    _load_ready_network,
    _runtime,
)

MAX_WAREHOUSE_BREAKDOWN_ROWS = 100
_BASELINE_ROLES = [
    PlanningSourceRole.DEMAND,
    PlanningSourceRole.EXISTING_WAREHOUSE,
    PlanningSourceRole.ROUTE_QUOTE,
]
_CANDIDATE_ROLES = [
    PlanningSourceRole.DEMAND,
    PlanningSourceRole.EXISTING_WAREHOUSE,
    PlanningSourceRole.CANDIDATE_WAREHOUSE,
    PlanningSourceRole.ROUTE_QUOTE,
]


class ProvidedRoutePolicy(StrictModel):
    """Use only the routes present in the prepared input."""

    kind: Literal["provided"] = "provided"


class HaversineRoutePolicy(StrictModel):
    """Estimate only the missing routes from straight-line distance."""

    kind: Literal["haversine"] = "haversine"
    detour_coefficient: float = Field(gt=0)
    average_speed_kph: float = Field(gt=0)


class NavigationRoutePolicy(StrictModel):
    """Use a provider-executed navigation matrix written to the Workspace."""

    kind: Literal["navigation"] = "navigation"
    navigation_matrix_relative_path: str = Field(min_length=1, max_length=1024)


RoutePolicy = Annotated[
    ProvidedRoutePolicy | HaversineRoutePolicy | NavigationRoutePolicy,
    Field(discriminator="kind"),
]


class AnalysisBasis(StrictModel):
    """Which input and which route and cost assumptions produced the result."""

    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    input_identity: PlanningInputIdentity
    warehouse_scope: Literal["existing_only", "existing_plus_candidates", "all_warehouses"]
    objective: AssignmentObjective
    route_method: Literal["provided", "haversine", "navigation"]
    estimated_route_pair_count: int = Field(ge=0)
    cost_source: Literal["quotes", "explicit", "observed_quote_mean"]
    estimated_cost_lane_count: int = Field(ge=0)
    cost_rule_evidence: ObservedQuoteMeanCostEvidence | None = None
    cost_rule_evidence_path: str | None = Field(default=None, max_length=1024)


class CostTotals(StrictModel):
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    total: float = Field(ge=0)
    linehaul: float = Field(ge=0)
    last_mile: float = Field(ge=0)
    complete: bool


class WarehouseBreakdown(StrictModel):
    """Demand one warehouse serves directly and how much of it meets the detail target.

    Linehaul cost belongs to the upstream center, so only last-mile cost is split here.
    """

    warehouse_id: str = Field(min_length=1, max_length=128)
    warehouse_name: str = Field(min_length=1, max_length=256)
    warehouse_type: Literal["center", "cross_docking"]
    is_existing: bool
    city_count: int = Field(ge=0)
    demand_quantity: Decimal = Field(ge=0)
    covered_city_count: int = Field(ge=0)
    covered_demand_quantity: Decimal = Field(ge=0)
    last_mile_cost: float | None = Field(default=None, ge=0)


class NetworkSnapshot(StrictModel):
    """Bounded view of one solved network; result_ref feeds the map and report tools."""

    result_ref: AssignmentResultResourceRef
    label: str = Field(min_length=1, max_length=64)
    active_warehouse_count: int = Field(ge=0)
    cost: CostTotals | None = None
    coverage: list[CoverageMetricSummary] = Field(max_length=32)
    detail_target_hours: float = Field(gt=0)
    uncovered_city_count: int = Field(ge=0)
    uncovered_cities: list[UncoveredCitySummary] = Field(max_length=10)
    uncovered_cities_truncated: bool
    warehouses: list[WarehouseBreakdown] = Field(max_length=MAX_WAREHOUSE_BREAKDOWN_ROWS)
    warehouses_truncated: bool


class WarehouseLabel(StrictModel):
    warehouse_id: str = Field(min_length=1, max_length=128)
    warehouse_name: str = Field(min_length=1, max_length=256)
    city_name: str = Field(min_length=1, max_length=256)
    is_existing: bool


class NetworkAnalysisNeedsInput(StrictModel):
    """Route or quote gaps with no selected policy: ask every open question once."""

    state: Literal["needs_input"]
    summary: str
    next_action: Literal["request_user_input"]
    retryable: Literal[False]
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    missing_route_pair_count: int = Field(ge=0)
    missing_cost_lane_count: int = Field(ge=0)
    questions: list[PlanningInputQuestion] = Field(min_length=1, max_length=3)


class PreparedInputUnavailable(StrictModel):
    state: Literal["prepared_unavailable"]
    summary: str
    next_action: Literal["prepare_data"]
    retryable: Literal[False]
    reasons: list[str] = Field(max_length=64)


class PreparedInputSelectionRequired(StrictModel):
    state: Literal["prepared_selection_required"]
    summary: str
    next_action: Literal["request_user_input"]
    retryable: Literal[False]
    candidates: list[PreparedCandidateSummary] = Field(min_length=2, max_length=64)


class UnresolvedWarehouse(StrictModel):
    identifier: str = Field(min_length=1, max_length=256)
    reason: Literal["not_found", "ambiguous_name"]
    matches: list[WarehouseLabel] = Field(max_length=10)


class WarehouseUnresolved(StrictModel):
    """A named warehouse matched none or several prepared warehouses."""

    state: Literal["warehouse_unresolved"]
    summary: str
    next_action: Literal["request_user_input"]
    retryable: Literal[False]
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    unresolved: list[UnresolvedWarehouse] = Field(min_length=1, max_length=16)


class BaselineReady(StrictModel):
    state: Literal["ready"]
    summary: str
    basis: AnalysisBasis
    baseline: NetworkSnapshot


class ScenarioReady(StrictModel):
    state: Literal["ready"]
    summary: str
    basis: AnalysisBasis
    baseline: NetworkSnapshot
    scenario: NetworkSnapshot
    plan_comparison_ref: NetworkPlanComparisonResourceRef
    added_warehouses: list[WarehouseLabel] = Field(max_length=256)
    removed_warehouses: list[WarehouseLabel] = Field(max_length=256)
    cost: FacilityChangeCostComparison
    coverage: list[CoverageComparison] = Field(max_length=32)
    affected_city_count: int = Field(ge=0)
    affected_city_changes: list[CityAssignmentChange] = Field(max_length=10)
    affected_city_changes_truncated: bool
    reassigned_city_count: int = Field(ge=0)


class SitingReady(StrictModel):
    """Solver outcome; an infeasible target is a valid business result."""

    state: Literal["ready"]
    summary: str
    basis: AnalysisBasis
    baseline: NetworkSnapshot
    status: Literal["optimal", "feasible", "timeout", "infeasible", "unavailable"]
    optimality: Literal["proven", "feasible_only", "not_available"]
    selected_number_to_open: int | None = Field(default=None, ge=0)
    minimum_number_to_open_proven: bool
    solver_stages: list[PMedianSolverStage] = Field(min_length=1, max_length=2)
    opened_candidates: list[WarehouseLabel] = Field(max_length=64)
    closed_existing: list[WarehouseLabel] = Field(max_length=64)
    solution: NetworkSnapshot | None = None
    plan_comparison_ref: NetworkPlanComparisonResourceRef | None = None
    cost: FacilityChangeCostComparison | None = None
    coverage: list[CoverageComparison] = Field(max_length=32)


_Unresolved = PreparedInputUnavailable | PreparedInputSelectionRequired
_Blocked = NetworkAnalysisNeedsInput | _Unresolved


class BaselineToolResult(
    RootModel[Annotated[BaselineReady | _Blocked, Field(discriminator="state")]]
):
    """Baseline result, or the one thing needed before it can be computed."""


class ScenarioToolResult(
    RootModel[
        Annotated[ScenarioReady | _Blocked | WarehouseUnresolved, Field(discriminator="state")]
    ]
):
    """Scenario comparison, or the one thing needed before it can be computed."""


class SitingToolResult(RootModel[Annotated[SitingReady | _Blocked, Field(discriminator="state")]]):
    """Siting result, or the one thing needed before it can be computed."""


class _Matrices(StrictModel):
    route_ref: ResourceRef
    cost_ref: ResourceRef
    basis: AnalysisBasis


def _resolve_prepared_path(
    explicit_path: str | None,
    required_roles: list[PlanningSourceRole],
    ctx: Context,
) -> str | _Unresolved:
    """Use the given prepared input, or the single reusable one for these roles."""
    if explicit_path is not None:
        return explicit_path
    workspace = _runtime().require_workspace(ctx)
    try:
        candidate_paths = [
            str(item["relative_path"]) for item in discover_prepared_candidates(workspace)
        ]
    except ValueError as error:
        raise McpResourceContractError(str(error)) from error
    if not candidate_paths:
        return PreparedInputUnavailable(
            state="prepared_unavailable",
            summary="The Workspace has no prepared network input; prepare the data first.",
            next_action="prepare_data",
            retryable=False,
            reasons=[],
        )
    if len(candidate_paths) > MAX_PREPARED_CANDIDATES:
        raise McpResourceContractError("prepared_candidate_limit_exceeded")
    try:
        outcome = resolve_prepared_inputs(workspace, candidate_paths, required_roles, None)
    except ValueError as error:
        raise McpResourceContractError(str(error)) from error
    if isinstance(outcome, PreparedResolutionReady):
        return outcome.prepared_input_relative_path
    if isinstance(outcome, PreparedResolutionSelectionRequired):
        return PreparedInputSelectionRequired(
            state="prepared_selection_required",
            summary=outcome.summary,
            next_action="request_user_input",
            retryable=False,
            candidates=outcome.candidates,
        )
    return PreparedInputUnavailable(
        state="prepared_unavailable",
        summary=outcome.summary,
        next_action="prepare_data",
        retryable=False,
        reasons=outcome.reasons,
    )


def _blocked_result(result: StrictModel) -> CallToolResult:
    structured = result.model_dump(mode="json", by_alias=True)
    return CallToolResult(
        content=[TextContent(type="text", text=structured["summary"])],
        structuredContent=structured,
    )


def _structured(result: CallToolResult, error_code: str) -> dict:
    if result.structuredContent is None:
        raise McpResourceContractError(error_code)
    return result.structuredContent


def _prepare_matrices(
    prepared_path: str,
    warehouse_scope: WarehouseScope,
    objective: AssignmentObjective,
    route_policy: RoutePolicy | None,
    cost_policy: CostPolicySelection | None,
    quote_mean_evidence_relative_path: str | None,
    ctx: Context,
) -> _Matrices | NetworkAnalysisNeedsInput:
    """Build both matrices, or collect every open completion question at once."""
    prepared, input_identity = _load_ready_network(prepared_path, ctx)
    warehouse_scope = canonical_warehouse_scope(prepared.warehouses, warehouse_scope)
    questions: list[PlanningInputQuestion] = []
    missing_route_pair_count = 0
    route_ref: ResourceRef | None = None
    if isinstance(route_policy, NavigationRoutePolicy):
        imported = route_tools.import_navigation_matrix(
            prepared_path,
            route_policy.navigation_matrix_relative_path,
            ctx,
        )
        route_ref = ResourceRef.model_validate(
            _structured(imported, "route_matrix_result_missing")["resource_ref"]
        )
    else:
        haversine = route_policy if isinstance(route_policy, HaversineRoutePolicy) else None
        route_result = _structured(
            route_tools.prepare_route_matrix(
                prepared_path,
                "haversine" if haversine is not None else "provided",
                ctx,
                warehouse_scope=warehouse_scope,
                detour_coefficient=haversine.detour_coefficient if haversine else None,
                average_speed_kph=haversine.average_speed_kph if haversine else None,
            ),
            "route_matrix_result_missing",
        )
        if route_result["state"] == "needs_input":
            missing_route_pair_count = route_result["missing_pair_count"]
            questions.extend(
                PlanningInputQuestion.model_validate(item) for item in route_result["questions"]
            )
        else:
            route_ref = ResourceRef.model_validate(route_result["resource_ref"])
    route_matrix = (
        _runtime().load_model(route_ref, "route_matrix.v3", ComposableRouteMatrix)
        if route_ref is not None
        else None
    )
    if route_matrix is not None and route_matrix.warehouse_scope != warehouse_scope:
        raise McpResourceContractError("navigation_route_scope_mismatch")
    missing_cost_lane_count = 0
    cost_ref: ResourceRef | None = None
    # A selected cost policy may price lanes by route distance, so it is only
    # applied once routes exist. Quote gaps are still detected without routes so
    # that both questions reach the user in one card.
    if route_ref is not None or cost_policy is None:
        cost_result = _structured(
            cost_tools.plan_cost_matrix(
                prepared_path,
                warehouse_scope,
                ctx,
                cost_policy=cost_policy,
                route_matrix_ref=route_ref,
                quote_mean_evidence_relative_path=(
                    quote_mean_evidence_relative_path if route_ref is not None else None
                ),
            ),
            "cost_matrix_result_missing",
        )
        if cost_result["state"] == "needs_input":
            missing_cost_lane_count = cost_result["missing_pair_count"]
            questions.extend(
                PlanningInputQuestion.model_validate(item) for item in cost_result["questions"]
            )
        else:
            cost_ref = ResourceRef.model_validate(cost_result["resource_ref"])
    if questions or route_ref is None or cost_ref is None or route_matrix is None:
        gaps = []
        if missing_route_pair_count:
            gaps.append(f"{missing_route_pair_count} route pairs")
        if missing_cost_lane_count:
            gaps.append(f"{missing_cost_lane_count} lane quotes")
        return NetworkAnalysisNeedsInput(
            state="needs_input",
            summary=(
                f"The analysis is missing {' and '.join(gaps)}. Ask every question in one "
                "input card, then call this tool again with the chosen policies."
            ),
            next_action="request_user_input",
            retryable=False,
            prepared_input_relative_path=prepared_path,
            missing_route_pair_count=missing_route_pair_count,
            missing_cost_lane_count=missing_cost_lane_count,
            questions=questions,
        )
    cost_matrix = _runtime().load_model(cost_ref, "cost_matrix.v3", CostMatrix)
    route_stats = route_matrix.stats
    return _Matrices(
        route_ref=route_ref,
        cost_ref=cost_ref,
        basis=AnalysisBasis(
            prepared_input_relative_path=prepared_path,
            input_identity=input_identity,
            warehouse_scope=warehouse_scope.kind,
            objective=objective,
            route_method=route_matrix.method,
            estimated_route_pair_count=(
                route_stats.computed_pair_count + route_stats.reused_pair_count
                if isinstance(route_stats, HaversineRouteMatrixStats)
                else 0
            ),
            cost_source=cost_matrix.calculation_rule_source or "quotes",
            estimated_cost_lane_count=sum(row.source == "calculated" for row in cost_matrix.rows),
            cost_rule_evidence=cost_matrix.calculation_rule_evidence,
            cost_rule_evidence_path=cost_matrix.calculation_rule_evidence_path,
        ),
    )


def _warehouse_label(warehouse: WarehouseRecord) -> WarehouseLabel:
    return WarehouseLabel(
        warehouse_id=warehouse.warehouse_id,
        warehouse_name=warehouse.warehouse_name,
        city_name=warehouse.city_name,
        is_existing=warehouse.is_existing,
    )


def _warehouse_labels(prepared: PreparedNetworkResource, ids: list[str]) -> list[WarehouseLabel]:
    by_id = {warehouse.warehouse_id: warehouse for warehouse in prepared.warehouses}
    return [_warehouse_label(by_id[item]) for item in ids]


def _snapshot(
    result_ref: ResourceRef,
    view: ComparableNetworkView,
    prepared: PreparedNetworkResource,
    targets: list[float],
) -> NetworkSnapshot:
    coverage, detail_target, uncovered = analysis_tools._baseline_coverage_projection(
        view.assignment,
        prepared.demand_cities,
        prepared.warehouses,
        targets,
    )
    by_id = {warehouse.warehouse_id: warehouse for warehouse in prepared.warehouses}
    totals: dict[str, list] = {}
    for row in view.assignment.rows:
        if row.warehouse_id is None:
            continue
        entry = totals.setdefault(row.warehouse_id, [0, Decimal(0), 0, Decimal(0)])
        entry[0] += 1
        entry[1] += row.demand_quantity
        if row.duration_hours is not None and row.duration_hours <= detail_target:
            entry[2] += 1
            entry[3] += row.demand_quantity
    last_mile_cost = view.cost.last_mile_by_warehouse if view.cost is not None else {}
    breakdown = sorted(
        (
            WarehouseBreakdown(
                warehouse_id=warehouse_id,
                warehouse_name=by_id[warehouse_id].warehouse_name,
                warehouse_type=by_id[warehouse_id].warehouse_type,
                is_existing=by_id[warehouse_id].is_existing,
                city_count=entry[0],
                demand_quantity=entry[1],
                covered_city_count=entry[2],
                covered_demand_quantity=entry[3],
                last_mile_cost=last_mile_cost.get(warehouse_id),
            )
            for warehouse_id, entry in totals.items()
        ),
        key=lambda item: (-item.demand_quantity, item.warehouse_id),
    )
    return NetworkSnapshot(
        result_ref=AssignmentResultResourceRef.model_validate(result_ref.model_dump(mode="json")),
        label=view.label,
        active_warehouse_count=len(view.active_warehouse_ids),
        cost=(
            CostTotals(
                currency=view.cost.currency,
                total=view.cost.total,
                linehaul=view.cost.linehaul,
                last_mile=view.cost.last_mile,
                complete=view.cost.complete,
            )
            if view.cost is not None
            else None
        ),
        coverage=coverage,
        detail_target_hours=detail_target,
        uncovered_city_count=len(uncovered),
        uncovered_cities=uncovered[:10],
        uncovered_cities_truncated=len(uncovered) > 10,
        warehouses=breakdown[:MAX_WAREHOUSE_BREAKDOWN_ROWS],
        warehouses_truncated=len(breakdown) > MAX_WAREHOUSE_BREAKDOWN_ROWS,
    )


def _baseline_snapshot(
    prepared_path: str,
    matrices: _Matrices,
    targets: list[float],
    coverage_mode: Literal["auto", "actual_current", "optimized_existing_footprint"],
    ctx: Context,
) -> tuple[NetworkSnapshot, ComparableNetworkResultRef, list[ContentBlock]]:
    published = analysis_tools.evaluate_network_baseline(
        prepared_path,
        matrices.route_ref,
        matrices.basis.objective,
        targets,
        ctx,
        coverage_mode=coverage_mode,
        cost_matrix_ref=matrices.cost_ref,
    )
    baseline_ref = ComparableNetworkResultRef.model_validate(
        _structured(published, "baseline_result_missing")["resource_ref"]
    )
    prepared, _identity = _load_ready_network(prepared_path, ctx)
    snapshot = _snapshot(
        baseline_ref,
        _load_comparable_resource(baseline_ref),
        prepared,
        targets,
    )
    return snapshot, baseline_ref, list(published.content[1:])


def _ready_result(result: StrictModel, links: list[ContentBlock]) -> CallToolResult:
    structured = result.model_dump(mode="json", by_alias=True)
    return CallToolResult(
        content=[TextContent(type="text", text=structured["summary"]), *links],
        structuredContent=structured,
    )


def _ordered_targets(service_targets: list[float]) -> list[float]:
    if any(target <= 0 for target in service_targets):
        raise McpResourceContractError("service_targets_invalid")
    return sorted(set(service_targets))


def _snapshot_summary(snapshot: NetworkSnapshot) -> str:
    cost = (
        f"{snapshot.cost.total:.2f} {snapshot.cost.currency}"
        if snapshot.cost is not None
        else "unavailable"
    )
    return (
        f"{snapshot.active_warehouse_count} active warehouses, cost {cost}, service "
        f"{_coverage_metric_summary(snapshot.coverage)}"
    )


def evaluate_baseline(
    service_targets: Annotated[
        list[float],
        Field(min_length=1, max_length=32, description="Service time limits in hours."),
    ],
    ctx: Context,
    prepared_input_relative_path: Annotated[
        str | None,
        Field(
            max_length=1024,
            description=(
                "Omit to reuse the single prepared input in the Workspace. Pass a path only "
                "after prepared_selection_required or a fresh data preparation."
            ),
        ),
    ] = None,
    route_policy: Annotated[
        RoutePolicy | None,
        Field(description="Omit first. Set only to the user's choice after needs_input."),
    ] = None,
    cost_policy: Annotated[
        CostPolicySelection | None,
        Field(description="Omit first. Set only to the user's choice after needs_input."),
    ] = None,
    objective: AssignmentObjective = "min_cost",
    coverage_mode: Literal["auto", "actual_current", "optimized_existing_footprint"] = "auto",
    quote_mean_evidence_relative_path: Annotated[str | None, Field(max_length=1024)] = None,
) -> Annotated[CallToolResult, BaselineToolResult]:
    """Evaluate the current network: service coverage, cost and a per-warehouse breakdown.

    One call builds routes and costs for the existing warehouses and evaluates
    them. The default auto mode uses the actual current assignment when the
    prepared input has one and otherwise the optimized existing footprint; the
    result label states which.
    """
    targets = _ordered_targets(service_targets)
    prepared_path = _resolve_prepared_path(prepared_input_relative_path, _BASELINE_ROLES, ctx)
    if not isinstance(prepared_path, str):
        return _blocked_result(prepared_path)
    matrices = _prepare_matrices(
        prepared_path,
        ExistingOnlyWarehouseScope(),
        objective,
        route_policy,
        cost_policy,
        quote_mean_evidence_relative_path,
        ctx,
    )
    if isinstance(matrices, NetworkAnalysisNeedsInput):
        return _blocked_result(matrices)
    baseline, _baseline_ref, links = _baseline_snapshot(
        prepared_path, matrices, targets, coverage_mode, ctx
    )
    return _ready_result(
        BaselineReady(
            state="ready",
            summary=f"Baseline ({baseline.label}): {_snapshot_summary(baseline)}.",
            basis=matrices.basis,
            baseline=baseline,
        ),
        links,
    )


def _resolve_warehouses(
    prepared: PreparedNetworkResource,
    identifiers: list[str],
) -> tuple[list[str], list[UnresolvedWarehouse]]:
    """Match each identifier to one warehouse by exact ID, then by exact name."""
    by_id = {warehouse.warehouse_id: warehouse for warehouse in prepared.warehouses}
    resolved: list[str] = []
    unresolved: list[UnresolvedWarehouse] = []
    for identifier in identifiers:
        if identifier in by_id:
            resolved.append(identifier)
            continue
        named = [
            warehouse for warehouse in prepared.warehouses if warehouse.warehouse_name == identifier
        ]
        if len(named) == 1:
            resolved.append(named[0].warehouse_id)
            continue
        unresolved.append(
            UnresolvedWarehouse(
                identifier=identifier,
                reason="ambiguous_name" if named else "not_found",
                matches=[_warehouse_label(warehouse) for warehouse in named[:10]],
            )
        )
    return resolved, unresolved


def evaluate_scenario(
    service_targets: Annotated[
        list[float],
        Field(min_length=1, max_length=32, description="Service time limits in hours."),
    ],
    ctx: Context,
    add_warehouses: Annotated[
        list[str] | None,
        Field(
            max_length=64,
            description="Candidate warehouses to open, each as its exact ID or exact name.",
        ),
    ] = None,
    remove_warehouses: Annotated[
        list[str] | None,
        Field(
            max_length=64,
            description="Existing warehouses to close, each as its exact ID or exact name.",
        ),
    ] = None,
    prepared_input_relative_path: Annotated[
        str | None,
        Field(
            max_length=1024,
            description=(
                "Omit to reuse the single prepared input in the Workspace. Pass a path only "
                "after prepared_selection_required or a fresh data preparation."
            ),
        ),
    ] = None,
    route_policy: Annotated[
        RoutePolicy | None,
        Field(description="Omit first. Set only to the user's choice after needs_input."),
    ] = None,
    cost_policy: Annotated[
        CostPolicySelection | None,
        Field(description="Omit first. Set only to the user's choice after needs_input."),
    ] = None,
    objective: AssignmentObjective = "min_cost",
    quote_mean_evidence_relative_path: Annotated[str | None, Field(max_length=1024)] = None,
) -> Annotated[CallToolResult, ScenarioToolResult]:
    """Compare the current network with one that opens and/or closes named warehouses.

    One call evaluates the baseline, applies the change and returns both with
    their comparison. A name that matches no warehouse or several returns
    warehouse_unresolved instead of a guess.
    """
    targets = _ordered_targets(service_targets)
    add_warehouses = add_warehouses or []
    remove_warehouses = remove_warehouses or []
    if not add_warehouses and not remove_warehouses:
        raise McpResourceContractError("scenario_requires_a_warehouse_change")
    required_roles = _CANDIDATE_ROLES if add_warehouses else _BASELINE_ROLES
    prepared_path = _resolve_prepared_path(prepared_input_relative_path, required_roles, ctx)
    if not isinstance(prepared_path, str):
        return _blocked_result(prepared_path)
    prepared, _identity = _load_ready_network(prepared_path, ctx)
    add_ids, unresolved_add = _resolve_warehouses(prepared, add_warehouses)
    remove_ids, unresolved_remove = _resolve_warehouses(prepared, remove_warehouses)
    unresolved = [*unresolved_add, *unresolved_remove]
    if unresolved:
        return _blocked_result(
            WarehouseUnresolved(
                state="warehouse_unresolved",
                summary=(
                    f"{len(unresolved)} warehouse identifier(s) did not match exactly one "
                    "prepared warehouse; ask the user which warehouse is meant."
                ),
                next_action="request_user_input",
                retryable=False,
                prepared_input_relative_path=prepared_path,
                unresolved=unresolved[:16],
            )
        )
    warehouse_scope: WarehouseScope = (
        ExistingPlusCandidatesWarehouseScope(candidate_ids=sorted(set(add_ids)))
        if add_ids
        else ExistingOnlyWarehouseScope()
    )
    matrices = _prepare_matrices(
        prepared_path,
        warehouse_scope,
        objective,
        route_policy,
        cost_policy,
        quote_mean_evidence_relative_path,
        ctx,
    )
    if isinstance(matrices, NetworkAnalysisNeedsInput):
        return _blocked_result(matrices)
    baseline, baseline_ref, baseline_links = _baseline_snapshot(
        prepared_path, matrices, targets, "auto", ctx
    )
    assessed = facility_tools.assess_facility_change(
        prepared_path,
        matrices.route_ref,
        baseline_ref,
        ScenarioSpec(
            add_warehouse_ids=add_ids,
            remove_warehouse_ids=remove_ids,
            objective=objective,
            service_targets=targets,
        ),
        ctx,
        cost_matrix_ref=matrices.cost_ref,
    )
    assessment = FacilityChangeAssessmentToolResult.model_validate(
        _structured(assessed, "facility_change_result_missing")
    )
    scenario_ref = ComparableNetworkResultRef.model_validate(
        assessment.scenario_ref.model_dump(mode="json")
    )
    scenario = _snapshot(scenario_ref, _load_comparable_resource(scenario_ref), prepared, targets)
    return _ready_result(
        ScenarioReady(
            state="ready",
            summary=(
                f"Baseline ({baseline.label}): {_snapshot_summary(baseline)}. "
                f"Scenario: {_snapshot_summary(scenario)}."
            ),
            basis=matrices.basis,
            baseline=baseline,
            scenario=scenario,
            plan_comparison_ref=assessment.plan_comparison_ref,
            added_warehouses=_warehouse_labels(prepared, assessment.added_warehouse_ids),
            removed_warehouses=_warehouse_labels(prepared, assessment.removed_warehouse_ids),
            cost=assessment.cost,
            coverage=assessment.coverage,
            affected_city_count=assessment.affected_city_count,
            affected_city_changes=assessment.affected_city_changes,
            affected_city_changes_truncated=assessment.affected_city_changes_truncated,
            reassigned_city_count=assessment.reassigned_city_count,
        ),
        [*baseline_links, *assessed.content[1:]],
    )


def find_sites_for_service_target(
    service_targets: Annotated[
        list[float],
        Field(min_length=1, max_length=32, description="Service time limits in hours to report."),
    ],
    ctx: Context,
    service_constraints: Annotated[
        list[ServiceCoverageConstraint] | None,
        Field(
            max_length=8,
            description=(
                "Coverage the plan must reach, for example 0.9 within 12 hours. Required "
                "unless number_to_open is given."
            ),
        ),
    ] = None,
    number_to_open: Annotated[
        int | None,
        Field(
            ge=0,
            le=64,
            description=(
                "Omit to find the fewest new warehouses that meet the constraints and then "
                "the cheapest combination of that size. Set only when the user fixes the count."
            ),
        ),
    ] = None,
    closable_existing_warehouses: Annotated[
        list[str] | None,
        Field(
            max_length=64,
            description=(
                "Existing warehouses the user explicitly allows to close, each as its exact "
                "ID or exact name. Every other existing warehouse stays open."
            ),
        ),
    ] = None,
    prepared_input_relative_path: Annotated[
        str | None,
        Field(
            max_length=1024,
            description=(
                "Omit to reuse the single prepared input in the Workspace. Pass a path only "
                "after prepared_selection_required or a fresh data preparation."
            ),
        ),
    ] = None,
    route_policy: Annotated[
        RoutePolicy | None,
        Field(description="Omit first. Set only to the user's choice after needs_input."),
    ] = None,
    cost_policy: Annotated[
        CostPolicySelection | None,
        Field(description="Omit first. Set only to the user's choice after needs_input."),
    ] = None,
    time_limit_seconds: Annotated[float, Field(gt=0, le=300)] = 60,
    quote_mean_evidence_relative_path: Annotated[str | None, Field(max_length=1024)] = None,
) -> Annotated[CallToolResult, SitingToolResult]:
    """Choose which candidate warehouses to open to reach a service target at least cost.

    One call evaluates the baseline, solves once and returns the plan with its
    comparison. Without number_to_open the answer is already the cheapest plan
    with the fewest new warehouses; do not retry other counts. An infeasible
    status is a valid business result.
    """
    targets = _ordered_targets(service_targets)
    if number_to_open is None and not service_constraints:
        raise McpResourceContractError("siting_requires_service_constraints_or_number_to_open")
    prepared_path = _resolve_prepared_path(prepared_input_relative_path, _CANDIDATE_ROLES, ctx)
    if not isinstance(prepared_path, str):
        return _blocked_result(prepared_path)
    prepared, _identity = _load_ready_network(prepared_path, ctx)
    closable_ids, unresolved = _resolve_warehouses(
        prepared, closable_existing_warehouses or []
    )
    if unresolved:
        raise McpResourceContractError("siting_closable_warehouse_unresolved")
    matrices = _prepare_matrices(
        prepared_path,
        AllWarehousesScope(),
        "min_cost",
        route_policy,
        cost_policy,
        quote_mean_evidence_relative_path,
        ctx,
    )
    if isinstance(matrices, NetworkAnalysisNeedsInput):
        return _blocked_result(matrices)
    baseline, baseline_ref, links = _baseline_snapshot(
        prepared_path, matrices, targets, "auto", ctx
    )
    solved = facility_tools.solve_p_median(
        prepared_path,
        matrices.route_ref,
        matrices.cost_ref,
        (
            MinimumFeasibleOpeningPolicy()
            if number_to_open is None
            else ExactOpeningPolicy(number_to_open=number_to_open)
        ),
        (
            AllowExistingWarehouseClosurePolicy(closable_existing_ids=closable_ids)
            if closable_ids
            else KeepAllExistingWarehousePolicy()
        ),
        targets,
        time_limit_seconds,
        ctx,
        service_constraints=service_constraints or None,
    )
    solution = PMedianSolutionToolResult.model_validate(
        _structured(solved, "p_median_result_missing")
    )
    snapshot: NetworkSnapshot | None = None
    comparison_ref: NetworkPlanComparisonResourceRef | None = None
    cost: FacilityChangeCostComparison | None = None
    coverage: list[CoverageComparison] = []
    if solution.status in {"optimal", "feasible"}:
        solution_ref = ComparableNetworkResultRef.model_validate(
            solution.resource_ref.model_dump(mode="json")
        )
        snapshot = _snapshot(
            solution_ref, _load_comparable_resource(solution_ref), prepared, targets
        )
        compared = analysis_tools.compare_network_scenarios(
            prepared_path, baseline_ref, solution_ref, targets, ctx
        )
        comparison_ref = NetworkPlanComparisonResourceRef.model_validate(
            _structured(compared, "comparison_result_missing")["resource_ref"]
        )
        comparison = (
            _runtime()
            .load_model(
                comparison_ref,
                "network_plan_comparison.v2",
                NetworkPlanComparisonResource,
            )
            .comparison
        )
        cost = FacilityChangeCostComparison(
            currency=snapshot.cost.currency if snapshot.cost is not None else None,
            before_total=comparison.before_cost,
            after_total=comparison.after_cost,
            delta=comparison.cost_delta,
            complete=comparison.before_cost is not None and comparison.after_cost is not None,
        )
        coverage = comparison.coverage
        links = [*links, *solved.content[1:], *compared.content[1:]]
    plan_summary = (
        f"opens {len(solution.opened_candidate_ids)} and closes "
        f"{len(solution.closed_existing_ids)} warehouses, cost "
        f"{_cost_metric_summary(solution.cost)}, service "
        f"{_coverage_metric_summary(solution.coverage)}"
        if snapshot is not None
        else "no plan was found"
    )
    return _ready_result(
        SitingReady(
            state="ready",
            summary=(
                f"Baseline ({baseline.label}): {_snapshot_summary(baseline)}. "
                f"Siting status {solution.status}: {plan_summary}."
            ),
            basis=matrices.basis,
            baseline=baseline,
            status=solution.status,
            optimality=solution.optimality,
            selected_number_to_open=solution.selected_number_to_open,
            minimum_number_to_open_proven=solution.minimum_number_to_open_proven,
            solver_stages=solution.solver_stages,
            opened_candidates=_warehouse_labels(prepared, solution.opened_candidate_ids),
            closed_existing=_warehouse_labels(prepared, solution.closed_existing_ids),
            solution=snapshot,
            plan_comparison_ref=comparison_ref,
            cost=cost,
            coverage=coverage,
        ),
        links,
    )


def register_tools(mcp) -> None:
    mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(
        evaluate_baseline
    )
    mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(
        evaluate_scenario
    )
    mcp.tool(structured_output=True, annotations=BOUNDED_LOCAL_COMPUTE_TOOL)(
        find_sites_for_service_target
    )
