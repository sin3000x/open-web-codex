from __future__ import annotations

import asyncio
import json
from pathlib import Path
from types import SimpleNamespace

import pytest
from _network_fixtures import (
    indonesia_current_assignments,
    indonesia_network_fixture,
    indonesia_provided_route_facts,
    indonesia_route_quotes,
)
from open_web_codex_provider import ProviderContractError, ResourceRef, ResourceStore
from supply_chain_planner.network import (
    analysis_tools,
    cost_tools,
    delivery_tools,
    facility_tools,
    route_tools,
    server,
    tool_runtime,
)
from supply_chain_planner.network.matrix_models import (
    AllWarehousesScope,
    CostCalculationPolicy,
    CostMatrix,
    DemandUnitCostRule,
    ExistingOnlyWarehouseScope,
    ExistingPlusCandidatesWarehouseScope,
    ExplicitCostPolicy,
    ObservedQuoteMeanCostEvidence,
    ObservedQuoteMeanCostPolicy,
)
from supply_chain_planner.network.matrix_models import (
    RouteMatrix as ComposableRouteMatrix,
)
from supply_chain_planner.network.models import RouteQuoteRecord
from supply_chain_planner.network.optimization_models import (
    ExactOpeningPolicy,
    KeepAllExistingWarehousePolicy,
    MinimumFeasibleOpeningPolicy,
    ScenarioSpec,
    ServiceCoverageConstraint,
)
from supply_chain_planner.network.solver import (
    SolverStageOutcome,
    SolverUnavailable,
    solve_p_median_stage,
)
from supply_chain_planner.shared.models import (
    ComparableNetworkResultRef,
    NetworkComparisonReportInput,
    PreparationRoleCounts,
    PreparedNetworkResource,
    PreparedSourceSelection,
)
from supply_chain_planner.shared.planning_input import derive_selected_source_identity
from supply_chain_planner.shared.resources import SupplyChainResources


def _runtime(tmp_path: Path, monkeypatch) -> tuple[Path, ResourceStore]:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    resources = SupplyChainResources(workspace, tmp_path / "profile")
    monkeypatch.setattr(tool_runtime, "_supply_chain_resources", resources)
    return workspace, resources.store


def _context(workspace: Path) -> SimpleNamespace:
    meta = SimpleNamespace(
        model_extra={"codex/sandbox-state-meta": {"sandboxCwd": workspace.as_uri()}}
    )
    return SimpleNamespace(request_context=SimpleNamespace(meta=meta))


