from __future__ import annotations

import pytest
from _network_fixtures import (
    TEST_INPUT_IDENTITY,
    indonesia_network_fixture,
    indonesia_provided_route_facts,
    network_case,
)
from supply_chain_planner.network.matrix import (
    build_haversine_route_matrix,
    build_provided_route_matrix,
    build_route_matrix_with_reuse,
    canonical_warehouse_scope,
    plan_route_matrix,
    register_navigation_route_matrix,
    validate_route_matrix,
)
from supply_chain_planner.network.matrix_models import (
    AllWarehousesScope,
    ExistingOnlyWarehouseScope,
    ExistingPlusCandidatesWarehouseScope,
    RouteMatrixRow,
)


def _with_identity(function):
    def call(*args, **kwargs):
        kwargs.setdefault("input_identity", TEST_INPUT_IDENTITY)
        return function(*args, **kwargs)

    return call


build_haversine_route_matrix = _with_identity(build_haversine_route_matrix)
build_provided_route_matrix = _with_identity(build_provided_route_matrix)
build_route_matrix_with_reuse = _with_identity(build_route_matrix_with_reuse)
plan_route_matrix = _with_identity(plan_route_matrix)
register_navigation_route_matrix = _with_identity(register_navigation_route_matrix)


def test_haversine_plan_and_matrix_use_explicit_parameters() -> None:
    case = network_case()
    plan = plan_route_matrix(
        case.demand,
        case.warehouses,
        "haversine",
        1.2,
        40,
        warehouse_scope=AllWarehousesScope(),
    )
    matrix = build_haversine_route_matrix(case.demand, case.warehouses, 1.2, 40)

    assert plan.route_count == 8
    assert plan.estimated_billable_calls == 0
    assert len(matrix.rows) == plan.route_count
    assert matrix.rows[0].duration_hours > 0
    assert matrix.stats.detour_coefficient == 1.2


def test_haversine_requires_speed_and_detour_coefficient() -> None:
    with pytest.raises(ValueError, match="haversine_requires"):
        case = network_case()
        plan_route_matrix(
            case.demand,
            case.warehouses,
            "haversine",
            None,
            40,
            warehouse_scope=AllWarehousesScope(),
        )


def test_existing_plus_candidates_scope_is_exact_and_canonical() -> None:
    case = network_case()
    scope = ExistingPlusCandidatesWarehouseScope(candidate_ids=["candidate-c"])
    matrix = build_haversine_route_matrix(
        case.demand,
        case.warehouses,
        1.2,
        40,
        warehouse_scope=scope,
    )
    assert matrix.warehouse_scope == scope
    assert matrix.warehouse_ids == ["candidate-c", "center-a", "cross-b"]
    with pytest.raises(ValueError, match="warehouse_scope_candidate_ids_duplicate"):
        ExistingPlusCandidatesWarehouseScope(candidate_ids=["candidate-c", "candidate-c"])



def test_candidate_list_naming_every_candidate_is_canonical_all_warehouses() -> None:
    case = network_case()
    every_candidate = ExistingPlusCandidatesWarehouseScope(candidate_ids=["candidate-c"])
    assert canonical_warehouse_scope(case.warehouses, every_candidate) == AllWarehousesScope()
    for scope in (AllWarehousesScope(), ExistingOnlyWarehouseScope()):
        assert canonical_warehouse_scope(case.warehouses, scope) == scope
    with pytest.raises(ValueError, match="candidate_unknown"):
        canonical_warehouse_scope(
            case.warehouses,
            ExistingPlusCandidatesWarehouseScope(candidate_ids=["missing"]),
        )


def test_existing_plus_candidates_rejects_existing_or_unknown_ids() -> None:
    case = network_case()
    with pytest.raises(ValueError, match="must_be_non_existing"):
        build_haversine_route_matrix(
            case.demand,
            case.warehouses,
            1.2,
            40,
            warehouse_scope=ExistingPlusCandidatesWarehouseScope(candidate_ids=["center-a"]),
        )
    with pytest.raises(ValueError, match="candidate_unknown"):
        build_haversine_route_matrix(
            case.demand,
            case.warehouses,
            1.2,
            40,
            warehouse_scope=ExistingPlusCandidatesWarehouseScope(candidate_ids=["missing"]),
        )


