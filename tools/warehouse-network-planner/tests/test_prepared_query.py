from __future__ import annotations

import asyncio
import csv
from decimal import Decimal
from pathlib import Path
from types import SimpleNamespace

import pytest
from pydantic import ValidationError
from supply_chain_planner.data import server as data_server
from supply_chain_planner.data.mapping import SourceRole
from supply_chain_planner.shared.models import (
    PreparedQueryFilter,
    PreparedQueryMeasure,
    PreparedQueryOrder,
    SourceInspectionIdentity,
    SourceSelection,
)
from supply_chain_planner.shared.resources import SupplyChainResources

PREPARED_PATH = "outputs/warehouse-network/prepared/query.json"
FIXTURE_ROOT = Path(__file__).parents[3] / "apps/web/scripts/fixtures/warehouse-network/mock_data"
SOURCE_NAMES = (
    "administrative-areas.json",
    "candidate-warehouses.csv",
    "demand-cities.csv",
    "existing-warehouses.csv",
    "route-quotes.csv",
)


def _context(workspace: Path) -> SimpleNamespace:
    meta = SimpleNamespace(
        model_extra={"codex/sandbox-state-meta": {"sandboxCwd": workspace.as_uri()}}
    )
    return SimpleNamespace(request_context=SimpleNamespace(meta=meta))


def _fixture_rows(name: str) -> list[dict[str, str]]:
    with (FIXTURE_ROOT / name).open(encoding="utf-8", newline="") as stream:
        return list(csv.DictReader(stream))


@pytest.fixture
def prepared_workspace(tmp_path, monkeypatch) -> tuple[Path, SimpleNamespace]:
    for name in SOURCE_NAMES:
        (tmp_path / name).write_bytes((FIXTURE_ROOT / name).read_bytes())
    resources = SupplyChainResources(tmp_path, tmp_path / "profile")
    monkeypatch.setattr(data_server, "_supply_chain_resources", resources)
    monkeypatch.setattr(data_server, "_workspace", lambda _ctx: tmp_path)
    ctx = _context(tmp_path)
    roles = [
        SourceRole.DEMAND,
        SourceRole.EXISTING_WAREHOUSE,
        SourceRole.CANDIDATE_WAREHOUSE,
        SourceRole.ROUTE_QUOTE,
    ]
    profile = data_server.inspect_workspace_sources(
        sorted(SOURCE_NAMES), roles, ctx=ctx, country_code="ID"
    ).structuredContent
    prepared = data_server.prepare_network_input(
        SourceInspectionIdentity.model_validate(profile["inspection_identity"]),
        profile["inspected_relative_paths"],
        [
            SourceSelection(
                relative_path="demand-cities.csv", unit_ref="table", role=SourceRole.DEMAND
            ),
            SourceSelection(
                relative_path="existing-warehouses.csv",
                unit_ref="table",
                role=SourceRole.EXISTING_WAREHOUSE,
            ),
            SourceSelection(
                relative_path="candidate-warehouses.csv",
                unit_ref="table",
                role=SourceRole.CANDIDATE_WAREHOUSE,
            ),
            SourceSelection(
                relative_path="route-quotes.csv", unit_ref="table", role=SourceRole.ROUTE_QUOTE
            ),
        ],
        "ID",
        PREPARED_PATH,
        ctx,
        administrative_catalog_relative_path="administrative-areas.json",
    ).model_dump(mode="json", by_alias=True)
    assert prepared["outcome"] == "ready"
    return tmp_path, ctx


def _query(ctx: SimpleNamespace, dataset: str, **kwargs) -> dict:
    result = data_server.query_prepared_network_input(PREPARED_PATH, dataset, ctx, **kwargs)
    payload = result.structuredContent
    assert result.content[0].text == payload["summary"]
    return payload


def test_query_tool_is_read_only_and_lists_every_dataset_field() -> None:
    tools = {tool.name: tool for tool in asyncio.run(data_server.mcp.list_tools())}
    query = tools["query_prepared_network_input"]

    assert query.annotations.readOnlyHint is True
    assert query.annotations.destructiveHint is False
    assert query.inputSchema["required"] == ["prepared_input_relative_path", "dataset"]
    assert query.outputSchema["discriminator"]["propertyName"] == "outcome"
    assert "warehouses: warehouse_id(text)" in query.description
    assert "warehouse_type(text)" in query.description
    assert "demand_quantity(number)" in query.description
    assert "is_existing(bool)" in query.description


def test_group_by_counts_every_record_not_only_the_preview(prepared_workspace) -> None:
    _root, ctx = prepared_workspace
    expected: dict[str, int] = {}
    for row in _fixture_rows("existing-warehouses.csv"):
        expected[row["warehouse_type"]] = expected.get(row["warehouse_type"], 0) + 1

    payload = _query(
        ctx,
        "warehouses",
        filters=[PreparedQueryFilter(field_name="is_existing", op="eq", value=True)],
        group_by=["warehouse_type"],
    )

    assert payload["outcome"] == "answered"
    assert payload["schemaVersion"] == "prepared_network_query.v1"
    assert payload["roles"] == ["candidate_warehouse", "demand", "existing_warehouse", "route_quote"]
    assert payload["role_counts"]["candidate_warehouse"] == 12
    assert payload["record_count"] == 23
    assert payload["matched_count"] == 11
    assert (
        {group["key"]["warehouse_type"]: group["count"] for group in payload["groups"]}
        == expected
        == {"center": 5, "cross_docking": 6}
    )
    assert payload["rows"] == []
    assert payload["rows_truncated"] is False
    assert "warehouse_type=cross_docking: 6" in payload["summary"]
    assert "warehouse_type=center: 5" in payload["summary"]


