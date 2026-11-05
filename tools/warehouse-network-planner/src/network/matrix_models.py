"""Distance, duration and transport-cost matrix contracts."""

from __future__ import annotations

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator
from supply_chain_planner.network.models import PlanningInputIdentity


class MatrixModel(BaseModel):
    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    @model_validator(mode="after")
    def validate_canonical_warehouse_ids(self):
        values = getattr(self, "warehouse_ids", None)
        if values is not None and (not values or values != sorted(set(values))):
            raise ValueError("warehouse_ids_not_canonical")
        return self


RouteMethod = Literal["haversine", "navigation", "provided"]
NetworkLayer = Literal["linehaul", "last_mile"]


class ExistingOnlyWarehouseScope(MatrixModel):
    kind: Literal["existing_only"] = "existing_only"


class ExistingPlusCandidatesWarehouseScope(MatrixModel):
    kind: Literal["existing_plus_candidates"] = "existing_plus_candidates"
    candidate_ids: list[str] = Field(min_length=1, max_length=256)

    @field_validator("candidate_ids")
    @classmethod
    def normalize_candidate_ids(cls, values: list[str]) -> list[str]:
        if any(not value for value in values):
            raise ValueError("warehouse_scope_candidate_id_invalid")
        if len(values) != len(set(values)):
            raise ValueError("warehouse_scope_candidate_ids_duplicate")
        return sorted(set(values))


class AllWarehousesScope(MatrixModel):
    kind: Literal["all_warehouses"] = "all_warehouses"


WarehouseScope = Annotated[
    ExistingOnlyWarehouseScope | ExistingPlusCandidatesWarehouseScope | AllWarehousesScope,
    Field(discriminator="kind"),
]


class RouteMatrixPlan(MatrixModel):
    schema_version: Literal["route_matrix_plan.v3"] = "route_matrix_plan.v3"
    origin_count: int = Field(ge=1)
    destination_count: int = Field(ge=1)
    route_count: int = Field(ge=1)
    method: RouteMethod
    detour_coefficient: float | None = Field(default=None, gt=0)
    average_speed_kph: float | None = Field(default=None, gt=0)
    estimated_billable_calls: int = Field(ge=0)
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    input_identity: PlanningInputIdentity


class RouteMatrixRow(MatrixModel):
    origin_id: str = Field(min_length=1, max_length=128)
    destination_id: str = Field(min_length=1, max_length=128)
    layer: NetworkLayer
    distance_km: float = Field(ge=0)
    duration_hours: float = Field(ge=0)
    method: RouteMethod
    tool_version: str = Field(min_length=1, max_length=128)
    status: Literal["ready", "unreachable", "error"] = "ready"
    origin_longitude: float | None = Field(default=None, ge=-180, le=180)
    origin_latitude: float | None = Field(default=None, ge=-90, le=90)
    destination_longitude: float | None = Field(default=None, ge=-180, le=180)
    destination_latitude: float | None = Field(default=None, ge=-90, le=90)
    detour_coefficient: float | None = Field(default=None, gt=0)
    average_speed_kph: float | None = Field(default=None, gt=0)
    navigation_provider: str | None = Field(default=None, min_length=1, max_length=128)
    navigation_profile: str | None = Field(default=None, min_length=1, max_length=128)


class ProvidedRouteMatrixStats(MatrixModel):
    kind: Literal["provided"] = "provided"
    expected_pair_count: int = Field(ge=0)
    provided_pair_count: int = Field(ge=0)
    ignored_input_pair_count: int = Field(ge=0)
    missing_pair_count: int = Field(ge=0)
    complete: bool
    source_method_counts: dict[str, int]


class HaversineRouteMatrixStats(MatrixModel):
    kind: Literal["haversine"] = "haversine"
    expected_pair_count: int = Field(ge=0)
    provided_pair_count: int = Field(ge=0)
    reused_pair_count: int = Field(ge=0)
    computed_pair_count: int = Field(ge=0)
    stale_pair_count: int = Field(ge=0)
    ignored_prior_row_count: int = Field(ge=0)
    missing_pair_count: int = Field(ge=0)
    last_mile_pair_count: int = Field(ge=0)
    linehaul_pair_count: int = Field(ge=0)
    detour_coefficient: float = Field(gt=0)
    average_speed_kph: float = Field(gt=0)
    complete: bool