def test_provided_route_matrix_materializes_exact_existing_scope() -> None:
    fixture = indonesia_network_fixture()
    existing = [warehouse for warehouse in fixture.warehouses if warehouse.is_existing]

    matrix = build_provided_route_matrix(
        fixture.demand,
        existing,
        indonesia_provided_route_facts(),
        warehouse_scope=ExistingOnlyWarehouseScope(),
    )

    assert matrix.method == "provided"
    assert len(matrix.rows) == 556
    assert matrix.missing_routes == []
    assert matrix.stats.model_dump(exclude={"kind"}) == {
        "expected_pair_count": 556,
        "provided_pair_count": 556,
        "ignored_input_pair_count": 24,
        "missing_pair_count": 0,
        "complete": True,
        "source_method_counts": {"haversine": 556},
    }
    assert {row.method for row in matrix.rows} == {"provided"}
    assert {row.tool_version for row in matrix.rows} == {"provided-input.v1"}


def test_navigation_registration_reports_missing_routes_without_filling_them() -> None:
    case = network_case()
    rows = [
        RouteMatrixRow(
            origin_id="center-a",
            destination_id="city-a",
            layer="last_mile",
            distance_km=1,
            duration_hours=0.1,
            method="navigation",
            tool_version="navigation-provider.v1",
            origin_longitude=106.8,
            origin_latitude=-6.2,
            destination_longitude=106.8,
            destination_latitude=-6.2,
            navigation_provider="test-provider",
            navigation_profile="truck",
        )
    ]
    matrix = register_navigation_route_matrix(
        case.demand,
        case.warehouses,
        rows,
        warehouse_scope=AllWarehousesScope(),
    )
    validation = validate_route_matrix(case.demand, case.warehouses, matrix)

    assert len(matrix.missing_routes) == 7
    assert validation.valid is False
    assert validation.missing_routes == matrix.missing_routes

    with pytest.raises(ValueError, match="navigation_route_duplicate_pair"):
        register_navigation_route_matrix(
            case.demand,
            case.warehouses,
            [rows[0], rows[0]],
            warehouse_scope=AllWarehousesScope(),
        )
    with pytest.raises(ValueError, match="navigation_route_unknown_pair"):
        register_navigation_route_matrix(
            case.demand,
            case.warehouses,
            [rows[0].model_copy(update={"destination_id": "not-required"})],
            warehouse_scope=AllWarehousesScope(),
        )

    haversine = build_haversine_route_matrix(case.demand, case.warehouses, 1.2, 40)
    mixed_provenance = [
        row.model_copy(
            update={
                "method": "navigation",
                "tool_version": f"navigation-provider.v{index}",
                "detour_coefficient": None,
                "average_speed_kph": None,
                "navigation_provider": "test-provider",
                "navigation_profile": "truck",
            }
        )
        for index, row in enumerate(haversine.rows[:2], start=1)
    ]
    with pytest.raises(ValueError, match="navigation_route_provenance_conflict"):
        register_navigation_route_matrix(
            case.demand,
            case.warehouses,
            mixed_provenance,
            warehouse_scope=AllWarehousesScope(),
        )


def test_route_reuse_is_exact_per_pair_and_ignores_unrelated_prior_rows() -> None:
    case = network_case()
    original = build_haversine_route_matrix(case.demand, case.warehouses, 1.2, 40)
    exact = original.rows[0]
    stale_version = original.rows[1].model_copy(update={"tool_version": "haversine.v0"})
    stale_coordinates = original.rows[2].model_copy(
        update={"destination_longitude": original.rows[2].destination_longitude + 0.01}
    )
    stale_distance = original.rows[3].model_copy(
        update={"distance_km": original.rows[3].distance_km + 1}
    )
    unrelated = original.rows[4].model_copy(
        update={"origin_id": "not-required", "destination_id": "also-not-required"}
    )

    rebuilt = build_route_matrix_with_reuse(
        case.demand,
        case.warehouses,
        [exact, stale_version, stale_coordinates, stale_distance, unrelated],
        detour_coefficient=1.2,
        average_speed_kph=40,
        warehouse_scope=AllWarehousesScope(),
    )

    assert rebuilt.stats.reused_pair_count == 1
    assert rebuilt.stats.stale_pair_count == 3
    assert rebuilt.stats.ignored_prior_row_count == 1
    assert rebuilt.stats.computed_pair_count == 7
    assert exact in rebuilt.rows
    refreshed = next(
        row
        for row in rebuilt.rows
        if (row.origin_id, row.destination_id, row.layer)
        == (stale_version.origin_id, stale_version.destination_id, stale_version.layer)
    )
    assert refreshed.tool_version == "haversine.v1"


