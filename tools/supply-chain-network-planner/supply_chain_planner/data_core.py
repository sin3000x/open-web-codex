"""Read-only source aggregation for the supply-chain Data Agent."""

from __future__ import annotations

import hashlib
import json
from collections import defaultdict

from .models import (
    DeliveryBaseline,
    DemandDistributionRow,
    DemandPoint,
    NetworkInput,
    PlanningDataQuality,
    PlanningDataset,
    PlanningSource,
    PlanningSourceInspection,
    PlanningSourceSummary,
)


def _canonical_source(source: PlanningSource) -> bytes:
    return json.dumps(
        source.model_dump(mode="json"),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def source_digest(source: PlanningSource) -> str:
    return hashlib.sha256(_canonical_source(source)).hexdigest()


def inspect_source(source: PlanningSource) -> PlanningSourceInspection:
    summary, _, _, quality = _analyze(source)
    return PlanningSourceInspection(source_summary=summary, data_quality=quality)


def build_planning_dataset(source: PlanningSource) -> PlanningDataset:
    summary, distribution, baseline, quality = _analyze(source)
    units_by_demand = {
        item.demand_id: item.demand_units for item in distribution
    }
    demand_points = [
        DemandPoint(
            demand_id=location.demand_id,
            location=location.location,
            demand_units=units_by_demand.get(location.demand_id, 0),
            region=location.region,
            current_facility_id=location.current_facility_id,
        )
        for location in sorted(source.demand_locations, key=lambda item: item.demand_id)
    ]
    network_input = NetworkInput(
        planning_period=source.planning_period,
        currency=source.currency,
        service_policy=source.service_policy,
        demand_points=demand_points,
        facilities=sorted(source.facilities, key=lambda item: item.facility_id),
        transport_rates=sorted(source.transport_rates, key=lambda item: item.rate_id),
    )
    digest = source_digest(source)
    return PlanningDataset(
        dataset_id=f"planning_dataset_{digest[:24]}",
        source_digest=digest,
        source_summary=summary,
        network_input=network_input,
        demand_distribution=distribution,
        delivery_baseline=baseline,
        data_quality=quality,
        assumptions=[
            "Demand is aggregated from source order rows by demand_id.",
            "Promotion share is demand-unit weighted.",
            "Delivery baseline uses the source service-policy threshold.",
            "Only de-identified fields declared by planning_source.v1 are accepted.",
        ],
    )


def _analyze(
    source: PlanningSource,
) -> tuple[
    PlanningSourceSummary,
    list[DemandDistributionRow],
    DeliveryBaseline,
    PlanningDataQuality,
]:
    demand_units: dict[str, int] = defaultdict(int)
    order_rows: dict[str, int] = defaultdict(int)
    promotion_units: dict[str, int] = defaultdict(int)
    observed = 0
    on_time = 0
    unobserved = 0
    for order in source.orders:
        demand_units[order.demand_id] += order.demand_units
        order_rows[order.demand_id] += 1
        if order.promotion:
            promotion_units[order.demand_id] += order.demand_units
        if order.actual_delivery_seconds is None:
            unobserved += order.demand_units
        else:
            observed += order.demand_units
            if order.actual_delivery_seconds <= source.service_policy.max_delivery_seconds:
                on_time += order.demand_units

    locations = {
        location.demand_id: location for location in source.demand_locations
    }
    distribution = [
        DemandDistributionRow(
            demand_id=demand_id,
            region=locations[demand_id].region,
            demand_units=units,
            order_row_count=order_rows[demand_id],
            promotion_units=promotion_units[demand_id],
            promotion_share=promotion_units[demand_id] / units if units else 0,
        )
        for demand_id, units in sorted(demand_units.items())
    ]
    dates = [order.order_date for order in source.orders]
    total_units = sum(demand_units.values())
    summary = PlanningSourceSummary(
        source_id=source.source_id,
        source_updated_at=source.source_updated_at,
        order_row_count=len(source.orders),
        demand_node_count=len(source.demand_locations),
        facility_count=len(source.facilities),
        date_from=min(dates),
        date_to=max(dates),
        demand_units=total_units,
    )
    baseline = DeliveryBaseline(
        observed_demand_units=observed,
        on_time_demand_units=on_time,
        unobserved_demand_units=unobserved,
        on_time_ratio=on_time / observed if observed else None,
        threshold_seconds=source.service_policy.max_delivery_seconds,
    )
    errors: list[str] = []
    warnings: list[str] = []
    missing_assignments = sorted(
        location.demand_id
        for location in source.demand_locations
        if location.current_facility_id is None
    )
    if missing_assignments:
        errors.append(
            "missing current facility assignments for demand nodes: "
            + ", ".join(missing_assignments)
        )
    zero_demand = sorted(set(locations) - set(demand_units))
    if zero_demand:
        warnings.append(
            "demand nodes have no source order rows: " + ", ".join(zero_demand)
        )
    if unobserved:
        warnings.append(
            f"{unobserved} demand units have no observed delivery duration"
        )
    promotional = sum(promotion_units.values())
    if promotional:
        warnings.append(
            f"{promotional}/{total_units} demand units "
            f"({promotional / total_units:.2%}) are promotion-associated"
        )
    quality = PlanningDataQuality(
        valid=not errors,
        errors=errors,
        warnings=warnings,
    )
    return summary, distribution, baseline, quality