def _prepared_input(workspace: Path, path: str = "prepared-network.json") -> str:
    fixture = indonesia_network_fixture()
    quotes = [
        RouteQuoteRecord.model_validate(item.model_dump(mode="python"))
        for item in indonesia_route_quotes()
    ]
    if Path(path).parent == Path("."):
        path = f"outputs/warehouse-network/prepared/{Path(path).name}"
    (workspace / "fixture").mkdir(exist_ok=True)
    (workspace / "fixture/input.csv").write_text("city_id,city_name,demand_quantity\n", encoding="utf-8")
    source_selections = [
        PreparedSourceSelection(
            relative_path="fixture/input.csv",
            unit_ref="table",
            role=role,
            mappings=sorted(
                mappings,
                key=lambda mapping: (
                    mapping["target_field"],
                    mapping["source_field"],
                    mapping["transform"],
                    str(mapping.get("factor")),
                ),
            ),
            raw_content_sha256="0" * 64,
        )
        for role, mappings in (
            (
                "demand",
                [
                    {"source_field": "city_id", "target_field": "city_id", "transform": "trim"},
                    {"source_field": "city_name", "target_field": "city_name", "transform": "trim"},
                    {"source_field": "demand_quantity", "target_field": "demand_quantity", "transform": "parse_integer"},
                ],
            ),
            (
                "existing_warehouse",
                [
                    {"source_field": "city_id", "target_field": "city_id", "transform": "trim"},
                    {"source_field": "city_name", "target_field": "city_name", "transform": "trim"},
                    {"source_field": "warehouse_id", "target_field": "warehouse_id", "transform": "trim"},
                    {"source_field": "warehouse_name", "target_field": "warehouse_name", "transform": "trim"},
                    {"source_field": "warehouse_type", "target_field": "warehouse_type", "transform": "normalize_warehouse_type"},
                ],
            ),
            (
                "candidate_warehouse",
                [
                    {"source_field": "city_id", "target_field": "city_id", "transform": "trim"},
                    {"source_field": "city_name", "target_field": "city_name", "transform": "trim"},
                    {"source_field": "warehouse_id", "target_field": "warehouse_id", "transform": "trim"},
                    {"source_field": "warehouse_name", "target_field": "warehouse_name", "transform": "trim"},
                    {"source_field": "warehouse_type", "target_field": "warehouse_type", "transform": "normalize_warehouse_type"},
                ],
            ),
            (
                "current_assignment",
                [
                    {"source_field": "demand_city_id", "target_field": "demand_city_id", "transform": "trim"},
                    {"source_field": "serving_warehouse_id", "target_field": "serving_warehouse_id", "transform": "trim"},
                ],
            ),
            (
                "route_quote",
                [
                    {"source_field": "origin_id", "target_field": "origin_id", "transform": "trim"},
                    {"source_field": "destination_id", "target_field": "destination_id", "transform": "trim"},
                    {"source_field": "price_per_vehicle", "target_field": "price_per_vehicle", "transform": "parse_decimal"},
                ],
            ),
        )
    ]
    role_counts = PreparationRoleCounts(
        demand=len(fixture.demand),
        existing_warehouse=sum(item.is_existing for item in fixture.warehouses),
        candidate_warehouse=sum(not item.is_existing for item in fixture.warehouses),
        current_assignment=len(indonesia_current_assignments()),
        route_quote=len(quotes),
        provided_route_fact=len(indonesia_provided_route_facts()),
    )
    prepared = PreparedNetworkResource(
        country_code="ID",
        state="ready",
        demand_cities=fixture.demand,
        warehouses=fixture.warehouses,
        current_assignments=indonesia_current_assignments(),
        route_quotes=quotes,
        provided_route_facts=indonesia_provided_route_facts(),
        issue_count=0,
        issues=[],
        issues_truncated=False,
        roles=["candidate_warehouse", "current_assignment", "demand", "existing_warehouse", "route_quote"],
        source_selections=sorted(
            source_selections,
            key=lambda item: (item.relative_path, item.unit_ref, item.role.value),
        ),
        selected_source_identity=derive_selected_source_identity("ID", source_selections, None),
        role_counts=role_counts,
    )
    target = workspace / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(prepared.model_dump_json(by_alias=True), encoding="utf-8")
    return path


def _ref(result) -> ResourceRef:
    assert result.structuredContent is not None
    return ResourceRef.model_validate(result.structuredContent["resource_ref"])


def _cost_policy() -> CostCalculationPolicy:
    return CostCalculationPolicy(
        rules=[
            DemandUnitCostRule(
                layer=layer,
                currency="IDR",
                fixed_cost_per_demand_unit=40_000,
                cost_per_km_per_demand_unit=1_500,
            )
            for layer in ("last_mile", "linehaul")
        ]
    )


def _pmedian_fixture(tmp_path: Path, monkeypatch):
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    costs = _ref(
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=routes,
        )
    )
    return workspace, ctx, prepared_path, routes, costs


def test_network_planning_tools_use_workspace_input_and_keep_compute_results_as_resources() -> None:
    tools = {tool.name: tool for tool in asyncio.run(server.mcp.list_tools())}
    for name in (
        "prepare_route_matrix",
        "create_navigation_matrix_request",
        "import_navigation_matrix",
        "plan_cost_matrix",
        "prepare_network_distribution_map",
        "prepare_network_coverage_map",
        "evaluate_network_baseline",
        "assess_facility_change",
        "solve_p_median",
        "compare_network_scenarios",
    ):
        schema = tools[name].inputSchema
        assert "prepared_input_relative_path" in schema["required"]
        assert "normalized_input_ref" not in schema["properties"]

    route_output = tools["prepare_route_matrix"].outputSchema
    route_variants = route_output["oneOf"]
    assert len(route_variants) == 2
    assert {variant["$ref"].rsplit("/", 1)[-1] for variant in route_variants} == {
        "RouteMatrixNeedsInput",
        "RouteMatrixReady",
    }
    assert "prepared_input_relative_path" in tools["prepare_route_matrix"].inputSchema["properties"]
    assert "cost_policy" in tools["plan_cost_matrix"].inputSchema["properties"]
    assert "calculation_policy" not in tools["plan_cost_matrix"].inputSchema["properties"]
    assert "calculation_rule_evidence" in tools["plan_cost_matrix"].outputSchema["properties"]
    assert "calculation_rule_evidence_path" in tools["plan_cost_matrix"].outputSchema["properties"]
    assert "coverage" in tools["solve_p_median"].outputSchema["properties"]
    assert "opening_policy" in tools["solve_p_median"].inputSchema["properties"]
    assert "number_to_open" not in tools["solve_p_median"].inputSchema["properties"]
    assert "solver_stages" in tools["solve_p_median"].outputSchema["properties"]