def test_indonesia_route_reuse_only_computes_two_removed_round_sensitive_pairs() -> None:
    fixture = indonesia_network_fixture()
    original = build_haversine_route_matrix(fixture.demand, fixture.warehouses, 1.2, 42)
    removed = {
        next(
            (row.origin_id, row.destination_id, row.layer)
            for row in original.rows
            if row.layer == layer
        )
        for layer in ("last_mile", "linehaul")
    }

    rebuilt = build_route_matrix_with_reuse(
        fixture.demand,
        fixture.warehouses,
        [
            row
            for row in original.rows
            if (row.origin_id, row.destination_id, row.layer) not in removed
        ],
        1.2,
        42,
        warehouse_scope=AllWarehousesScope(),
    )

    assert len(original.rows) == 1168
    assert rebuilt.stats.reused_pair_count == 1166
    assert rebuilt.stats.computed_pair_count == 2
    assert rebuilt.stats.stale_pair_count == 0


def test_crossdock_requires_explicit_upstream_center() -> None:
    case = network_case()
    warehouses = [
        warehouse.model_copy(update={"upstream_center_id": None})
        if warehouse.warehouse_id == "cross-b"
        else warehouse
        for warehouse in case.warehouses
    ]

    with pytest.raises(ValueError, match="warehouse_upstream_center_required:cross-b"):
        build_haversine_route_matrix(case.demand, warehouses, 1.2, 40)


def test_missing_coordinates_are_reported_as_layered_pairs() -> None:
    case = network_case()
    demand = [
        city.model_copy(update={"longitude": None}) if city.city_id == "city-a" else city
        for city in case.demand
    ]

    matrix = build_haversine_route_matrix(demand, case.warehouses, 1.2, 40)

    assert matrix.stats.missing_pair_count == 3
    assert ("center-a", "city-a", "last_mile") in matrix.missing_routes
    assert ("cross-b", "city-a", "last_mile") in matrix.missing_routes
    assert ("candidate-c", "city-a", "last_mile") in matrix.missing_routes


def test_conflicting_prior_route_rows_are_rejected_per_pair() -> None:
    case = network_case()
    original = build_haversine_route_matrix(case.demand, case.warehouses, 1.2, 40)

    with pytest.raises(ValueError, match="route_fact_duplicate_pair"):
        build_route_matrix_with_reuse(
            case.demand,
            case.warehouses,
            [original.rows[0], original.rows[0]],
            1.2,
            40,
            warehouse_scope=AllWarehousesScope(),
        )


def test_haversine_builder_does_not_mislabel_navigation_prior_fact() -> None:
    case = network_case()
    original = build_haversine_route_matrix(case.demand, case.warehouses, 1.2, 40)
    navigation = original.rows[0].model_copy(
        update={
            "method": "navigation",
            "tool_version": "navigation-provider.v1",
            "detour_coefficient": None,
            "average_speed_kph": None,
            "navigation_provider": "test-provider",
            "navigation_profile": "truck",
        }
    )

    rebuilt = build_route_matrix_with_reuse(
        case.demand,
        case.warehouses,
        [navigation],
        1.2,
        40,
        warehouse_scope=AllWarehousesScope(),
    )

    assert rebuilt.method == "haversine"
    assert rebuilt.stats.reused_pair_count == 0
    assert rebuilt.stats.stale_pair_count == 1
    matching = next(
        row
        for row in rebuilt.rows
        if (row.origin_id, row.destination_id, row.layer)
        == (navigation.origin_id, navigation.destination_id, navigation.layer)
    )
    assert matching.method == "haversine"
