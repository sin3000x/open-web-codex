"""Typed public contracts for supply-chain network planning."""

from __future__ import annotations

from datetime import date, datetime, timezone
from decimal import Decimal
from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

SCHEMA_VERSION = "1.0"
MCP_SERVER_NAME = "supply_chain_planner"

NonNegativeMoney = Annotated[Decimal, Field(ge=0, decimal_places=6)]


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid")


class DataRef(StrictModel):
    type: Literal["mcp_resource"] = "mcp_resource"
    server: Literal["supply_chain_planner"] = MCP_SERVER_NAME
    uri: str = Field(pattern=r"^supply-chain://resources/[a-z0-9_.-]{1,160}$")
    format: Literal["json"] = "json"
    resource_schema: str


class DataAgentRef(StrictModel):
    type: Literal["mcp_resource"] = "mcp_resource"
    server: Literal["supply_chain_data"] = "supply_chain_data"
    uri: str = Field(pattern=r"^supply-chain-data://resources/[a-z0-9_.-]{1,160}$")
    format: Literal["json"] = "json"
    resource_schema: str


class Point(StrictModel):
    latitude: float = Field(ge=-90, le=90)
    longitude: float = Field(ge=-180, le=180)


class ServicePolicy(StrictModel):
    policy_id: str = Field(min_length=1, max_length=128)
    max_delivery_seconds: int = Field(gt=0)
    order_cutoff_wait_seconds: int = Field(default=0, ge=0)
    last_mile_buffer_seconds: int = Field(default=0, ge=0)
    label: str | None = Field(default=None, max_length=256)


class DemandPoint(StrictModel):
    demand_id: str = Field(min_length=1, max_length=128)
    location: Point
    demand_units: int = Field(ge=0)
    region: str | None = Field(default=None, max_length=128)
    current_facility_id: str | None = Field(default=None, max_length=128)


class Facility(StrictModel):
    facility_id: str = Field(min_length=1, max_length=128)
    location: Point
    capacity_units: int = Field(ge=0)
    is_existing: bool
    handling_seconds: int = Field(default=0, ge=0)
    fixed_cost: NonNegativeMoney = Decimal("0")
    handling_cost_per_unit: NonNegativeMoney = Decimal("0")
    label: str | None = Field(default=None, max_length=256)


class TransportRate(StrictModel):
    rate_id: str = Field(min_length=1, max_length=128)
    origin_facility_id: str
    destination_demand_id: str | None = None
    destination_region: str | None = None
    base_cost_per_unit: NonNegativeMoney = Decimal("0")
    distance_cost_per_km_per_unit: NonNegativeMoney = Decimal("0")

    @model_validator(mode="after")
    def one_destination_selector(self) -> TransportRate:
        if self.destination_demand_id is not None and self.destination_region is not None:
            raise ValueError(
                "transport rate may select a demand point or a region, but not both"
            )
        return self


class NetworkInput(StrictModel):
    schema_version: Literal["network_input.v1"] = "network_input.v1"
    planning_period: str = Field(min_length=1, max_length=128)
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    service_policy: ServicePolicy
    demand_points: list[DemandPoint] = Field(min_length=1)
    facilities: list[Facility] = Field(min_length=1)
    transport_rates: list[TransportRate] = Field(default_factory=list)

    @model_validator(mode="after")
    def validate_references(self) -> NetworkInput:
        demand_ids = [item.demand_id for item in self.demand_points]
        facility_ids = [item.facility_id for item in self.facilities]
        rate_ids = [item.rate_id for item in self.transport_rates]
        for label, identifiers in (
            ("demand", demand_ids),
            ("facility", facility_ids),
            ("transport rate", rate_ids),
        ):
            if len(identifiers) != len(set(identifiers)):
                raise ValueError(f"duplicate {label} identifier")

        demand_id_set = set(demand_ids)
        facility_id_set = set(facility_ids)
        regions = {item.region for item in self.demand_points if item.region is not None}
        for demand in self.demand_points:
            if (
                demand.current_facility_id is not None
                and demand.current_facility_id not in facility_id_set
            ):
                raise ValueError(
                    f"demand {demand.demand_id!r} references unknown current facility "
                    f"{demand.current_facility_id!r}"
                )
        selectors: set[tuple[str, str, str]] = set()
        for rate in self.transport_rates:
            if rate.origin_facility_id not in facility_id_set:
                raise ValueError(
                    f"rate {rate.rate_id!r} references unknown origin facility "
                    f"{rate.origin_facility_id!r}"
                )
            if (
                rate.destination_demand_id is not None
                and rate.destination_demand_id not in demand_id_set
            ):
                raise ValueError(
                    f"rate {rate.rate_id!r} references unknown demand "
                    f"{rate.destination_demand_id!r}"
                )
            if rate.destination_region is not None and rate.destination_region not in regions:
                raise ValueError(
                    f"rate {rate.rate_id!r} references unused region "
                    f"{rate.destination_region!r}"
                )
            if rate.destination_demand_id is not None:
                selector = ("demand", rate.origin_facility_id, rate.destination_demand_id)
            elif rate.destination_region is not None:
                selector = ("region", rate.origin_facility_id, rate.destination_region)
            else:
                selector = ("default", rate.origin_facility_id, "")
            if selector in selectors:
                raise ValueError(
                    "ambiguous transport rates: more than one rate has selector "
                    f"{selector!r}"
                )
            selectors.add(selector)
        for demand in self.demand_points:
            for facility in self.facilities:
                candidates = [
                    rate
                    for rate in self.transport_rates
                    if rate.origin_facility_id == facility.facility_id
                    and (
                        rate.destination_demand_id == demand.demand_id
                        or (
                            rate.destination_demand_id is None
                            and rate.destination_region == demand.region
                        )
                        or (
                            rate.destination_demand_id is None
                            and rate.destination_region is None
                        )
                    )
                ]
                if not candidates:
                    raise ValueError(
                        "missing transport rate for facility "
                        f"{facility.facility_id!r} and demand {demand.demand_id!r}"
                    )
        return self