def test_provided_candidate_route_gap_returns_typed_needs_input_without_resource(
    tmp_path: Path,
    monkeypatch,
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)

    result = route_tools.prepare_route_matrix(
        prepared_path,
        "provided",
        ctx,
        warehouse_scope=ExistingPlusCandidatesWarehouseScope(
            candidate_ids=["WH-CANDIDATE-BALIKPAPAN"]
        ),
    )

    assert result.structuredContent is not None
    payload = result.structuredContent
    assert payload["state"] == "needs_input"
    assert payload["next_action"] == "request_user_input"
    assert payload["retryable"] is False
    assert payload["missing_pair_count"] == 51
    assert len(payload["missing_pairs"]) == 20
    assert payload["missing_pairs_truncated"] is True
    assert payload["questions"][0]["id"] == "route_completion"
    assert payload["questions"][0]["options"][0]["label"].endswith("(Recommended)")
    assert "resource_ref" not in payload


def test_analysis_tools_reject_incomplete_route_resources_defensively(
    tmp_path: Path,
    monkeypatch,
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    prepared, input_identity = tool_runtime._load_ready_network(prepared_path, ctx)
    complete = route_tools._build_provided_route_matrix(
        prepared.demand_cities,
        prepared.warehouses,
        prepared.provided_route_facts,
        warehouse_scope=ExistingOnlyWarehouseScope(),
        input_identity=input_identity,
    )
    incomplete = complete.model_copy(update={"rows": complete.rows[:-1]})
    incomplete_ref = _ref(
        tool_runtime._runtime().publish(
            incomplete.schema_version,
            incomplete,
            "test incomplete route matrix",
        )
    )

    with pytest.raises(ProviderContractError, match="baseline_route_matrix_incomplete"):
        analysis_tools.evaluate_network_baseline(
            prepared_path,
            incomplete_ref,
            "min_time",
            [12],
            ctx,
        )

    scenario = ScenarioSpec(
        add_warehouse_ids=[],
        remove_warehouse_ids=[],
        relocations=[],
        objective="min_time",
        service_targets=[12],
    )
    with pytest.raises(ProviderContractError, match="scenario_route_matrix_incomplete"):
        facility_tools._load_facility_scenario_inputs(
            prepared_path,
            incomplete_ref,
            None,
            scenario,
            ctx,
        )


def test_comparable_loader_rejects_facility_without_assignment(monkeypatch) -> None:
    class RuntimeWithUnavailableFacility:
        def load_model(self, *_args, **_kwargs):
            return SimpleNamespace(assignment=None)

    monkeypatch.setattr(
        tool_runtime,
        "_runtime",
        lambda: RuntimeWithUnavailableFacility(),
    )
    ref = ComparableNetworkResultRef.model_validate(
        {
            "type": "mcp_resource",
            "server": "supply_chain",
            "uri": "supply-chain://resources/facility",
            "resource_schema": "facility_location_solution.v4",
        }
    )
    with pytest.raises(ProviderContractError, match="comparable_assignment_unavailable"):
        tool_runtime._load_comparable_resource(ref)


def test_route_prior_requires_exact_method_scope_and_parameters(tmp_path: Path, monkeypatch) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    prior = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    prior_matrix = tool_runtime._runtime().load_model(
        prior,
        "route_matrix.v3",
        ComposableRouteMatrix,
    )

    def publish_prior(value: ComposableRouteMatrix) -> ResourceRef:
        return _ref(tool_runtime._runtime().publish(value.schema_version, value, "test prior"))

    identity_mismatch = publish_prior(
        prior_matrix.model_copy(
            update={
                "input_identity": prior_matrix.input_identity.model_copy(
                    update={"content_sha256": "f" * 64}
                )
            }
        )
    )
    with pytest.raises(ProviderContractError, match="route_prior_input_identity_mismatch"):
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
            prior_route_matrix_ref=identity_mismatch,
        )
    warehouse_set_mismatch = publish_prior(
        prior_matrix.model_copy(update={"warehouse_ids": prior_matrix.warehouse_ids[:-1]})
    )
    with pytest.raises(ProviderContractError, match="route_prior_warehouse_set_mismatch"):
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
            prior_route_matrix_ref=warehouse_set_mismatch,
        )
    with pytest.raises(ProviderContractError, match="route_prior_method_mismatch"):
        route_tools.prepare_route_matrix(
            prepared_path,
            "provided",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            prior_route_matrix_ref=prior,
        )
    with pytest.raises(ProviderContractError, match="route_prior_scope_mismatch"):
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=ExistingOnlyWarehouseScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
            prior_route_matrix_ref=prior,
        )
    with pytest.raises(ProviderContractError, match="route_prior_parameters_mismatch"):
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.3,
            average_speed_kph=42,
            prior_route_matrix_ref=prior,
        )