class NavigationRouteMatrixStats(MatrixModel):
    kind: Literal["navigation"] = "navigation"
    route_count: int = Field(ge=0)
    reused_pair_count: int = Field(ge=0)
    registered_pair_count: int = Field(ge=0)
    missing_pair_count: int = Field(ge=0)
    complete: bool


class NavigationRouteRequest(MatrixModel):
    origin_id: str = Field(min_length=1, max_length=128)
    destination_id: str = Field(min_length=1, max_length=128)
    layer: NetworkLayer
    origin_longitude: float = Field(ge=-180, le=180)
    origin_latitude: float = Field(ge=-90, le=90)
    destination_longitude: float = Field(ge=-180, le=180)
    destination_latitude: float = Field(ge=-90, le=90)


class NavigationMatrixRequest(MatrixModel):
    """Exact lane set approved for one billable navigation execution."""

    schema_version: Literal["navigation_matrix_request.v2"] = "navigation_matrix_request.v2"
    input_identity: PlanningInputIdentity
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    routes: list[NavigationRouteRequest] = Field(default_factory=list)
    estimated_billable_elements: int = Field(ge=0)

    @model_validator(mode="after")
    def validate_route_keys(self):
        keys = [(route.origin_id, route.destination_id, route.layer) for route in self.routes]
        if len(keys) != len(set(keys)):
            raise ValueError("navigation_route_duplicate_pair")
        return self


class NavigationMatrixResult(MatrixModel):
    """Provider-produced lane facts awaiting planner-side import and validation."""

    schema_version: Literal["navigation_matrix_result.v2"] = "navigation_matrix_result.v2"
    input_identity: PlanningInputIdentity
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    rows: list[RouteMatrixRow] = Field(min_length=1)

    @model_validator(mode="after")
    def validate_route_keys(self):
        keys = [(row.origin_id, row.destination_id, row.layer) for row in self.rows]
        if len(keys) != len(set(keys)):
            raise ValueError("navigation_route_duplicate_pair")
        return self


RouteMatrixStats = Annotated[
    ProvidedRouteMatrixStats | HaversineRouteMatrixStats | NavigationRouteMatrixStats,
    Field(discriminator="kind"),
]


class RouteMatrix(MatrixModel):
    schema_version: Literal["route_matrix.v3"] = "route_matrix.v3"
    method: RouteMethod
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    rows: list[RouteMatrixRow] = Field(default_factory=list)
    missing_routes: list[tuple[str, str, NetworkLayer]] = Field(default_factory=list)
    stats: RouteMatrixStats
    input_identity: PlanningInputIdentity

    @model_validator(mode="after")
    def validate_method_stats(self):
        if self.method != self.stats.kind:
            raise ValueError("route_matrix_stats_method_mismatch")
        return self


class RouteCostQuote(MatrixModel):
    origin_id: str = Field(min_length=1, max_length=128)
    destination_id: str = Field(min_length=1, max_length=128)
    layer: NetworkLayer
    price_per_vehicle: float = Field(ge=0)
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    vehicle_capacity: int = Field(gt=0)


class RouteFactProvenance(MatrixModel):
    method: RouteMethod
    tool_version: str = Field(min_length=1, max_length=128)
    origin_longitude: float = Field(ge=-180, le=180)
    origin_latitude: float = Field(ge=-90, le=90)
    destination_longitude: float = Field(ge=-180, le=180)
    destination_latitude: float = Field(ge=-90, le=90)
    detour_coefficient: float | None = Field(default=None, gt=0)
    average_speed_kph: float | None = Field(default=None, gt=0)
    navigation_provider: str | None = Field(default=None, min_length=1, max_length=128)
    navigation_profile: str | None = Field(default=None, min_length=1, max_length=128)


