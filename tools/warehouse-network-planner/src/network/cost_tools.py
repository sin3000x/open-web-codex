"""Owner module for cost tools."""

import math
from pathlib import (
    PurePosixPath,
)
from typing import (
    Annotated,
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
    ValidationError,
)
from supply_chain_planner.data.workspace_intake import (
    read_json_document,
)
from supply_chain_planner.network.matrix import (
    build_cost_matrix as _build_composable_cost_matrix,
)
from supply_chain_planner.network.matrix import (
    canonical_warehouse_scope,
    derive_observed_quote_mean_cost_policy,
    resolve_warehouse_scope,
)
from supply_chain_planner.network.matrix_models import (
    CostCalculationPolicy,
    CostMatrix,
    CostPolicySelection,
    ExplicitCostPolicy,
    ObservedQuoteMeanCostEvidence,
    ObservedQuoteMeanCostPolicy,
    WarehouseScope,
)
from supply_chain_planner.network.matrix_models import (
    RouteMatrix as ComposableRouteMatrix,
)
from supply_chain_planner.shared.models import (
    CostMatrixNeedsInput,
    CostMatrixPlanningToolResult,
    CostMatrixReady,
    PlanningInputOption,
    PlanningInputQuestion,
)
from supply_chain_planner.shared.planning_input import (
    require_matching_input,
)

from .tool_runtime import (
    CONTENT_ADDRESSED_RESOURCE_TOOL,
    McpResourceContractError,
    _load_ready_network,
    _runtime,
)


def _validate_quote_mean_evidence_path(
    relative_path: str,
    prepared_input_relative_path: str,
) -> str:
    """Accept only a create-new typed evidence file in the package output dir."""

    candidate = PurePosixPath(relative_path)
    expected_parent = PurePosixPath("outputs/warehouse-network/calculations")
    if (
        candidate.is_absolute()
        or candidate.as_posix() != relative_path
        or candidate.parent != expected_parent
        or candidate.suffix.lower() != ".json"
        or candidate.name in {"", ".", ".."}
        or relative_path == prepared_input_relative_path
    ):
        raise McpResourceContractError("cost_evidence_path_invalid")
    return candidate.as_posix()


def _quote_mean_evidence_matches(
    expected: ObservedQuoteMeanCostEvidence,
    supplied: ObservedQuoteMeanCostEvidence,
) -> bool:
    """Compare script evidence to canonical Tool output without float noise."""

    if (
        expected.schema_version != supplied.schema_version
        or expected.prepared_input_relative_path != supplied.prepared_input_relative_path
        or expected.input_identity != supplied.input_identity
        or expected.total_quote_count != supplied.total_quote_count
        or expected.method != supplied.method
        or expected.tool_version != supplied.tool_version
        or expected.formula != supplied.formula
        or expected.considered_quote_count != supplied.considered_quote_count
        or expected.ignored_quote_count != supplied.ignored_quote_count
    ):
        return False
    expected_rules = {rule.layer: rule for rule in expected.rules}
    supplied_rules = {rule.layer: rule for rule in supplied.rules}
    if (
        len(expected.rules) != len(supplied.rules)
        or len(expected_rules) != len(expected.rules)
        or len(supplied_rules) != len(supplied.rules)
        or set(expected_rules) != set(supplied_rules)
    ):
        return False
    return all(
        expected_rules[layer].currency == supplied_rules[layer].currency
        and expected_rules[layer].quote_count == supplied_rules[layer].quote_count
        and math.isclose(
            expected_rules[layer].mean_cost_per_demand_unit,
            supplied_rules[layer].mean_cost_per_demand_unit,
            rel_tol=0,
            abs_tol=1e-6,
        )
        for layer in expected_rules
    )