def test_plan_cost_matrix_derives_bounded_full_quote_means(
    tmp_path: Path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )

    calculation_dir = workspace / "outputs/warehouse-network/calculations"
    calculation_dir.mkdir(parents=True)
    derived = cost_tools.plan_cost_matrix(
        prepared_path,
        AllWarehousesScope(),
        ctx,
        cost_policy=ObservedQuoteMeanCostPolicy(),
        route_matrix_ref=routes,
    )
    assert derived.structuredContent is not None
    evidence_path = "outputs/warehouse-network/calculations/quote-means.json"
    (workspace / evidence_path).write_text(
        json.dumps(derived.structuredContent["calculation_rule_evidence"]),
        encoding="utf-8",
    )

    result = cost_tools.plan_cost_matrix(
        prepared_path,
        AllWarehousesScope(),
        ctx,
        cost_policy=ObservedQuoteMeanCostPolicy(),
        route_matrix_ref=routes,
        quote_mean_evidence_relative_path=evidence_path,
    )

    assert result.structuredContent is not None
    evidence = result.structuredContent["calculation_rule_evidence"]
    assert result.structuredContent["calculation_rule_source"] == "observed_quote_mean"
    assert result.structuredContent["input_identity"]["schema_version"] == (
        "prepared_network_input.v2"
    )
    assert evidence["considered_quote_count"] == 580
    assert evidence["total_quote_count"] == 580
    assert {rule["layer"]: rule["quote_count"] for rule in evidence["rules"]} == {
        "last_mile": 550,
        "linehaul": 30,
    }
    matrix = tool_runtime._runtime().load_model(_ref(result), "cost_matrix.v3", CostMatrix)
    assert matrix.missing_routes == []
    assert matrix.calculation_rule_source == "observed_quote_mean"
    assert matrix.calculation_rule_evidence_path == evidence_path
    assert matrix.calculation_rule_evidence == ObservedQuoteMeanCostEvidence.model_validate(evidence)

    second_evidence_path = "outputs/warehouse-network/calculations/quote-means-copy.json"
    (workspace / second_evidence_path).write_text(
        json.dumps(derived.structuredContent["calculation_rule_evidence"]),
        encoding="utf-8",
    )
    reused = cost_tools.plan_cost_matrix(
        prepared_path,
        AllWarehousesScope(),
        ctx,
        cost_policy=ObservedQuoteMeanCostPolicy(),
        route_matrix_ref=routes,
        prior_cost_matrix_ref=_ref(result),
        quote_mean_evidence_relative_path=second_evidence_path,
    )
    assert reused.structuredContent is not None
    assert reused.structuredContent["reused_pair_count"] == 1168
    prior_matrix = tool_runtime._runtime().load_model(
        _ref(result),
        "cost_matrix.v3",
        CostMatrix,
    )

    def publish_cost_prior(value: CostMatrix) -> ResourceRef:
        return _ref(tool_runtime._runtime().publish(value.schema_version, value, "test prior"))

    identity_mismatch = publish_cost_prior(
        prior_matrix.model_copy(
            update={
                "input_identity": prior_matrix.input_identity.model_copy(
                    update={"content_sha256": "e" * 64}
                )
            }
        )
    )
    with pytest.raises(ProviderContractError, match="cost_prior_input_identity_mismatch"):
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ObservedQuoteMeanCostPolicy(),
            route_matrix_ref=routes,
            prior_cost_matrix_ref=identity_mismatch,
        )
    warehouse_set_mismatch = publish_cost_prior(
        prior_matrix.model_copy(update={"warehouse_ids": prior_matrix.warehouse_ids[:-1]})
    )
    with pytest.raises(ProviderContractError, match="cost_prior_warehouse_set_mismatch"):
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ObservedQuoteMeanCostPolicy(),
            route_matrix_ref=routes,
            prior_cost_matrix_ref=warehouse_set_mismatch,
        )
    with pytest.raises(ProviderContractError, match="cost_prior_rule_mismatch"):
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=routes,
            prior_cost_matrix_ref=_ref(result),
        )