def test_measures_and_ordered_rows_use_exact_complete_values(prepared_workspace) -> None:
    _root, ctx = prepared_workspace
    demand = _fixture_rows("demand-cities.csv")
    total = sum(Decimal(row["demand_quantity"]) for row in demand)
    top = sorted(demand, key=lambda row: Decimal(row["demand_quantity"]), reverse=True)[:3]

    payload = _query(
        ctx,
        "demand_cities",
        measures=[
            PreparedQueryMeasure(op="sum", field_name="demand_quantity"),
            PreparedQueryMeasure(op="max", field_name="demand_quantity"),
        ],
        select=["city_name", "demand_quantity"],
        order_by=PreparedQueryOrder(field_name="demand_quantity", descending=True),
        limit=3,
    )

    assert payload["matched_count"] == len(demand) == 50
    assert Decimal(payload["measures"][0]["value"]) == total
    assert payload["measures"][0]["value_count"] == 50
    assert Decimal(payload["measures"][1]["value"]) == Decimal(top[0]["demand_quantity"])
    assert [row["city_name"] for row in payload["rows"]] == [row["city_name"] for row in top]
    assert set(payload["rows"][0]) == {"city_name", "demand_quantity"}
    assert payload["rows_truncated"] is True


def test_in_and_range_filters_coerce_numbers_exactly(prepared_workspace) -> None:
    _root, ctx = prepared_workspace
    demand = _fixture_rows("demand-cities.csv")
    threshold = Decimal("5000")
    expected = sum(1 for row in demand if Decimal(row["demand_quantity"]) >= threshold)

    ranged = _query(
        ctx,
        "demand_cities",
        filters=[PreparedQueryFilter(field_name="demand_quantity", op="gte", value="5000")],
        limit=0,
    )
    listed = _query(
        ctx,
        "warehouses",
        filters=[
            PreparedQueryFilter(
                field_name="warehouse_id",
                op="in",
                values=["WH-CENTER-JAKARTA", "WH-CENTER-MEDAN", "WH-UNKNOWN"],
            )
        ],
        select=["warehouse_id"],
        order_by=PreparedQueryOrder(field_name="warehouse_id"),
    )

    assert ranged["matched_count"] == expected
    assert ranged["rows"] == []
    assert ranged["rows_truncated"] is (expected > 0)
    assert [row["warehouse_id"] for row in listed["rows"]] == [
        "WH-CENTER-JAKARTA",
        "WH-CENTER-MEDAN",
    ]


@pytest.mark.parametrize(
    ("kwargs", "message"),
    [
        (
            {"group_by": ["warehouse_kind"]},
            "query_field_invalid: warehouse_kind is not a warehouses field; allowed: warehouse_id",
        ),
        (
            {"measures": [PreparedQueryMeasure(op="sum", field_name="warehouse_name")]},
            "query_measure_requires_number_field: warehouse_name",
        ),
        (
            {"filters": [PreparedQueryFilter(field_name="city_name", op="gt", value=1)]},
            "query_filter_range_requires_number_field: city_name",
        ),
        (
            {"filters": [PreparedQueryFilter(field_name="is_existing", op="eq", value="yes")]},
            "query_filter_value_invalid: is_existing expects true or false",
        ),
        (
            {
                "group_by": ["warehouse_type"],
                "order_by": PreparedQueryOrder(field_name="warehouse_id"),
            },
            "query_order_by_requires_ungrouped_rows",
        ),
    ],
)
def test_invalid_queries_fail_with_typed_reasons(prepared_workspace, kwargs, message) -> None:
    _root, ctx = prepared_workspace

    with pytest.raises(ValueError, match=message):
        _query(ctx, "warehouses", **kwargs)


def test_filter_operands_are_validated_by_the_contract() -> None:
    with pytest.raises(ValidationError, match="query_filter_in_requires_values"):
        PreparedQueryFilter(field_name="warehouse_id", op="in", value="WH-1")
    with pytest.raises(ValidationError, match="query_filter_values_only_for_in"):
        PreparedQueryFilter(field_name="warehouse_id", op="eq", values=["WH-1"])
    with pytest.raises(ValidationError, match="query_filter_range_requires_value"):
        PreparedQueryFilter(field_name="demand_quantity", op="gt", value=True)


def test_changed_selected_source_returns_stale_instead_of_facts(prepared_workspace) -> None:
    root, ctx = prepared_workspace
    source = root / "existing-warehouses.csv"
    source.write_text(
        source.read_text(encoding="utf-8").replace("center,", "cross_docking,", 1),
        encoding="utf-8",
    )

    payload = _query(ctx, "warehouses", group_by=["warehouse_type"])

    assert payload == {
        "outcome": "prepared_input_stale",
        "summary": payload["summary"],
        "reason": "source_changed",
        "prepared_input_relative_path": PREPARED_PATH,
        "next_action": "reinspect",
        "retryable": False,
    }


def test_query_rejects_paths_outside_the_prepared_directory(prepared_workspace) -> None:
    _root, ctx = prepared_workspace

    with pytest.raises(ValueError, match="prepared_input_path_invalid"):
        data_server.query_prepared_network_input("existing-warehouses.csv", "warehouses", ctx)
