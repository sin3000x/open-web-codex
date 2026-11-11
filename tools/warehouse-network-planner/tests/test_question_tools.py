from __future__ import annotations

from decimal import Decimal
from pathlib import Path

import pytest
from open_web_codex_provider import ProviderContractError
from supply_chain_planner.network import question_tools
from supply_chain_planner.network.matrix_models import ObservedQuoteMeanCostPolicy
from supply_chain_planner.network.optimization_models import ServiceCoverageConstraint
from supply_chain_planner.network.tool_runtime import _load_ready_network
from supply_chain_planner.shared.models import (
    PreparationRoleCounts,
    PreparedCandidateSummary,
    PreparedResolutionReady,
    PreparedResolutionSelectionRequired,
)
from test_network_planning_resource_tools import _context, _prepared_input, _runtime

CANDIDATE_ID = "WH-CANDIDATE-PADANG"
CANDIDATE_NAME = "Padang Candidate Cross Docking"
HAVERSINE = question_tools.HaversineRoutePolicy(detour_coefficient=1.3, average_speed_kph=40)
QUOTE_MEAN = ObservedQuoteMeanCostPolicy()


def _workspace(tmp_path: Path, monkeypatch):
    workspace, _store = _runtime(tmp_path, monkeypatch)
    return workspace, _context(workspace), _prepared_input(workspace)


def _resource_schemas(result) -> list[str]:
    return [block.title for block in result.content if block.type == "resource_link"]


def test_baseline_answers_in_one_call_with_a_per_warehouse_breakdown(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    result = question_tools.evaluate_baseline([24, 12], ctx, prepared_input_relative_path=path)

    structured = result.structuredContent
    assert structured["state"] == "ready"
    assert structured["basis"] == {
        "prepared_input_relative_path": path,
        "input_identity": structured["basis"]["input_identity"],
        "warehouse_scope": "existing_only",
        "objective": "min_cost",
        "route_method": "provided",
        "estimated_route_pair_count": 0,
        "cost_source": "quotes",
        "estimated_cost_lane_count": 0,
        "cost_rule_evidence": None,
        "cost_rule_evidence_path": None,
    }
    baseline = structured["baseline"]
    assert baseline["label"] == "actual_current"
    assert baseline["result_ref"]["resource_schema"] == "network_baseline.v2"
    assert [metric["target_hours"] for metric in baseline["coverage"]] == [12, 24]
    assert baseline["detail_target_hours"] == 24
    detail = baseline["coverage"][-1]
    warehouses = baseline["warehouses"]
    assert not baseline["warehouses_truncated"]
    assert sum(item["city_count"] for item in warehouses) == detail["total_city_count"]
    assert sum(item["covered_city_count"] for item in warehouses) == detail["covered_city_count"]
    assert sum(Decimal(item["covered_demand_quantity"]) for item in warehouses) == Decimal(
        detail["covered_demand"]
    )
    assert sum(item["last_mile_cost"] for item in warehouses) == pytest.approx(
        baseline["cost"]["last_mile"]
    )
    demands = [Decimal(item["demand_quantity"]) for item in warehouses]
    assert demands == sorted(demands, reverse=True)
    # Route and cost matrices are internal: only the reusable result is linked.
    assert _resource_schemas(result) == ["network_baseline.v2"]


def test_missing_prepared_input_is_a_typed_result(tmp_path: Path, monkeypatch) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)

    result = question_tools.evaluate_baseline([12], _context(workspace))

    assert result.structuredContent["state"] == "prepared_unavailable"
    assert result.structuredContent["next_action"] == "prepare_data"


def test_stale_prepared_input_is_not_reused(tmp_path: Path, monkeypatch) -> None:
    _workspace_root, ctx, _path = _workspace(tmp_path, monkeypatch)

    result = question_tools.evaluate_baseline([12], ctx)

    assert result.structuredContent["state"] == "prepared_unavailable"
    assert result.structuredContent["reasons"] == [
        "prepared candidate outputs/warehouse-network/prepared/prepared-network.json "
        "ignored: source_changed"
    ]