def plan_cost_matrix(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    warehouse_scope: WarehouseScope,
    ctx: Context,
    cost_policy: CostPolicySelection | None = None,
    route_matrix_ref: ResourceRef | None = None,
    prior_cost_matrix_ref: ResourceRef | None = None,
    quote_mean_evidence_relative_path: Annotated[str | None, Field(max_length=1024)] = None,
) -> Annotated[CallToolResult, CostMatrixPlanningToolResult]:
    """Build quote-first costs; lanes without a quote use the selected fallback policy.

    Without cost_policy, quote gaps return needs_input instead of a matrix.
    Pass {"kind": "observed_quote_mean"} to fill gaps with the tool-computed
    mean unit cost of all prepared quotes.
    """
    prepared, input_identity = _load_ready_network(prepared_input_relative_path, ctx)
    warehouse_scope = canonical_warehouse_scope(prepared.warehouses, warehouse_scope)
    warehouses = resolve_warehouse_scope(prepared.warehouses, warehouse_scope)
    warehouse_ids = [warehouse.warehouse_id for warehouse in warehouses]
    calculation_policy = None
    calculation_rule_source = None
    calculation_rule_evidence = None
    if isinstance(cost_policy, ExplicitCostPolicy):
        if quote_mean_evidence_relative_path is not None:
            raise McpResourceContractError("cost_evidence_requires_observed_quote_mean")
        calculation_policy = CostCalculationPolicy(rules=cost_policy.rules)
        calculation_rule_source = "explicit"
    elif isinstance(cost_policy, ObservedQuoteMeanCostPolicy):
        calculation_policy, calculation_rule_evidence = derive_observed_quote_mean_cost_policy(
            prepared.demand_cities,
            warehouses,
            prepared.route_quotes,
            prepared_input_relative_path=prepared_input_relative_path,
            input_identity=input_identity,
        )
        calculation_rule_source = "observed_quote_mean"
        if quote_mean_evidence_relative_path is not None:
            evidence_path = _validate_quote_mean_evidence_path(
                quote_mean_evidence_relative_path,
                prepared_input_relative_path,
            )
            try:
                evidence_payload = read_json_document(
                    _runtime().require_workspace(ctx),
                    evidence_path,
                )
                supplied_evidence = ObservedQuoteMeanCostEvidence.model_validate(
                    evidence_payload
                )
            except (ValueError, ValidationError) as error:
                raise McpResourceContractError("cost_evidence_invalid") from error
            if not _quote_mean_evidence_matches(calculation_rule_evidence, supplied_evidence):
                raise McpResourceContractError("cost_evidence_mismatch")
            quote_mean_evidence_relative_path = evidence_path
        else:
            quote_mean_evidence_relative_path = None
    elif quote_mean_evidence_relative_path is not None:
        raise McpResourceContractError("cost_evidence_requires_observed_quote_mean")
    route_matrix = (
        _runtime().load_model(
            route_matrix_ref,
            "route_matrix.v3",
            ComposableRouteMatrix,
        )
        if route_matrix_ref is not None
        else None
    )
    if route_matrix is not None:
        try:
            require_matching_input(input_identity, route_matrix.input_identity)
        except ValueError as error:
            raise McpResourceContractError("cost_route_input_identity_mismatch") from error
        if route_matrix.warehouse_scope != warehouse_scope:
            raise McpResourceContractError("cost_route_scope_mismatch")
        if route_matrix.warehouse_ids != warehouse_ids:
            raise McpResourceContractError("cost_route_warehouse_set_mismatch")
    prior = (
        _runtime().load_model(
            prior_cost_matrix_ref,
            "cost_matrix.v3",
            CostMatrix,
        )
        if prior_cost_matrix_ref is not None
        else None
    )
    if prior is not None:
        if route_matrix is not None and not _prior_route_facts_match(prior, route_matrix):
            raise McpResourceContractError("cost_prior_route_mismatch")
        try:
            require_matching_input(input_identity, prior.input_identity)
        except ValueError as error:
            raise McpResourceContractError("cost_prior_input_identity_mismatch") from error
        if prior.warehouse_scope != warehouse_scope:
            raise McpResourceContractError("cost_prior_scope_mismatch")
        if prior.warehouse_ids != warehouse_ids:
            raise McpResourceContractError("cost_prior_warehouse_set_mismatch")
        if (
            prior.calculation_rule != calculation_policy
            or prior.calculation_rule_source != calculation_rule_source
            or prior.calculation_rule_evidence != calculation_rule_evidence
        ):
            raise McpResourceContractError("cost_prior_rule_mismatch")
    matrix = _build_composable_cost_matrix(
        prepared.demand_cities,
        warehouses,
        prepared.route_quotes,
        calculation_policy,
        route_matrix,
        prior.rows if prior is not None else None,
        warehouse_scope=warehouse_scope,
        input_identity=input_identity,
        calculation_rule_source=calculation_rule_source,
        calculation_rule_evidence=calculation_rule_evidence,
        calculation_rule_evidence_path=quote_mean_evidence_relative_path,
    )
    stats = matrix.stats
    if calculation_policy is None and stats.missing_pair_count:
        return _cost_gap_needs_input(warehouse_scope, matrix)
    policy_summary = ""
    if calculation_rule_evidence is not None:
        policy_summary = "; observed quote means: " + ", ".join(
            f"{rule.layer}={rule.mean_cost_per_demand_unit:.6f} {rule.currency} "
            f"from {rule.quote_count} quotes"
            for rule in calculation_rule_evidence.rules
        )
    summary = (
        "Built cost matrix with "
        f"{stats.reused_pair_count} reused, "
        f"{stats.computed_pair_count} computed, and "
        f"{stats.missing_pair_count} missing lane costs{policy_summary}."
    )
    published = _runtime().publish(
        matrix.schema_version,
        matrix,
        summary,
    )
    if published.structuredContent is None:
        raise McpResourceContractError("cost_matrix_result_missing")
    resource_ref = ResourceRef.model_validate(published.structuredContent["resource_ref"])
    result = CostMatrixReady(
        state="ready",
        summary=summary,
        resource_ref=resource_ref,
        input_identity=input_identity,
        calculation_rule_source=matrix.calculation_rule_source,
        calculation_rule_evidence=matrix.calculation_rule_evidence,
        calculation_rule_evidence_path=matrix.calculation_rule_evidence_path,
        expected_pair_count=stats.expected_pair_count,
        reused_pair_count=stats.reused_pair_count,
        computed_pair_count=stats.computed_pair_count,
        missing_pair_count=stats.missing_pair_count,
        warehouse_ids=warehouse_ids,
    )
    published.structuredContent = result.model_dump(mode="json")
    return published