class DemandLocation(StrictModel):
    demand_id: str = Field(min_length=1, max_length=128)
    location: Point
    region: str | None = Field(default=None, max_length=128)
    current_facility_id: str | None = Field(default=None, max_length=128)


class OrderFact(StrictModel):
    demand_id: str = Field(min_length=1, max_length=128)
    order_date: date
    demand_units: int = Field(gt=0)
    promotion: bool = False
    actual_delivery_seconds: int | None = Field(default=None, ge=0)


class PlanningSource(StrictModel):
    schema_version: Literal["planning_source.v1"] = "planning_source.v1"
    source_id: str = Field(pattern=r"^[a-z0-9][a-z0-9_-]{0,127}$")
    source_updated_at: datetime
    planning_period: str = Field(min_length=1, max_length=128)
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    service_policy: ServicePolicy
    demand_locations: list[DemandLocation] = Field(min_length=1)
    facilities: list[Facility] = Field(min_length=1)
    transport_rates: list[TransportRate] = Field(min_length=1)
    orders: list[OrderFact] = Field(min_length=1)

    @model_validator(mode="after")
    def validate_source_references(self) -> PlanningSource:
        demand_ids = [item.demand_id for item in self.demand_locations]
        if len(demand_ids) != len(set(demand_ids)):
            raise ValueError("duplicate demand location identifier")
        facility_ids = {item.facility_id for item in self.facilities}
        if any(not item.is_existing for item in self.facilities):
            raise ValueError("planning source may contain only existing facilities")
        for location in self.demand_locations:
            if (
                location.current_facility_id is not None
                and location.current_facility_id not in facility_ids
            ):
                raise ValueError(
                    f"demand {location.demand_id!r} references unknown current facility "
                    f"{location.current_facility_id!r}"
                )
        demand_id_set = set(demand_ids)
        for order in self.orders:
            if order.demand_id not in demand_id_set:
                raise ValueError(
                    f"order references unknown demand location {order.demand_id!r}"
                )
        projected_demands = [
            DemandPoint(
                demand_id=location.demand_id,
                location=location.location,
                demand_units=0,
                region=location.region,
                current_facility_id=location.current_facility_id,
            )
            for location in self.demand_locations
        ]
        NetworkInput(
            planning_period=self.planning_period,
            currency=self.currency,
            service_policy=self.service_policy,
            demand_points=projected_demands,
            facilities=self.facilities,
            transport_rates=self.transport_rates,
        )
        return self


class PlanningSourceSummary(StrictModel):
    source_id: str
    source_updated_at: datetime
    order_row_count: int = Field(ge=0)
    demand_node_count: int = Field(ge=0)
    facility_count: int = Field(ge=0)
    date_from: date
    date_to: date
    demand_units: int = Field(ge=0)
    truncated: Literal[False] = False


class DemandDistributionRow(StrictModel):
    demand_id: str
    region: str | None
    demand_units: int = Field(ge=0)
    order_row_count: int = Field(ge=0)
    promotion_units: int = Field(ge=0)
    promotion_share: float = Field(ge=0, le=1)


class DeliveryBaseline(StrictModel):
    observed_demand_units: int = Field(ge=0)
    on_time_demand_units: int = Field(ge=0)
    unobserved_demand_units: int = Field(ge=0)
    on_time_ratio: float | None = Field(default=None, ge=0, le=1)
    threshold_seconds: int = Field(gt=0)


class PlanningDataQuality(StrictModel):
    valid: bool
    errors: list[str] = Field(default_factory=list)
    warnings: list[str] = Field(default_factory=list)


class PlanningDataset(StrictModel):
    schema_version: Literal["planning-dataset.v1"] = "planning-dataset.v1"
    dataset_id: str
    created_at: datetime = Field(default_factory=utc_now)
    source_digest: str = Field(pattern=r"^[a-f0-9]{64}$")
    source_summary: PlanningSourceSummary
    network_input: NetworkInput
    demand_distribution: list[DemandDistributionRow]
    delivery_baseline: DeliveryBaseline
    data_quality: PlanningDataQuality
    assumptions: list[str] = Field(default_factory=list)