def test_single_reusable_prepared_input_is_resolved_for_the_question_roles(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)
    prepared, identity = _load_ready_network(path, ctx)
    requested: list[list[str]] = []

    def resolve(_root, paths, required_roles, country_code):
        requested.append([role.value for role in required_roles])
        assert paths == [path]
        assert country_code is None
        return PreparedResolutionReady(
            outcome="prepared_ready",
            operation="reused",
            summary="reused",
            next_action="handoff",
            retryable=False,
            prepared_input_relative_path=path,
            input_identity=identity,
            roles=prepared.roles,
            role_counts=prepared.role_counts,
            equivalent_count=1,
            warnings=[],
            warning_count=0,
            warnings_truncated=False,
        )

    monkeypatch.setattr(question_tools, "resolve_prepared_inputs", resolve)

    baseline = question_tools.evaluate_baseline([12], ctx)
    scenario = question_tools.evaluate_scenario([12], ctx, add_warehouses=[CANDIDATE_ID])

    assert baseline.structuredContent["state"] == "ready"
    assert baseline.structuredContent["basis"]["prepared_input_relative_path"] == path
    assert scenario.structuredContent["state"] == "needs_input"
    assert requested == [
        ["demand", "existing_warehouse", "route_quote"],
        ["demand", "existing_warehouse", "candidate_warehouse", "route_quote"],
    ]


def test_differing_prepared_inputs_require_a_user_choice(tmp_path: Path, monkeypatch) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)
    _prepared, identity = _load_ready_network(path, ctx)
    candidates = [
        PreparedCandidateSummary(
            prepared_input_relative_path=f"outputs/warehouse-network/prepared/{name}.json",
            input_identity=identity,
            roles=["demand"],
            role_counts=PreparationRoleCounts(
                demand=1,
                existing_warehouse=0,
                candidate_warehouse=0,
                current_assignment=0,
                route_quote=0,
                provided_route_fact=0,
            ),
            equivalent_count=1,
        )
        for name in ("a", "b")
    ]
    monkeypatch.setattr(
        question_tools,
        "resolve_prepared_inputs",
        lambda *_args: PreparedResolutionSelectionRequired(
            outcome="prepared_selection_required",
            summary="choose one",
            next_action="request_user_input",
            retryable=False,
            candidates=candidates,
            candidate_count=2,
            candidates_truncated=False,
        ),
    )

    result = question_tools.evaluate_baseline([12], ctx)

    assert result.structuredContent["state"] == "prepared_selection_required"
    assert [
        item["prepared_input_relative_path"] for item in result.structuredContent["candidates"]
    ] == [
        "outputs/warehouse-network/prepared/a.json",
        "outputs/warehouse-network/prepared/b.json",
    ]


def test_route_and_quote_gaps_are_asked_in_one_card(tmp_path: Path, monkeypatch) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    result = question_tools.evaluate_scenario(
        [12], ctx, add_warehouses=[CANDIDATE_NAME], prepared_input_relative_path=path
    )

    structured = result.structuredContent
    assert structured["state"] == "needs_input"
    assert structured["next_action"] == "request_user_input"
    assert [item["id"] for item in structured["questions"]] == [
        "route_completion",
        "cost_completion",
    ]
    assert structured["missing_route_pair_count"] > 0
    assert structured["missing_cost_lane_count"] > 0
    assert _resource_schemas(result) == []