def _cost_gap_needs_input(warehouse_scope: WarehouseScope, matrix: CostMatrix) -> CallToolResult:
    stats = matrix.stats
    result = CostMatrixPlanningToolResult(
        root=CostMatrixNeedsInput(
            state="needs_input",
            summary=(
                f"The {warehouse_scope.kind} cost scope has no quote for "
                f"{stats.missing_pair_count} of {stats.expected_pair_count} lanes. "
                "Choose one cost fallback before network evaluation."
            ),
            next_action="request_user_input",
            retryable=False,
            warehouse_scope=warehouse_scope,
            warehouse_ids=matrix.warehouse_ids,
            expected_pair_count=stats.expected_pair_count,
            missing_pair_count=stats.missing_pair_count,
            questions=[
                PlanningInputQuestion(
                    id="cost_completion",
                    header="成本补齐",
                    question=(
                        f"当前分析范围有 {stats.missing_pair_count} 条线路没有报价，"
                        "请选择补齐方式。"
                    ),
                    options=[
                        PlanningInputOption(
                            label="报价均值估算 (Recommended)",
                            description="按全部现有报价算出各层平均单位成本补齐，并明确标注为估算。",
                        ),
                        PlanningInputOption(
                            label="指定单位成本",
                            description="由你给出各层每单位需求的成本和币种，再补齐缺失线路。",
                        ),
                        PlanningInputOption(
                            label="上传报价",
                            description="暂停计算，等待上传覆盖当前仓库范围的完整报价。",
                        ),
                    ],
                )
            ],
        )
    )
    structured = result.model_dump(mode="json", by_alias=True)
    return CallToolResult(
        content=[TextContent(type="text", text=structured["summary"])],
        structuredContent=structured,
    )


def _prior_route_facts_match(
    prior: CostMatrix,
    route_matrix: ComposableRouteMatrix,
) -> bool:
    current = {
        (row.origin_id, row.destination_id, row.layer): row for row in route_matrix.rows
    }
    for row in prior.rows:
        fact = row.route_fact
        if fact is None:
            continue
        route = current.get((row.origin_id, row.destination_id, row.layer))
        if route is None:
            return False
        if (
            route.method != fact.method
            or route.tool_version != fact.tool_version
            or route.origin_longitude != fact.origin_longitude
            or route.origin_latitude != fact.origin_latitude
            or route.destination_longitude != fact.destination_longitude
            or route.destination_latitude != fact.destination_latitude
            or route.detour_coefficient != fact.detour_coefficient
            or route.average_speed_kph != fact.average_speed_kph
            or route.navigation_provider != fact.navigation_provider
            or route.navigation_profile != fact.navigation_profile
        ):
            return False
    return True


def register_tools(mcp, *, phase: str = "all") -> None:
    if phase in ("all", "all"):
        mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(plan_cost_matrix)