class CostMatrixRow(MatrixModel):
    origin_id: str = Field(min_length=1, max_length=128)
    destination_id: str = Field(min_length=1, max_length=128)
    layer: NetworkLayer
    cost_per_demand_unit: float = Field(ge=0)
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    source: Literal["quote", "calculated"]
    tool_version: str = Field(min_length=1, max_length=128)
    quote_price_per_vehicle: float | None = Field(default=None, ge=0)
    quote_vehicle_capacity: float | None = Field(default=None, gt=0)
    route_distance_km: float | None = Field(default=None, ge=0)
    fixed_cost_per_demand_unit: float | None = Field(default=None, ge=0)
    cost_per_km_per_demand_unit: float | None = Field(default=None, ge=0)
    route_fact: RouteFactProvenance | None = None


class DemandUnitCostRule(MatrixModel):
    """Explicit cost units for one route layer."""

    layer: NetworkLayer
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    fixed_cost_per_demand_unit: float = Field(ge=0)
    cost_per_km_per_demand_unit: float = Field(ge=0)


class CostCalculationPolicy(MatrixModel):
    rules: list[DemandUnitCostRule] = Field(min_length=1)


class ExplicitCostPolicy(MatrixModel):
    """Caller-supplied numeric cost rules."""

    kind: Literal["explicit"] = "explicit"
    rules: list[DemandUnitCostRule] = Field(min_length=1)


class ObservedQuoteMeanCostPolicy(MatrixModel):
    """Derive per-layer fallback unit costs from all matching normalized quotes."""

    kind: Literal["observed_quote_mean"] = "observed_quote_mean"


CostPolicySelection = Annotated[
    ExplicitCostPolicy | ObservedQuoteMeanCostPolicy,
    Field(discriminator="kind"),
]


class QuoteMeanCostRuleEvidence(MatrixModel):
    layer: NetworkLayer
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    quote_count: int = Field(gt=0)
    mean_cost_per_demand_unit: float = Field(ge=0)


class ObservedQuoteMeanCostEvidence(MatrixModel):
    """Stable evidence produced by a full prepared-input quote aggregation.

    The evidence file is intentionally small enough to pass through the Agent
    context, while the Planner re-derives and validates it from the complete
    prepared input before accepting it.  This keeps a user-authored script
    auditable without allowing its numeric output to become a second source of
    business truth.
    """

    schema_version: Literal["warehouse_quote_mean_calculation.v1"]
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    input_identity: PlanningInputIdentity
    total_quote_count: int = Field(gt=0)
    method: Literal["observed_quote_mean"]
    tool_version: Literal["observed-quote-mean.v1"]
    formula: Literal["arithmetic_mean(price_per_vehicle / vehicle_capacity)"]
    considered_quote_count: int = Field(gt=0)
    ignored_quote_count: int = Field(ge=0)
    rules: list[QuoteMeanCostRuleEvidence] = Field(min_length=1)


class CostMatrixStats(MatrixModel):
    expected_pair_count: int = Field(ge=0)
    reused_pair_count: int = Field(ge=0)
    computed_pair_count: int = Field(ge=0)
    missing_pair_count: int = Field(ge=0)
    ignored_quote_count: int = Field(ge=0)
    ignored_prior_row_count: int = Field(ge=0)
    stale_pair_count: int = Field(ge=0)
    complete: bool


class CostMatrix(MatrixModel):
    schema_version: Literal["cost_matrix.v3"] = "cost_matrix.v3"
    currency: str = Field(pattern=r"^[A-Z]{3}$")
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    rows: list[CostMatrixRow] = Field(default_factory=list)
    missing_routes: list[tuple[str, str, NetworkLayer]] = Field(default_factory=list)
    calculation_rule: CostCalculationPolicy | None = None
    calculation_rule_source: Literal["explicit", "observed_quote_mean"] | None = None
    calculation_rule_evidence: ObservedQuoteMeanCostEvidence | None = None
    calculation_rule_evidence_path: str | None = Field(default=None, max_length=1024)
    stats: CostMatrixStats
    input_identity: PlanningInputIdentity


class RouteMatrixValidation(MatrixModel):
    valid: bool
    errors: list[str]
    missing_routes: list[tuple[str, str, NetworkLayer]]
    route_count: int = Field(ge=0)