class PlanningSourceInspection(StrictModel):
    schema_version: Literal["planning_source_inspection.v1"] = (
        "planning_source_inspection.v1"
    )
    source_summary: PlanningSourceSummary
    data_quality: PlanningDataQuality


class DataAgentResourceToolResult(StrictModel):
    summary: str
    data_ref: DataAgentRef


class NetworkSnapshot(NetworkInput):
    schema_version: Literal["network_snapshot.v1"] = "network_snapshot.v1"
    snapshot_id: str
    created_at: datetime = Field(default_factory=utc_now)
    source_name: str | None = None


class RouteEntry(StrictModel):
    origin_facility_id: str
    destination_demand_id: str
    distance_meters: int | None = Field(default=None, ge=0)
    travel_seconds: int | None = Field(default=None, ge=0)
    status: Literal["ready", "unreachable", "error"] = "ready"
    error_code: str | None = Field(default=None, max_length=128)

    @model_validator(mode="after")
    def ready_route_has_metrics(self) -> RouteEntry:
        if self.status == "ready" and (
            self.distance_meters is None or self.travel_seconds is None
        ):
            raise ValueError("ready route entries require distance_meters and travel_seconds")
        return self


class RouteMatrix(StrictModel):
    schema_version: Literal["route_matrix.v1"] = "route_matrix.v1"
    route_matrix_id: str
    snapshot_id: str
    provider: str = Field(min_length=1, max_length=128)
    method: Literal["navigation", "quoted", "haversine_estimate"]
    calculated_at: datetime = Field(default_factory=utc_now)
    entries: list[RouteEntry]


class Allocation(StrictModel):
    demand_id: str
    facility_id: str | None
    units: int = Field(ge=0)
    covered: bool
    end_to_end_seconds: int | None = Field(default=None, ge=0)
    distance_meters: int | None = Field(default=None, ge=0)
    variable_cost: NonNegativeMoney = Decimal("0")
    rate_id: str | None = None
    reason: str | None = None


class NetworkMetrics(StrictModel):
    total_demand_units: int = Field(ge=0)
    covered_demand_units: int = Field(ge=0)
    uncovered_demand_units: int = Field(ge=0)
    coverage_ratio: float = Field(ge=0, le=1)
    fixed_cost: NonNegativeMoney
    variable_cost: NonNegativeMoney
    total_cost: NonNegativeMoney
    active_facility_count: int = Field(ge=0)


class NetworkScenarioResult(StrictModel):
    schema_version: Literal["network_scenario_result.v1"] = "network_scenario_result.v1"
    result_id: str
    snapshot_id: str
    route_matrix_id: str
    service_policy_id: str
    scenario_id: str
    mode: Literal["current_assignment", "optimized"]
    active_facility_ids: list[str]
    metrics: NetworkMetrics
    allocations: list[Allocation]
    issues: list[str] = Field(default_factory=list)
    calculated_at: datetime = Field(default_factory=utc_now)


class CurrentCoverageResult(StrictModel):
    schema_version: Literal["current_coverage_result.v1"] = "current_coverage_result.v1"
    snapshot_id: str
    route_matrix_id: str
    actual_result_ref: DataRef
    optimized_result_ref: DataRef
    actual_metrics: NetworkMetrics
    optimized_metrics: NetworkMetrics
    interpretation: str


class ScenarioComparison(StrictModel):
    schema_version: Literal["scenario_comparison.v1"] = "scenario_comparison.v1"
    baseline_result_id: str
    candidate_result_id: str
    coverage_ratio_delta: float
    covered_demand_units_delta: int
    total_cost_delta: Decimal
    fixed_cost_delta: Decimal
    variable_cost_delta: Decimal
    comparable: bool = True


class FacilityLocationSolution(StrictModel):
    schema_version: Literal["facility_location_solution.v1"] = (
        "facility_location_solution.v1"
    )
    solution_id: str
    snapshot_id: str
    route_matrix_id: str
    target_coverage_ratio: float = Field(gt=0, le=1)
    status: Literal["optimal", "infeasible"]
    selected_candidate_facility_ids: list[str]
    active_facility_ids: list[str]
    evaluated_subset_count: int = Field(ge=0)
    result_ref: DataRef
    metrics: NetworkMetrics
    method: Literal["exact_subset_enumeration_with_min_cost_flow"] = (
        "exact_subset_enumeration_with_min_cost_flow"
    )
    assumptions: list[str]


class ValidationResult(StrictModel):
    schema_version: Literal["network_validation.v1"] = "network_validation.v1"
    valid: bool
    resource_schema: str
    errors: list[str] = Field(default_factory=list)
    warnings: list[str] = Field(default_factory=list)
    checks: list[str] = Field(default_factory=list)


class ResourceToolResult(StrictModel):
    summary: str
    data_ref: DataRef


class CurrentCoverageToolResult(CurrentCoverageResult):
    summary: str
    data_ref: DataRef


class ComparisonToolResult(ScenarioComparison):
    summary: str
    data_ref: DataRef


class FacilityLocationToolResult(FacilityLocationSolution):
    summary: str
    solution_ref: DataRef