def test_plan_cost_matrix_rejects_unbound_quote_mean_evidence(
    tmp_path: Path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    derived = cost_tools.plan_cost_matrix(
        prepared_path,
        AllWarehousesScope(),
        ctx,
        cost_policy=ObservedQuoteMeanCostPolicy(),
        route_matrix_ref=routes,
    )
    assert derived.structuredContent is not None
    payload = dict(derived.structuredContent["calculation_rule_evidence"])
    payload["total_quote_count"] = 579
    calculation_dir = workspace / "outputs/warehouse-network/calculations"
    calculation_dir.mkdir(parents=True)
    evidence_path = "outputs/warehouse-network/calculations/unbound.json"
    (workspace / evidence_path).write_text(json.dumps(payload), encoding="utf-8")
    with pytest.raises(ProviderContractError, match="cost_evidence_mismatch"):
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ObservedQuoteMeanCostPolicy(),
            route_matrix_ref=routes,
            quote_mean_evidence_relative_path=evidence_path,
        )


def test_plan_cost_matrix_rejects_duplicate_quote_mean_layers(
    tmp_path: Path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    derived = cost_tools.plan_cost_matrix(
        prepared_path,
        AllWarehousesScope(),
        ctx,
        cost_policy=ObservedQuoteMeanCostPolicy(),
        route_matrix_ref=routes,
    )
    assert derived.structuredContent is not None
    payload = dict(derived.structuredContent["calculation_rule_evidence"])
    payload["rules"] = [payload["rules"][0], payload["rules"][0]]
    calculation_dir = workspace / "outputs/warehouse-network/calculations"
    calculation_dir.mkdir(parents=True)
    evidence_path = "outputs/warehouse-network/calculations/duplicate-layer.json"
    (workspace / evidence_path).write_text(json.dumps(payload), encoding="utf-8")
    with pytest.raises(ProviderContractError, match="cost_evidence_mismatch"):
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ObservedQuoteMeanCostPolicy(),
            route_matrix_ref=routes,
            quote_mean_evidence_relative_path=evidence_path,
        )


