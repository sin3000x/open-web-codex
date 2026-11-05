from __future__ import annotations

from supply_chain_planner.data.mapping import SourceRole, TransformKind, TransformSpec
from supply_chain_planner.data.normalization import (
    ConfirmedFieldMapping,
    ConfirmedSourceRows,
    normalize_confirmed_rows,
)


def _mapping(target: str, kind: TransformKind) -> ConfirmedFieldMapping:
    return ConfirmedFieldMapping(
        target_field=target,
        source_field=target,
        transform=TransformSpec(kind=kind),
    )


def test_pure_normalization_accepts_verified_rows_without_case_or_io() -> None:
    demand = ConfirmedSourceRows(
        role=SourceRole.DEMAND,
        rows=[
            {
                "city_id": "CITY-1",
                "city_name": "City One",
                "demand_quantity": "10",
                "longitude": "100.1",
                "latitude": "1.2",
            }
        ],
        mappings=[
            _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("city_name", TransformKind.TRIM),
            _mapping("demand_quantity", TransformKind.PARSE_INTEGER),
            _mapping("longitude", TransformKind.PARSE_DECIMAL),
            _mapping("latitude", TransformKind.PARSE_DECIMAL),
        ],
    )
    warehouse = ConfirmedSourceRows(
        role=SourceRole.EXISTING_WAREHOUSE,
        rows=[
            {
                "warehouse_id": "WH-1",
                "warehouse_name": "Warehouse One",
                "warehouse_type": "center",
                "city_id": "CITY-1",
                "city_name": "City One",
                "longitude": "100.1",
                "latitude": "1.2",
            }
        ],
        mappings=[
            _mapping("warehouse_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("warehouse_name", TransformKind.TRIM),
            _mapping("warehouse_type", TransformKind.NORMALIZE_WAREHOUSE_TYPE),
            _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("city_name", TransformKind.TRIM),
            _mapping("longitude", TransformKind.PARSE_DECIMAL),
            _mapping("latitude", TransformKind.PARSE_DECIMAL),
        ],
    )

    state, batch = normalize_confirmed_rows([warehouse, demand])

    assert state == "ready"
    assert [item.city_id for item in batch.demand_cities] == ["CITY-1"]
    assert [item.warehouse_id for item in batch.warehouses] == ["WH-1"]
    assert batch.warehouses[0].is_existing is True
    assert batch.warehouses[0].is_fixed is None
    assert set(ConfirmedSourceRows.__dataclass_fields__) == {
        "role",
        "rows",
        "mappings",
        "relative_path",
        "unit_ref",
        "selection_key",
    }

    without_coordinates = [
        ConfirmedSourceRows(
            role=source.role,
            rows=[
                {
                    key: value
                    for key, value in source.rows[0].items()
                    if key not in {"longitude", "latitude"}
                }
            ],
            mappings=[
                mapping
                for mapping in source.mappings
                if mapping.target_field not in {"longitude", "latitude"}
            ],
        )
        for source in (warehouse, demand)
    ]
    geography_state, _ = normalize_confirmed_rows(without_coordinates)
    assert geography_state == "needs_geography"


def test_pure_normalization_returns_typed_missing_and_unknown_role_issues() -> None:
    state, batch = normalize_confirmed_rows(
        [
            ConfirmedSourceRows(role="unknown", rows=[{}], mappings=[]),
            ConfirmedSourceRows(
                role=SourceRole.ADMINISTRATIVE_CATALOG,
                rows=[{"city_id": "CITY-1"}],
                mappings=[],
            ),
        ]
    )

    assert state == "needs_input"
    assert {item.code for item in batch.issues} >= {
        "normalization_role_unknown",
        "normalization_role_unsupported",
        "demand_missing",
        "warehouse_missing",
    }


def test_route_quote_missing_business_fields_is_typed_issue() -> None:
    base_row = {
        "origin_id": "WH-1",
        "destination_id": "CITY-1",
        "price_per_vehicle": "100",
        "layer": "last_mile",
        "currency": "USD",
        "vehicle_capacity": "20",
    }
    kinds = {
        "origin_id": TransformKind.NORMALIZE_IDENTIFIER,
        "destination_id": TransformKind.NORMALIZE_IDENTIFIER,
        "price_per_vehicle": TransformKind.PARSE_DECIMAL,
        "layer": TransformKind.TRIM,
        "currency": TransformKind.TRIM,
        "vehicle_capacity": TransformKind.PARSE_DECIMAL,
    }
    for missing in ("layer", "currency", "vehicle_capacity"):
        row = {key: value for key, value in base_row.items() if key != missing}
        state, batch = normalize_confirmed_rows(
            [
                ConfirmedSourceRows(
                    role=SourceRole.ROUTE_QUOTE,
                    rows=[row],
                    mappings=[_mapping(field, kind) for field, kind in kinds.items()],
                )
            ]
        )

        assert state == "needs_input"
        assert f"route_quote_{missing}_missing" in {item.code for item in batch.issues}


def test_route_quote_preserves_complete_provided_route_fact() -> None:
    row = {
        "origin_id": "WH-1",
        "destination_id": "CITY-1",
        "destination_name": "City One",
        "layer": "last_mile",
        "distance_km": "12.5",
        "duration_hours": "0.5",
        "price_per_vehicle": "100",
        "currency": "IDR",
        "vehicle_capacity": "1",
        "method": "haversine",
    }
    decimal_fields = {
        "distance_km",
        "duration_hours",
        "price_per_vehicle",
        "vehicle_capacity",
    }
    id_fields = {"origin_id", "destination_id"}
    state, batch = normalize_confirmed_rows(
        [
            ConfirmedSourceRows(
                role=SourceRole.ROUTE_QUOTE,
                rows=[row],
                mappings=[
                    _mapping(
                        field,
                        TransformKind.PARSE_DECIMAL
                        if field in decimal_fields
                        else TransformKind.NORMALIZE_IDENTIFIER
                        if field in id_fields
                        else TransformKind.TRIM,
                    )
                    for field in row
                ],
            )
        ]
    )

    assert state == "needs_input"  # demand and warehouse inputs are intentionally absent
    assert len(batch.route_quotes) == 1
    assert len(batch.provided_route_facts) == 1
    fact = batch.provided_route_facts[0]
    assert fact.distance_km == 12.5
    assert fact.duration_hours == 0.5
    assert fact.source_method == "haversine"


def test_incomplete_provided_route_fact_rejects_the_entire_route_row() -> None:
    state, batch = normalize_confirmed_rows(
        [
            ConfirmedSourceRows(
                role=SourceRole.ROUTE_QUOTE,
                rows=[
                    {
                        "origin_id": "WH-1",
                        "destination_id": "CITY-1",
                        "layer": "last_mile",
                        "distance_km": "12.5",
                        "price_per_vehicle": "100",
                        "currency": "IDR",
                        "vehicle_capacity": "1",
                    }
                ],
                mappings=[
                    _mapping(
                        field,
                        TransformKind.PARSE_DECIMAL
                        if field in {"distance_km", "price_per_vehicle", "vehicle_capacity"}
                        else TransformKind.NORMALIZE_IDENTIFIER
                        if field in {"origin_id", "destination_id"}
                        else TransformKind.TRIM,
                    )
                    for field in (
                        "origin_id",
                        "destination_id",
                        "layer",
                        "distance_km",
                        "price_per_vehicle",
                        "currency",
                        "vehicle_capacity",
                    )
                ],
            )
        ]
    )

    assert state == "needs_input"
    assert batch.route_quotes == []
    assert batch.provided_route_facts == []
    assert "provided_route_fact_incomplete" in {issue.code for issue in batch.issues}


def test_same_role_duplicate_records_are_bounded_and_deterministic() -> None:
    mapping = [
        _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
        _mapping("city_name", TransformKind.TRIM),
        _mapping("demand_quantity", TransformKind.PARSE_INTEGER),
        _mapping("longitude", TransformKind.PARSE_DECIMAL),
        _mapping("latitude", TransformKind.PARSE_DECIMAL),
    ]
    def source(key: str) -> ConfirmedSourceRows:
        return ConfirmedSourceRows(
            role=SourceRole.DEMAND,
            rows=[
                {
                    "city_id": "C-1",
                    "city_name": "Jakarta",
                    "demand_quantity": "10",
                    "longitude": "106.8",
                    "latitude": "-6.2",
                }
            ],
            mappings=mapping,
            relative_path=f"{key}.csv",
            unit_ref="table",
            selection_key=key,
        )
    warehouse = ConfirmedSourceRows(
        role=SourceRole.EXISTING_WAREHOUSE,
        rows=[
            {
                "warehouse_id": "W-1",
                "warehouse_name": "Jakarta",
                "warehouse_type": "center",
                "city_id": "C-1",
                "city_name": "Jakarta",
                "longitude": "106.8",
                "latitude": "-6.2",
            }
        ],
        mappings=[
            _mapping("warehouse_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("warehouse_name", TransformKind.TRIM),
            _mapping("warehouse_type", TransformKind.NORMALIZE_WAREHOUSE_TYPE),
            _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("city_name", TransformKind.TRIM),
            _mapping("longitude", TransformKind.PARSE_DECIMAL),
            _mapping("latitude", TransformKind.PARSE_DECIMAL),
        ],
        relative_path="warehouse.csv",
        unit_ref="table",
        selection_key="warehouse",
    )
    state, batch = normalize_confirmed_rows([source("first"), source("second"), warehouse])
    assert state == "ready"
    duplicate_issues = [issue for issue in batch.issues if issue.code == "demand_city_duplicate_identical"]
    assert len(duplicate_issues) == 1
    assert "重复记录数：1" in duplicate_issues[0].business_message

    conflict = source("conflict")
    conflict.rows[0]["city_name"] = "Surabaya"
    state, batch = normalize_confirmed_rows([source("first"), conflict, warehouse])
    assert state == "needs_input"
    assert len([issue for issue in batch.issues if issue.code == "demand_city_duplicate_conflict"]) == 1


def test_network_validator_aggregates_repeated_unknown_assignment_errors() -> None:
    demand = ConfirmedSourceRows(
        role=SourceRole.DEMAND,
        rows=[{"city_id": "C-1", "city_name": "Jakarta", "demand_quantity": "10"}],
        mappings=[
            _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("city_name", TransformKind.TRIM),
            _mapping("demand_quantity", TransformKind.PARSE_INTEGER),
        ],
    )
    warehouse = ConfirmedSourceRows(
        role=SourceRole.EXISTING_WAREHOUSE,
        rows=[
            {
                "warehouse_id": "W-1",
                "warehouse_name": "Jakarta",
                "warehouse_type": "center",
                "city_id": "C-1",
                "city_name": "Jakarta",
            }
        ],
        mappings=[
            _mapping("warehouse_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("warehouse_name", TransformKind.TRIM),
            _mapping("warehouse_type", TransformKind.NORMALIZE_WAREHOUSE_TYPE),
            _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("city_name", TransformKind.TRIM),
        ],
    )
    assignments = ConfirmedSourceRows(
        role=SourceRole.CURRENT_ASSIGNMENT,
        rows=[
            {"demand_city_id": f"UNKNOWN-{index}", "serving_warehouse_id": "W-1"}
            for index in range(100)
        ],
        mappings=[
            _mapping("demand_city_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("serving_warehouse_id", TransformKind.NORMALIZE_IDENTIFIER),
        ],
    )
    state, batch = normalize_confirmed_rows([demand, warehouse, assignments])
    assert state == "needs_input"
    errors = [issue for issue in batch.issues if issue.code == "assignment_demand_unknown"]
    assert len(errors) == 1
    assert "问题记录数：100" in errors[0].business_message


def test_cross_dock_without_upstream_center_needs_input_at_preparation() -> None:
    demand = ConfirmedSourceRows(
        role=SourceRole.DEMAND,
        rows=[{"city_id": "C-1", "city_name": "Jakarta", "demand_quantity": "10"}],
        mappings=[
            _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
            _mapping("city_name", TransformKind.TRIM),
            _mapping("demand_quantity", TransformKind.PARSE_INTEGER),
        ],
    )
    warehouse_mappings = [
        _mapping("warehouse_id", TransformKind.NORMALIZE_IDENTIFIER),
        _mapping("warehouse_name", TransformKind.TRIM),
        _mapping("warehouse_type", TransformKind.NORMALIZE_WAREHOUSE_TYPE),
        _mapping("city_id", TransformKind.NORMALIZE_IDENTIFIER),
        _mapping("city_name", TransformKind.TRIM),
    ]
    existing = ConfirmedSourceRows(
        role=SourceRole.EXISTING_WAREHOUSE,
        rows=[
            {
                "warehouse_id": "W-1",
                "warehouse_name": "Jakarta",
                "warehouse_type": "center",
                "city_id": "C-1",
                "city_name": "Jakarta",
            }
        ],
        mappings=warehouse_mappings,
    )
    candidates = ConfirmedSourceRows(
        role=SourceRole.CANDIDATE_WAREHOUSE,
        rows=[
            {
                "warehouse_id": "W-CANDIDATE",
                "warehouse_name": "Balikpapan",
                "warehouse_type": "cross_docking",
                "city_id": "C-1",
                "city_name": "Jakarta",
            }
        ],
        mappings=warehouse_mappings,
    )

    state, batch = normalize_confirmed_rows([demand, existing, candidates])

    assert state == "needs_input"
    errors = [
        issue for issue in batch.issues if issue.code == "warehouse_upstream_center_required"
    ]
    assert len(errors) == 1
    assert errors[0].severity == "error"
    assert "上游中心仓" in errors[0].business_message