def test_only_the_remaining_gap_is_asked_after_a_route_choice(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    result = question_tools.evaluate_scenario(
        [12],
        ctx,
        add_warehouses=[CANDIDATE_ID],
        prepared_input_relative_path=path,
        route_policy=HAVERSINE,
    )

    structured = result.structuredContent
    assert structured["state"] == "needs_input"
    assert [item["id"] for item in structured["questions"]] == ["cost_completion"]
    assert structured["missing_route_pair_count"] == 0


def test_scenario_returns_baseline_scenario_and_comparison_together(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    result = question_tools.evaluate_scenario(
        [12],
        ctx,
        add_warehouses=[CANDIDATE_NAME],
        prepared_input_relative_path=path,
        route_policy=HAVERSINE,
        cost_policy=QUOTE_MEAN,
    )

    structured = result.structuredContent
    assert structured["state"] == "ready"
    assert structured["basis"]["warehouse_scope"] == "existing_plus_candidates"
    assert structured["basis"]["route_method"] == "haversine"
    assert structured["basis"]["estimated_route_pair_count"] > 0
    assert structured["basis"]["cost_source"] == "observed_quote_mean"
    assert structured["basis"]["estimated_cost_lane_count"] > 0
    evidence = structured["basis"]["cost_rule_evidence"]
    assert evidence["schema_version"] == "warehouse_quote_mean_calculation.v1"
    assert {rule["layer"] for rule in evidence["rules"]} == {"last_mile", "linehaul"}
    assert structured["basis"]["cost_rule_evidence_path"] is None
    assert structured["added_warehouses"] == [
        {
            "warehouse_id": CANDIDATE_ID,
            "warehouse_name": CANDIDATE_NAME,
            "city_name": structured["added_warehouses"][0]["city_name"],
            "is_existing": False,
        }
    ]
    assert structured["removed_warehouses"] == []
    baseline, scenario = structured["baseline"], structured["scenario"]
    assert scenario["active_warehouse_count"] == baseline["active_warehouse_count"] + 1
    assert scenario["result_ref"]["resource_schema"] == "network_scenario.v2"
    assert structured["plan_comparison_ref"]["resource_schema"] == "network_plan_comparison.v2"
    comparison = structured["coverage"][0]
    assert comparison["before"] == baseline["coverage"][0]
    assert comparison["after"] == scenario["coverage"][0]
    assert structured["cost"]["before_total"] == pytest.approx(baseline["cost"]["total"])
    assert structured["cost"]["after_total"] == pytest.approx(scenario["cost"]["total"])
    assert _resource_schemas(result) == [
        "network_baseline.v2",
        "network_scenario.v2",
        "network_plan_comparison.v2",
    ]


def test_unknown_or_ambiguous_warehouse_names_are_not_guessed(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)
    prepared, _identity = _load_ready_network(path, ctx)

    result = question_tools.evaluate_scenario(
        [12], ctx, add_warehouses=["Padang"], prepared_input_relative_path=path
    )

    assert result.structuredContent["state"] == "warehouse_unresolved"
    assert result.structuredContent["unresolved"] == [
        {"identifier": "Padang", "reason": "not_found", "matches": []}
    ]
    duplicated = prepared.model_copy(
        update={
            "warehouses": [
                warehouse.model_copy(update={"warehouse_name": "Same Name"})
                for warehouse in prepared.warehouses[:2]
            ]
        }
    )
    resolved, unresolved = question_tools._resolve_warehouses(
        duplicated, ["Same Name", duplicated.warehouses[0].warehouse_id]
    )
    assert resolved == [duplicated.warehouses[0].warehouse_id]
    assert [(item.identifier, item.reason, len(item.matches)) for item in unresolved] == [
        ("Same Name", "ambiguous_name", 2)
    ]


def test_scenario_requires_a_warehouse_change(tmp_path: Path, monkeypatch) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    with pytest.raises(ProviderContractError, match="scenario_requires_a_warehouse_change"):
        question_tools.evaluate_scenario([12], ctx, prepared_input_relative_path=path)


def test_siting_requires_a_target_or_a_fixed_count(tmp_path: Path, monkeypatch) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    with pytest.raises(
        ProviderContractError, match="siting_requires_service_constraints_or_number_to_open"
    ):
        question_tools.find_sites_for_service_target([12], ctx, prepared_input_relative_path=path)


def test_siting_returns_the_plan_with_its_baseline_comparison(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    result = question_tools.find_sites_for_service_target(
        [12],
        ctx,
        number_to_open=1,
        prepared_input_relative_path=path,
        route_policy=HAVERSINE,
        cost_policy=QUOTE_MEAN,
        time_limit_seconds=30,
    )

    structured = result.structuredContent
    assert structured["state"] == "ready"
    assert structured["status"] in {"optimal", "feasible"}
    assert structured["basis"]["warehouse_scope"] == "all_warehouses"
    assert structured["selected_number_to_open"] == 1
    assert len(structured["opened_candidates"]) == 1
    assert structured["opened_candidates"][0]["is_existing"] is False
    assert structured["closed_existing"] == []
    solution, baseline = structured["solution"], structured["baseline"]
    assert solution["result_ref"]["resource_schema"] == "facility_location_solution.v4"
    assert solution["active_warehouse_count"] == baseline["active_warehouse_count"] + 1
    assert structured["plan_comparison_ref"]["resource_schema"] == "network_plan_comparison.v2"
    assert structured["coverage"][0]["after"] == solution["coverage"][0]
    assert structured["cost"]["after_total"] == pytest.approx(solution["cost"]["total"])
    assert _resource_schemas(result) == [
        "network_baseline.v2",
        "facility_location_solution.v4",
        "network_plan_comparison.v2",
    ]


def test_unreachable_service_target_is_a_valid_business_result(
    tmp_path: Path, monkeypatch
) -> None:
    _workspace_root, ctx, path = _workspace(tmp_path, monkeypatch)

    result = question_tools.find_sites_for_service_target(
        [0.01],
        ctx,
        service_constraints=[ServiceCoverageConstraint(target_hours=0.01, minimum_coverage=1.0)],
        prepared_input_relative_path=path,
        route_policy=HAVERSINE,
        cost_policy=QUOTE_MEAN,
        time_limit_seconds=30,
    )

    structured = result.structuredContent
    assert structured["state"] == "ready"
    assert structured["status"] == "infeasible"
    assert structured["solution"] is None
    assert structured["plan_comparison_ref"] is None
    assert structured["opened_candidates"] == []
    assert structured["baseline"]["result_ref"]["resource_schema"] == "network_baseline.v2"
    assert _resource_schemas(result) == ["network_baseline.v2"]