def test_prepared_input_drives_baseline_optimization_map_and_report(
    tmp_path: Path, monkeypatch
) -> None:
    workspace, store = _runtime(tmp_path, monkeypatch)
    (workspace / "outputs").mkdir()
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)

    routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    costs = _ref(
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=routes,
        )
    )
    baseline_routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=ExistingOnlyWarehouseScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    baseline_costs = _ref(
        cost_tools.plan_cost_matrix(
            prepared_path,
            ExistingOnlyWarehouseScope(),
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=baseline_routes,
        )
    )
    baseline = _ref(
        analysis_tools.evaluate_network_baseline(
            prepared_path,
            baseline_routes,
            "min_cost",
            [6, 12, 18],
            ctx,
            coverage_mode="actual_current",
            cost_matrix_ref=baseline_costs,
        )
    )
    with pytest.raises(ProviderContractError, match="baseline_route_scope_mismatch"):
        analysis_tools.evaluate_network_baseline(
            prepared_path,
            routes,
            "min_cost",
            [6, 12, 18],
            ctx,
            coverage_mode="actual_current",
            cost_matrix_ref=costs,
        )
    facility_result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [6, 12, 18],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert facility_result.structuredContent is not None
    assert facility_result.structuredContent["status"] == "optimal"
    assert facility_result.structuredContent["selected_number_to_open"] == 2
    assert facility_result.structuredContent["minimum_number_to_open_proven"] is True
    assert [stage["kind"] for stage in facility_result.structuredContent["solver_stages"]] == [
        "minimum_openings",
        "minimum_cost",
    ]
    assert len(facility_result.structuredContent["solver_stages"]) <= 2
    assert [metric["target_hours"] for metric in facility_result.structuredContent["coverage"]] == [
        6.0,
        12.0,
        18.0,
    ]
    facility = _ref(facility_result)
    facility_comparable = ComparableNetworkResultRef.model_validate(
        facility.model_dump(mode="json")
    )
    prepared_payload = json.loads((workspace / prepared_path).read_text(encoding="utf-8"))
    existing_ids = {
        item["warehouse_id"] for item in prepared_payload["warehouses"] if item["is_existing"]
    }
    facility_view = tool_runtime._load_comparable_resource(facility)
    before_candidate_ids = sorted(set(facility_view.active_warehouse_ids) - existing_ids)
    new_candidate_id = next(
        item["warehouse_id"]
        for item in prepared_payload["warehouses"]
        if not item["is_existing"] and item["warehouse_id"] not in before_candidate_ids
    )
    exact_candidate_ids = sorted([*before_candidate_ids, new_candidate_id])
    exact_scope = ExistingPlusCandidatesWarehouseScope(candidate_ids=exact_candidate_ids)
    missing_scope = ExistingPlusCandidatesWarehouseScope(candidate_ids=[new_candidate_id])
    exact_routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=exact_scope,
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    exact_costs = _ref(
        cost_tools.plan_cost_matrix(
            prepared_path,
            exact_scope,
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=exact_routes,
        )
    )
    missing_routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=missing_scope,
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    missing_costs = _ref(
        cost_tools.plan_cost_matrix(
            prepared_path,
            missing_scope,
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=missing_routes,
        )
    )
    scenario = {
        "add_warehouse_ids": [new_candidate_id],
        "remove_warehouse_ids": [],
        "relocations": [],
        "objective": "min_cost",
        "service_targets": [12],
    }
    with pytest.raises(ProviderContractError, match="scenario_route_scope_mismatch"):
        facility_tools.assess_facility_change(
            prepared_path,
            routes,
            facility_comparable,
            ScenarioSpec.model_validate(scenario),
            ctx,
            cost_matrix_ref=costs,
        )
    with pytest.raises(ProviderContractError, match="scenario_route_scope_mismatch"):
        facility_tools.assess_facility_change(
            prepared_path,
            missing_routes,
            facility_comparable,
            ScenarioSpec.model_validate(scenario),
            ctx,
            cost_matrix_ref=missing_costs,
        )
    assessed = facility_tools.assess_facility_change(
        prepared_path,
        exact_routes,
        facility_comparable,
        ScenarioSpec.model_validate(scenario),
        ctx,
        cost_matrix_ref=exact_costs,
    )
    assert assessed.structuredContent is not None
    with pytest.raises(
        ProviderContractError, match="p_median_route_matrix_requires_all_warehouses_scope"
    ):
        facility_tools.solve_p_median(
            prepared_path,
            exact_routes,
            exact_costs,
            ExactOpeningPolicy(number_to_open=0),
            KeepAllExistingWarehousePolicy(),
            [12],
            30,
            ctx,
        )
    comparison = _ref(
        analysis_tools.compare_network_scenarios(
            prepared_path,
            baseline,
            facility,
            [6, 12, 18],
            ctx,
        )
    )
    assert comparison.resource_schema == "network_plan_comparison.v2"

    map_result = delivery_tools.prepare_network_comparison_map(comparison, ctx)
    assert map_result.structuredContent is not None
    assert (
        map_result.structuredContent["data_ref"]["resource_schema"]
        == "network_comparison_geojson.v2"
    )

    coverage = delivery_tools.prepare_network_coverage_map(prepared_path, facility, 12, ctx)
    assert coverage.structuredContent is not None
    assert coverage.structuredContent["feature_count"] > 0
    assert coverage.structuredContent["service_target_hours"] == 12
    coverage_resource_id = coverage.structuredContent["data_ref"]["uri"].rsplit("/", 1)[-1]
    coverage_geojson = json.loads(store.read(coverage_resource_id))
    assert coverage_geojson["service_target_hours"] == 12
    demand_features = [
        feature
        for feature in coverage_geojson["features"]
        if feature["properties"]["kind"] == "demand"
    ]
    statuses = [feature["properties"]["service_status"] for feature in demand_features]
    assert set(statuses) <= {"attained", "missed", "unassigned"}
    target_metric = next(
        metric for metric in facility_result.structuredContent["coverage"]
        if metric["target_hours"] == 12
    )
    assert statuses.count("attained") == target_metric["covered_city_count"]
    attained_demand = sum(
        feature["properties"]["demand_quantity"]
        for feature in demand_features
        if feature["properties"]["service_status"] == "attained"
    )
    assert attained_demand == float(target_metric["covered_demand"])
    with pytest.raises(ProviderContractError, match="coverage_map_service_target_unavailable"):
        delivery_tools.prepare_network_coverage_map(prepared_path, facility, 7, ctx)

    report = delivery_tools.publish_network_planning_report(
        NetworkComparisonReportInput(plan_comparison_ref=comparison),
        "outputs/warehouse-network/deliverables/network-report.md",
        ctx,
    )
    assert report.structuredContent is not None
    assert (workspace / "outputs/warehouse-network/deliverables/network-report.md").is_file()
    assert (
        "network_planning_report_markdown.v2"
        in (workspace / "outputs/warehouse-network/deliverables/network-report.md").read_text()
    )


def test_minimum_feasible_requires_service_constraints(tmp_path: Path, monkeypatch) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    costs = _ref(
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=routes,
        )
    )
    with pytest.raises(
        ProviderContractError,
        match="p_median_minimum_feasible_requires_service_constraints",
    ):
        facility_tools.solve_p_median(
            prepared_path,
            routes,
            costs,
            MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
            KeepAllExistingWarehousePolicy(),
            [12],
            30,
            ctx,
        )


