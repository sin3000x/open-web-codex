from __future__ import annotations

import json
from pathlib import Path

import pytest
from pydantic import ValidationError

from supply_chain_planner.data_core import (
    build_planning_dataset,
    inspect_source,
)
from supply_chain_planner.models import PlanningSource

ROOT = Path(__file__).resolve().parents[1]
SOURCE_PATH = (
    ROOT / "examples" / "data-sources" / "warehouse-network-fixture.json"
)


@pytest.fixture
def source() -> PlanningSource:
    return PlanningSource.model_validate(json.loads(SOURCE_PATH.read_text()))


def test_inspection_is_bounded_and_reports_source_scope(source: PlanningSource) -> None:
    inspection = inspect_source(source)

    assert inspection.source_summary.source_id == "warehouse-network-fixture"
    assert inspection.source_summary.order_row_count == 6
    assert inspection.source_summary.demand_units == 100
    assert inspection.source_summary.demand_node_count == 3
    assert inspection.source_summary.facility_count == 2
    assert inspection.source_summary.truncated is False


def test_builds_planning_dataset_and_network_handoff(source: PlanningSource) -> None:
    dataset = build_planning_dataset(source)

    assert dataset.schema_version == "planning-dataset.v1"
    assert dataset.source_summary.demand_units == 100
    assert [item.demand_units for item in dataset.demand_distribution] == [25, 40, 35]
    assert sum(item.promotion_units for item in dataset.demand_distribution) == 30
    assert dataset.delivery_baseline.observed_demand_units == 90
    assert dataset.delivery_baseline.on_time_demand_units == 55
    assert dataset.delivery_baseline.unobserved_demand_units == 10
    assert dataset.delivery_baseline.on_time_ratio == pytest.approx(55 / 90)
    assert sum(
        item.demand_units for item in dataset.network_input.demand_points
    ) == 100
    assert all(item.is_existing for item in dataset.network_input.facilities)
    assert dataset.data_quality.valid is True
    assert any("promotion-associated" in item for item in dataset.data_quality.warnings)


def test_dataset_identity_is_stable_for_same_source(source: PlanningSource) -> None:
    first = build_planning_dataset(source)
    second = build_planning_dataset(source)

    assert first.dataset_id == second.dataset_id
    assert first.source_digest == second.source_digest


def test_source_contract_rejects_direct_pii_fields() -> None:
    payload = json.loads(SOURCE_PATH.read_text())
    payload["orders"][0]["customer_name"] = "not allowed"

    with pytest.raises(ValidationError, match="Extra inputs are not permitted"):
        PlanningSource.model_validate(payload)


def test_source_contract_rejects_candidate_facilities() -> None:
    payload = json.loads(SOURCE_PATH.read_text())
    payload["facilities"][0]["is_existing"] = False

    with pytest.raises(ValidationError, match="only existing facilities"):
        PlanningSource.model_validate(payload)