def test_minimum_feasible_retains_stage1_when_stage2_times_out(
    tmp_path: Path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _prepared_input(workspace)
    routes = _ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    costs = _ref(
        cost_tools.plan_cost_matrix(
            prepared_path,
            AllWarehousesScope(),
            ctx,
            cost_policy=ExplicitCostPolicy(rules=_cost_policy().rules),
            route_matrix_ref=routes,
        )
    )
    real_stage_solver = solve_p_median_stage
    calls: list[str] = []

    def fake_stage(*args, **kwargs):
        kind = kwargs.get("stage_kind", args[4])
        calls.append(kind)
        if kind == "minimum_cost":
            return SolverStageOutcome(
                kind,
                "timeout",
                "not_available",
                None,
                None,
                None,
                None,
                0,
            )
        outcome = real_stage_solver(*args, **kwargs)
        return SolverStageOutcome(
            kind,
            "feasible",
            "feasible_only",
            outcome.result,
            outcome.selected_number_to_open,
            outcome.objective_value,
            outcome.best_bound,
            outcome.branches,
        )

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", fake_stage)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert result.structuredContent is not None
    assert calls == ["minimum_openings", "minimum_cost"]
    assert result.structuredContent["status"] == "feasible"
    assert result.structuredContent["optimality"] == "feasible_only"
    assert result.structuredContent["selected_number_to_open"] == 2
    assert [stage["status"] for stage in result.structuredContent["solver_stages"]] == [
        "feasible",
        "timeout",
    ]


@pytest.mark.parametrize("stage2_status", ["feasible", "timeout", "unavailable"])
def test_minimum_feasible_stage2_statuses_retain_stage1(
    tmp_path: Path, monkeypatch, stage2_status: str
) -> None:
    _workspace, ctx, prepared_path, routes, costs = _pmedian_fixture(tmp_path, monkeypatch)
    real_stage_solver = solve_p_median_stage
    calls: list[str] = []

    def fake_stage(*args, **kwargs):
        kind = args[4]
        calls.append(kind)
        if kind == "minimum_openings":
            outcome = real_stage_solver(*args, **kwargs)
            return outcome
        if stage2_status == "feasible":
            first = real_stage_solver(
                args[0],
                args[1],
                args[2],
                args[3],
                "minimum_cost",
                args[5],
                args[6],
                args[7],
                args[8],
                args[9],
                args[10],
            )
            return SolverStageOutcome(
                kind,
                "feasible",
                "feasible_only",
                first.result,
                first.selected_number_to_open,
                first.objective_value,
                first.best_bound,
                first.branches,
            )
        return SolverStageOutcome(
            kind,
            stage2_status,
            "not_available",
            None,
            None,
            None,
            None,
            0,
        )

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", fake_stage)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert result.structuredContent is not None
    assert calls == ["minimum_openings", "minimum_cost"]
    assert result.structuredContent["status"] == "feasible"
    assert result.structuredContent["optimality"] == "feasible_only"
    assert result.structuredContent["minimum_number_to_open_proven"] is True
    assert len(result.structuredContent["solver_stages"]) == 2
    assert result.structuredContent["selected_number_to_open"] == 2


def test_minimum_feasible_stage1_feasible_stage2_optimal_is_not_proven(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace, ctx, prepared_path, routes, costs = _pmedian_fixture(tmp_path, monkeypatch)
    real_stage_solver = solve_p_median_stage
    calls: list[str] = []

    def fake_stage(*args, **kwargs):
        kind = args[4]
        calls.append(kind)
        outcome = real_stage_solver(*args, **kwargs)
        if kind == "minimum_openings":
            return SolverStageOutcome(
                kind,
                "feasible",
                "feasible_only",
                outcome.result,
                outcome.selected_number_to_open,
                outcome.objective_value,
                outcome.best_bound,
                outcome.branches,
            )
        return outcome

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", fake_stage)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert result.structuredContent is not None
    assert calls == ["minimum_openings", "minimum_cost"]
    assert result.structuredContent["status"] == "feasible"
    assert result.structuredContent["optimality"] == "feasible_only"
    assert result.structuredContent["minimum_number_to_open_proven"] is False


@pytest.mark.parametrize("stage1_status", ["infeasible", "timeout", "unavailable"])
def test_minimum_feasible_stage1_terminal_does_not_run_stage2(
    tmp_path: Path, monkeypatch, stage1_status: str
) -> None:
    _workspace, ctx, prepared_path, routes, costs = _pmedian_fixture(tmp_path, monkeypatch)
    calls: list[str] = []

    def fake_stage(*args, **kwargs):
        calls.append(args[4])
        return SolverStageOutcome(
            "minimum_openings",
            stage1_status,
            "proven" if stage1_status == "infeasible" else "not_available",
            None,
            None,
            None,
            None,
            0,
        )

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", fake_stage)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert result.structuredContent is not None
    assert calls == ["minimum_openings"]
    assert result.structuredContent["status"] == stage1_status
    assert result.structuredContent["optimality"] == "not_available"
    assert result.structuredContent["selected_number_to_open"] is None
    assert len(result.structuredContent["solver_stages"]) == 1


def test_minimum_feasible_stage1_solver_unavailable_is_typed_terminal(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace, ctx, prepared_path, routes, costs = _pmedian_fixture(tmp_path, monkeypatch)

    def unavailable(*_args, **_kwargs):
        raise SolverUnavailable("test unavailable")

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", unavailable)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert result.structuredContent is not None
    assert result.structuredContent["status"] == "unavailable"
    assert result.structuredContent["solver_stages"][0]["status"] == "unavailable"


def test_exact_policy_runs_one_minimum_cost_stage(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace, ctx, prepared_path, routes, costs = _pmedian_fixture(tmp_path, monkeypatch)
    real_stage_solver = solve_p_median_stage
    calls: list[str] = []

    def fake_stage(*args, **kwargs):
        calls.append(args[4])
        return real_stage_solver(*args, **kwargs)

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", fake_stage)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        ExactOpeningPolicy(number_to_open=2),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
    )
    assert result.structuredContent is not None
    assert calls == ["minimum_cost"]
    assert result.structuredContent["status"] == "optimal"
    assert result.structuredContent["optimality"] == "proven"
    assert result.structuredContent["selected_number_to_open"] == 2
    assert result.structuredContent["minimum_number_to_open_proven"] is False
    assert len(result.structuredContent["solver_stages"]) == 1


def test_solver_stages_share_decreasing_remaining_budget(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace, ctx, prepared_path, routes, costs = _pmedian_fixture(tmp_path, monkeypatch)
    real_stage_solver = solve_p_median_stage
    remaining: list[float] = []

    def fake_stage(*args, **kwargs):
        remaining.append(args[9])
        return real_stage_solver(*args, **kwargs)

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", fake_stage)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert result.structuredContent is not None
    assert len(remaining) == 2
    assert 0 < remaining[1] < remaining[0] <= 30


def test_budget_exhaustion_records_synthetic_stage2_timeout(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace, ctx, prepared_path, routes, costs = _pmedian_fixture(tmp_path, monkeypatch)
    clock = iter([0.0, 0.0, 100.0])
    monkeypatch.setattr(facility_tools.time, "monotonic", lambda: next(clock))
    calls: list[str] = []

    def fake_stage(*args, **kwargs):
        calls.append(args[4])
        return solve_p_median_stage(*args, **kwargs)

    monkeypatch.setattr(facility_tools, "solve_p_median_stage", fake_stage)
    result = facility_tools.solve_p_median(
        prepared_path,
        routes,
        costs,
        MinimumFeasibleOpeningPolicy(maximum_number_to_open=3),
        KeepAllExistingWarehousePolicy(),
        [12],
        30,
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=12, minimum_coverage=0.9)],
    )
    assert result.structuredContent is not None
    assert calls == ["minimum_openings"]
    assert result.structuredContent["status"] == "feasible"
    assert result.structuredContent["optimality"] == "feasible_only"
    assert result.structuredContent["minimum_number_to_open_proven"] is True
    assert [stage["status"] for stage in result.structuredContent["solver_stages"]] == [
        "optimal",
        "timeout",
    ]


def test_scenario_validation_stops_before_loading_missing_workspace_inputs() -> None:
    unknown = ResourceRef(
        server="supply_chain",
        uri="supply-chain://resources/not-loaded",
        resource_schema="route_matrix.v3",
    )
    fake_context = SimpleNamespace(
        request_context=SimpleNamespace(meta=SimpleNamespace(model_extra={}))
    )
    with pytest.raises(ProviderContractError, match="scenario_service_targets_invalid"):
        facility_tools._load_facility_scenario_inputs(
            "not-loaded.json",
            unknown,
            None,
            ScenarioSpec(
                objective="min_time",
                service_targets=[float(target) for target in range(1, 34)],
            ),
            fake_context,
        )
