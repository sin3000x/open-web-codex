"""Typed public contracts for supply-chain network planning."""

from __future__ import annotations

from decimal import Decimal
from typing import Annotated, Any, Literal

from open_web_codex_provider import ResourceRef as _ResourceRef
from pydantic import BaseModel, ConfigDict, Field, RootModel, field_validator, model_validator
from supply_chain_planner.data.mapping import (
    REQUIRED_FIELDS,
    TARGET_ALIASES,
    SourceRole,
    TransformKind,
)
from supply_chain_planner.network.matrix_models import ObservedQuoteMeanCostEvidence, WarehouseScope
from supply_chain_planner.network.models import (
    CurrentAssignmentRecord,
    DataQualityIssue,
    DemandCityRecord,
    PlanningInputIdentity,
    ProvidedRouteFactRecord,
    RouteQuoteRecord,
    WarehouseRecord,
)
from supply_chain_planner.network.optimization_models import (
    AssignmentComparison,
    CityAssignmentChange,
    CostSummary,
    CoverageComparison,
    CoverageMetricSummary,
    OpeningPolicySelection,
    PMedianSolverStage,
)


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    @model_validator(mode="after")
    def validate_canonical_warehouse_ids(self):
        values = getattr(self, "warehouse_ids", None)
        if values is not None and (not values or values != sorted(set(values))):
            raise ValueError("warehouse_ids_not_canonical")
        return self


class ConfirmedFieldDecision(StrictModel):
    source_field: str = Field(min_length=1, max_length=256)
    target_field: str = Field(min_length=1, max_length=128)
    transform: TransformKind
    factor: Decimal | None = None


class SourceSelection(StrictModel):
    """One exact inspected source unit selected for one business role."""

    relative_path: str = Field(min_length=1, max_length=1024)
    unit_ref: str = Field(min_length=1, max_length=512)
    role: SourceRole
    mappings: list[ConfirmedFieldDecision] = Field(default_factory=list, max_length=64)

    @model_validator(mode="after")
    def validate_role_mapping_shape(self) -> SourceSelection:
        if self.role == SourceRole.ADMINISTRATIVE_CATALOG:
            raise ValueError("administrative catalog is prepared by the geography tool")
        allowed = set(TARGET_ALIASES[self.role])
        targets = [mapping.target_field for mapping in self.mappings]
        sources = [mapping.source_field for mapping in self.mappings]
        unknown = sorted(set(targets) - allowed)
        if unknown:
            raise ValueError(
                "source selection target fields are not allowed for "
                f"{self.role.value}: {', '.join(unknown)}; allowed: "
                f"{', '.join(sorted(allowed))}"
            )
        if len(set(targets)) != len(targets):
            raise ValueError("source selection target fields must be unique")
        if len(set(sources)) != len(sources):
            raise ValueError("source selection source fields must be unique")
        for mapping in self.mappings:
            constant_transform = mapping.transform in {
                TransformKind.DIVIDE_CONSTANT,
                TransformKind.MULTIPLY_CONSTANT,
            }
            if constant_transform and (mapping.factor is None or mapping.factor <= 0):
                raise ValueError("source selection transform factor must be positive")
            if not constant_transform and mapping.factor is not None:
                raise ValueError("source selection transform factor is only valid for constants")
            if mapping.transform in {
                TransformKind.ADMINISTRATIVE_LOOKUP,
                TransformKind.COORDINATE_LOOKUP,
            }:
                raise ValueError("source selection transform requires a geography tool")
        return self


class PreparationRoleCounts(StrictModel):
    demand: int = Field(ge=0)
    existing_warehouse: int = Field(ge=0)
    candidate_warehouse: int = Field(ge=0)
    current_assignment: int = Field(ge=0)
    route_quote: int = Field(ge=0)
    provided_route_fact: int = Field(ge=0)


class PreparedSourceSelection(SourceSelection):
    """Resolved v2 provenance for one selected raw source unit."""

    raw_content_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")

    @model_validator(mode="after")
    def validate_resolved_mapping(self) -> PreparedSourceSelection:
        missing = REQUIRED_FIELDS[self.role] - {
            mapping.target_field for mapping in self.mappings
        }
        if not self.mappings or missing:
            raise ValueError("prepared_source_mapping_incomplete")
        canonical = sorted(
            self.mappings,
            key=lambda mapping: (
                mapping.target_field,
                mapping.source_field,
                mapping.transform.value,
                str(mapping.factor),
            ),
        )
        if self.mappings != canonical:
            raise ValueError("prepared_source_mappings_not_canonical")
        return self


class PreparedAdministrativeCatalog(StrictModel):
    relative_path: str = Field(min_length=1, max_length=1024)
    content_sha256: str = Field(pattern=r"^[a-f0-9]{64}$")


class PreparedNetworkResource(StrictModel):
    """Validated, provenance-bound Workspace input for network planning."""

    schema_version: Literal["prepared_network_input.v2"] = Field(
        default="prepared_network_input.v2",
        alias="schemaVersion",
    )
    country_code: str = Field(pattern=r"^[A-Z]{2}$")
    state: Literal["ready", "needs_geography"]
    demand_cities: list[DemandCityRecord]
    warehouses: list[WarehouseRecord]
    current_assignments: list[CurrentAssignmentRecord]
    route_quotes: list[RouteQuoteRecord]
    provided_route_facts: list[ProvidedRouteFactRecord] = Field(default_factory=list)
    issues: list[DataQualityIssue] = Field(default_factory=list, max_length=64)
    issue_count: int = Field(ge=0)
    issues_truncated: bool
    roles: list[SourceRole] = Field(min_length=1, max_length=5)
    source_selections: list[PreparedSourceSelection] = Field(min_length=1, max_length=640)
    administrative_catalog: PreparedAdministrativeCatalog | None = None
    selected_source_identity: str = Field(pattern=r"^[a-f0-9]{64}$")
    role_counts: PreparationRoleCounts

    @model_validator(mode="after")
    def validate_prepared_contract(self) -> PreparedNetworkResource:
        if self.roles != sorted(set(self.roles), key=lambda role: role.value):
            raise ValueError("prepared_roles_not_canonical")
        if set(self.roles) != {selection.role for selection in self.source_selections}:
            raise ValueError("prepared_roles_mismatch")
        selection_keys = [
            (selection.relative_path, selection.unit_ref, selection.role.value)
            for selection in self.source_selections
        ]
        if selection_keys != sorted(selection_keys) or len(selection_keys) != len(set(selection_keys)):
            raise ValueError("prepared_source_selections_not_canonical")
        if any(issue.severity != "warning" for issue in self.issues):
            raise ValueError("prepared_error_issue_forbidden")
        if len(self.issues) != min(self.issue_count, 64):
            raise ValueError("prepared_issue_count_bounded_length_mismatch")
        if self.issues_truncated != (self.issue_count > 64):
            raise ValueError("prepared_issue_count_truncation_mismatch")
        actual = PreparationRoleCounts(
            demand=len(self.demand_cities),
            existing_warehouse=sum(item.is_existing for item in self.warehouses),
            candidate_warehouse=sum(not item.is_existing for item in self.warehouses),
            current_assignment=len(self.current_assignments),
            route_quote=len(self.route_quotes),
            provided_route_fact=len(self.provided_route_facts),
        )
        if actual != self.role_counts:
            raise ValueError("prepared_role_counts_mismatch")
        return self


class GeographyOverride(StrictModel):
    entity: Literal["demand", "warehouse"]
    entity_id: str = Field(min_length=1, max_length=128)
    catalog_city_id: str = Field(min_length=1, max_length=128)


class SourceInspectionIdentity(StrictModel):
    """Identity of the exact regular Workspace files inspected by Data."""

    schema_version: Literal["workspace_source_inspection.v2"] = Field(
        default="workspace_source_inspection.v2",
        alias="schemaVersion",
    )
    content_sha256: str = Field(pattern=r"^[0-9a-f]{64}$")
    source_count: int = Field(ge=1, le=500)


class DataSourceRequirement(StrictModel):
    """A source decision that cannot progress without explicit user input."""

    code: Literal[
        "required_fields_missing",
        "source_role_confirmation_required",
        "field_mapping_confirmation_required",
        "source_data_invalid",
        "source_duplicate_conflict",
        "business_rule_unknown",
    ]
    relative_path: str = Field(min_length=1, max_length=1024)
    unit_ref: str = Field(min_length=1, max_length=512)
    candidate_roles: list[SourceRole] = Field(min_length=1, max_length=5)
    missing_required_fields: list[str] = Field(max_length=16)
    field_name: str | None = Field(default=None, max_length=256)
    question: str = Field(min_length=1, max_length=400)

    @model_validator(mode="after")
    def validate_canonical_requirement(self) -> DataSourceRequirement:
        if self.candidate_roles != sorted(set(self.candidate_roles), key=lambda item: item.value):
            raise ValueError("candidate_roles_not_canonical")
        if self.missing_required_fields != sorted(set(self.missing_required_fields)):
            raise ValueError("missing_required_fields_not_canonical")
        if self.code == "required_fields_missing" and not self.missing_required_fields:
            raise ValueError("missing_required_fields_required")
        return self


class InspectionLimitCounts(StrictModel):
    """Bounded observed and allowed inspection dimensions."""

    files: int = Field(ge=0, le=500)
    units: int = Field(ge=0, le=500)
    bytes: int = Field(ge=0, le=100 * 1024 * 1024)


class DataInspectionInspected(StrictModel):
    """Successful v2 source-unit inspection."""

    outcome: Literal["inspected"]
    schema_version: Literal["workspace_source_profile.v2"] = Field(alias="schemaVersion")
    summary: str
    next_action: Literal["confirm_sources"]
    retryable: Literal[False]
    country_code: str | None = Field(default=None, pattern=r"^[A-Z]{2}$")
    source_profile: dict[str, Any]
    inspection_identity: SourceInspectionIdentity
    inspected_relative_paths: list[str] = Field(min_length=1, max_length=64)
    warnings: list[str] = Field(max_length=64)
    warning_count: int = Field(ge=0)
    warnings_truncated: bool

    @model_validator(mode="after")
    def validate_warning_count(self) -> DataInspectionInspected:
        if len(self.warnings) != min(self.warning_count, 64):
            raise ValueError("inspection_warning_count_bounded_length_mismatch")
        if self.warnings_truncated != (self.warning_count > 64):
            raise ValueError("inspection_warning_count_truncation_mismatch")
        return self


class DataInspectionSelectionRequired(StrictModel):
    """Typed bounded request to reduce inspection scope before retrying."""

    outcome: Literal["selection_required"]
    schema_version: Literal["workspace_source_profile.v2"] = Field(alias="schemaVersion")
    summary: str
    code: Literal["inspection_selection_required"]
    next_action: Literal["select_fewer_sources"]
    retryable: Literal[False]
    observed: InspectionLimitCounts
    limit: InspectionLimitCounts


class PreparedCandidateSummary(StrictModel):
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    input_identity: PlanningInputIdentity
    role_counts: PreparationRoleCounts


class DataInspectionPreparedReady(StrictModel):
    outcome: Literal["prepared_ready"]
    schema_version: Literal["workspace_source_profile.v2"] = Field(alias="schemaVersion")
    operation: Literal["reused"]
    summary: str
    next_action: Literal["handoff"]
    retryable: Literal[False]
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    input_identity: PlanningInputIdentity
    role_counts: PreparationRoleCounts
    warnings: list[str] = Field(max_length=64)
    warning_count: int = Field(ge=0)
    warnings_truncated: bool

    @model_validator(mode="after")
    def validate_warning_count(self) -> DataInspectionPreparedReady:
        if len(self.warnings) != min(self.warning_count, 64):
            raise ValueError("prepared_warning_count_bounded_length_mismatch")
        if self.warnings_truncated != (self.warning_count > 64):
            raise ValueError("prepared_warning_count_truncation_mismatch")
        return self


class DataInspectionPreparedSelectionRequired(StrictModel):
    outcome: Literal["prepared_selection_required"]
    schema_version: Literal["workspace_source_profile.v2"] = Field(alias="schemaVersion")
    summary: str
    next_action: Literal["request_user_input"]
    retryable: Literal[False]
    candidates: list[PreparedCandidateSummary] = Field(max_length=64)
    candidate_count: int = Field(ge=1)
    candidates_truncated: bool

    @model_validator(mode="after")
    def validate_candidate_count(self) -> DataInspectionPreparedSelectionRequired:
        if len(self.candidates) != min(self.candidate_count, 64):
            raise ValueError("prepared_candidate_count_bounded_length_mismatch")
        if self.candidates_truncated != (self.candidate_count > 64):
            raise ValueError("prepared_candidate_count_truncation_mismatch")
        return self


class DataInspectionToolResult(
    RootModel[
        Annotated[
            DataInspectionInspected
            | DataInspectionSelectionRequired
            | DataInspectionPreparedReady
            | DataInspectionPreparedSelectionRequired,
            Field(discriminator="outcome"),
        ]
    ]
):
    """Discriminated v2 inspection contract; no global business blockers."""


class CandidateWarehouseSummary(StrictModel):
    warehouse_id: str = Field(min_length=1, max_length=128)
    warehouse_name: str = Field(min_length=1, max_length=256)
    city_name: str = Field(min_length=1, max_length=256)


class DataPreparationReady(StrictModel):
    outcome: Literal["ready"]
    operation: Literal["created", "reused"]
    summary: str
    next_action: Literal["handoff", "prepare_geography"]
    retryable: Literal[False]
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    input_identity: PlanningInputIdentity
    state: Literal["ready", "needs_geography"]
    role_counts: PreparationRoleCounts
    warnings: list[str] = Field(max_length=64)
    warning_count: int = Field(ge=0)
    warnings_truncated: bool
    candidate_warehouse_count: int = Field(ge=0)
    candidate_warehouses: list[CandidateWarehouseSummary] = Field(max_length=64)
    candidate_warehouses_truncated: bool

    @model_validator(mode="after")
    def validate_bounded_counts(self) -> DataPreparationReady:
        if len(self.warnings) != min(self.warning_count, 64):
            raise ValueError("warning_count_bounded_length_mismatch")
        if self.warnings_truncated != (self.warning_count > 64):
            raise ValueError("warning_count_truncation_mismatch")
        if len(self.candidate_warehouses) != min(self.candidate_warehouse_count, 64):
            raise ValueError("candidate_count_bounded_length_mismatch")
        if self.candidate_warehouses_truncated != (
            self.candidate_warehouse_count > 64
        ):
            raise ValueError("candidate_count_truncation_mismatch")
        return self


class DataPreparationNeedsInput(StrictModel):
    outcome: Literal["needs_input"]
    summary: str
    next_action: Literal["request_user_input"]
    retryable: Literal[False]
    requirements: list[DataSourceRequirement] = Field(min_length=1, max_length=64)
    requirement_count: int = Field(ge=1)
    requirements_truncated: bool

    @model_validator(mode="after")
    def validate_requirement_count(self) -> DataPreparationNeedsInput:
        if len(self.requirements) != min(self.requirement_count, 64):
            raise ValueError("requirement_count_bounded_length_mismatch")
        if self.requirements_truncated != (self.requirement_count > 64):
            raise ValueError("requirement_count_truncation_mismatch")
        return self


class DataPreparationSourceChanged(StrictModel):
    outcome: Literal["source_changed"]
    summary: str
    next_action: Literal["reinspect"]
    retryable: Literal[False]


class DataPreparationToolResult(
    RootModel[
        Annotated[
            DataPreparationReady | DataPreparationNeedsInput | DataPreparationSourceChanged,
            Field(discriminator="outcome"),
        ]
    ]
):
    """Discriminated Data-to-Network preparation result."""


PreparedQueryDataset = Literal[
    "warehouses",
    "demand_cities",
    "current_assignments",
    "route_quotes",
    "provided_route_facts",
]
PreparedQueryScalar = str | bool | float | None
PreparedQueryKey = dict[str, PreparedQueryScalar]


class PreparedQueryFilter(StrictModel):
    """One typed predicate over a normalized prepared record field."""

    field_name: str = Field(min_length=1, max_length=64)
    op: Literal["eq", "ne", "in", "gt", "gte", "lt", "lte"]
    value: PreparedQueryScalar = None
    values: list[str | bool | float] | None = Field(default=None, min_length=1, max_length=64)

    @model_validator(mode="after")
    def validate_operands(self) -> PreparedQueryFilter:
        if self.op == "in":
            if self.values is None or self.value is not None:
                raise ValueError("query_filter_in_requires_values")
        elif self.values is not None:
            raise ValueError("query_filter_values_only_for_in")
        if self.op in {"gt", "gte", "lt", "lte"} and (
            self.value is None or isinstance(self.value, bool)
        ):
            raise ValueError("query_filter_range_requires_value")
        return self


class PreparedQueryMeasure(StrictModel):
    op: Literal["sum", "min", "max", "mean"]
    field_name: str = Field(min_length=1, max_length=64)


class PreparedQueryOrder(StrictModel):
    field_name: str = Field(min_length=1, max_length=64)
    descending: bool = False


class PreparedQueryMeasureValue(StrictModel):
    op: Literal["sum", "min", "max", "mean"]
    field_name: str = Field(min_length=1, max_length=64)
    value: Decimal | None
    value_count: int = Field(ge=0)


class PreparedQueryGroup(StrictModel):
    key: PreparedQueryKey
    count: int = Field(ge=1)
    measures: list[PreparedQueryMeasureValue] = Field(max_length=4)


class PreparedQueryAnswered(StrictModel):
    """Exact facts computed over every record of one prepared dataset."""

    outcome: Literal["answered"]
    schema_version: Literal["prepared_network_query.v1"] = Field(alias="schemaVersion")
    summary: str
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    input_identity: PlanningInputIdentity
    country_code: str = Field(pattern=r"^[A-Z]{2}$")
    roles: list[SourceRole] = Field(min_length=1, max_length=5)
    role_counts: PreparationRoleCounts
    dataset: PreparedQueryDataset
    record_count: int = Field(ge=0)
    matched_count: int = Field(ge=0)
    measures: list[PreparedQueryMeasureValue] = Field(max_length=4)
    groups: list[PreparedQueryGroup] = Field(max_length=200)
    group_count: int = Field(ge=0)
    groups_truncated: bool
    rows: list[PreparedQueryKey] = Field(max_length=100)
    rows_truncated: bool

    @model_validator(mode="after")
    def validate_bounded_counts(self) -> PreparedQueryAnswered:
        if self.matched_count > self.record_count:
            raise ValueError("query_matched_count_exceeds_records")
        if len(self.groups) != min(self.group_count, 200):
            raise ValueError("query_group_count_bounded_length_mismatch")
        if self.groups_truncated != (self.group_count > 200):
            raise ValueError("query_group_count_truncation_mismatch")
        if self.groups and self.rows:
            raise ValueError("query_groups_and_rows_exclusive")
        return self


class PreparedQueryStale(StrictModel):
    """The prepared file no longer matches its selected Workspace sources."""

    outcome: Literal["prepared_input_stale"]
    summary: str
    reason: str = Field(min_length=1, max_length=128)
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    next_action: Literal["reinspect"]
    retryable: Literal[False]


class PreparedQueryToolResult(
    RootModel[
        Annotated[
            PreparedQueryAnswered | PreparedQueryStale,
            Field(discriminator="outcome"),
        ]
    ]
):
    """Discriminated read-only query result over one prepared planning input."""


class PlanningInputOption(StrictModel):
    label: str = Field(min_length=1, max_length=64)
    description: str = Field(min_length=1, max_length=240)


class PlanningInputQuestion(StrictModel):
    id: str = Field(pattern=r"^[a-z][a-z0-9_]{0,63}$")
    header: str = Field(min_length=1, max_length=12)
    question: str = Field(min_length=1, max_length=300)
    options: list[PlanningInputOption] = Field(min_length=2, max_length=3)

    @model_validator(mode="after")
    def validate_options(self) -> PlanningInputQuestion:
        labels = [option.label for option in self.options]
        if len(labels) != len(set(labels)):
            raise ValueError("planning_input_option_labels_duplicate")
        if not labels[0].endswith("(Recommended)"):
            raise ValueError("planning_input_recommended_option_must_be_first")
        return self


class RouteMatrixReady(StrictModel):
    state: Literal["ready"]
    summary: str
    resource_ref: _ResourceRef
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    missing_pair_count: Literal[0] = 0

    @model_validator(mode="after")
    def validate_state_schema(self) -> RouteMatrixReady:
        if self.resource_ref.resource_schema != "route_matrix.v3":
            raise ValueError("ready requires route_matrix.v3")
        return self


class RouteMatrixNeedsInput(StrictModel):
    state: Literal["needs_input"]
    summary: str
    next_action: Literal["request_user_input"]
    retryable: Literal[False]
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    missing_pair_count: int = Field(gt=0)
    missing_pairs: list[tuple[str, str, Literal["last_mile", "linehaul"]]] = Field(
        min_length=1,
        max_length=20,
    )
    missing_pairs_truncated: bool
    questions: list[PlanningInputQuestion] = Field(min_length=1, max_length=3)

    @model_validator(mode="after")
    def validate_missing_pairs(self) -> RouteMatrixNeedsInput:
        if len(self.missing_pairs) != min(self.missing_pair_count, 20):
            raise ValueError("route_missing_pair_count_bounded_length_mismatch")
        if self.missing_pairs_truncated != (self.missing_pair_count > 20):
            raise ValueError("route_missing_pair_count_truncation_mismatch")
        if self.missing_pairs != sorted(set(self.missing_pairs)):
            raise ValueError("route_missing_pairs_not_canonical")
        return self


class RouteMatrixPreparationToolResult(
    RootModel[
        Annotated[
            RouteMatrixReady | RouteMatrixNeedsInput,
            Field(discriminator="state"),
        ]
    ]
):
    """Complete route matrix or one bounded request for a completion choice."""


class CostMatrixPlanningToolResult(StrictModel):
    """Bounded cost-matrix result with auditable fallback-policy evidence."""

    summary: str
    resource_ref: _ResourceRef
    input_identity: PlanningInputIdentity
    calculation_rule_source: Literal["explicit", "observed_quote_mean"] | None = None
    calculation_rule_evidence: ObservedQuoteMeanCostEvidence | None = None
    calculation_rule_evidence_path: str | None = Field(default=None, max_length=1024)
    expected_pair_count: int = Field(ge=0)
    reused_pair_count: int = Field(ge=0)
    computed_pair_count: int = Field(ge=0)
    missing_pair_count: int = Field(ge=0)
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)


class PMedianSolutionToolResult(StrictModel):
    """Bounded facility-location result for model and E2E verification."""

    summary: str
    resource_ref: _ResourceRef
    input_identity: PlanningInputIdentity
    status: Literal["optimal", "feasible", "timeout", "infeasible", "unavailable"]
    optimality: Literal["proven", "feasible_only", "not_available"]
    opening_policy: OpeningPolicySelection
    selected_number_to_open: int | None = Field(default=None, ge=0)
    minimum_number_to_open_proven: bool
    solver_stages: list[PMedianSolverStage] = Field(min_length=1, max_length=2)
    active_warehouse_count: int = Field(ge=0)
    opened_candidate_ids: list[str] = Field(max_length=64)
    closed_existing_ids: list[str] = Field(max_length=64)
    cost: CostSummary | None = None
    coverage: list[CoverageMetricSummary] = Field(max_length=32)

    @model_validator(mode="after")
    def validate_solution_contract(self):
        if self.selected_number_to_open is not None and self.selected_number_to_open != len(
            self.opened_candidate_ids
        ):
            raise ValueError("selected_number_to_open_mismatch")
        if self.status in {"optimal", "feasible"}:
            if self.selected_number_to_open is None:
                raise ValueError("selected_number_to_open_required")
            if (self.status, self.optimality) not in {
                ("optimal", "proven"),
                ("feasible", "feasible_only"),
            }:
                raise ValueError("solution_status_optimality_mismatch")
        else:
            if (self.status, self.optimality) not in {
                ("timeout", "not_available"),
                ("infeasible", "not_available"),
                ("unavailable", "not_available"),
            }:
                raise ValueError("solution_status_optimality_mismatch")
        if self.status not in {"optimal", "feasible"} and (
            self.selected_number_to_open is not None
            or self.active_warehouse_count
            or self.opened_candidate_ids
            or self.closed_existing_ids
            or self.cost is not None
            or self.coverage
        ):
            raise ValueError("unsolved_solution_fields_forbidden")
        if self.minimum_number_to_open_proven:
            first = self.solver_stages[0]
            if not (
                self.opening_policy.kind == "minimum_feasible"
                and first.kind == "minimum_openings"
                and first.status == "optimal"
                and first.optimality == "proven"
            ):
                raise ValueError("minimum_number_proof_invalid")
        return self


class NavigationMatrixRequestToolResult(StrictModel):
    summary: str
    state: Literal["execution_required", "ready"]
    navigation_request_relative_path: str | None = Field(default=None, max_length=1024)
    input_identity: PlanningInputIdentity
    warehouse_scope: WarehouseScope
    warehouse_ids: list[str] = Field(min_length=1, max_length=256)
    route_count: int = Field(ge=0)
    estimated_billable_elements: int = Field(ge=0)


class UncoveredCitySummary(StrictModel):
    demand_city_id: str = Field(min_length=1, max_length=128)
    demand_city_name: str = Field(min_length=1, max_length=256)
    warehouse_id: str | None = Field(default=None, max_length=128)
    warehouse_name: str | None = Field(default=None, max_length=256)
    duration_hours: float | None = Field(default=None, ge=0)
    demand_quantity: Decimal = Field(ge=0)


class NetworkBaselineResourceToolResult(StrictModel):
    summary: str
    resource_ref: _ResourceRef
    coverage_metrics: list[CoverageMetricSummary] = Field(max_length=32)
    detail_target_hours: float = Field(gt=0)
    uncovered_city_count: int = Field(ge=0)
    uncovered_cities: list[UncoveredCitySummary] = Field(max_length=10)
    uncovered_cities_truncated: bool


class FacilityChangeCostComparison(StrictModel):
    """Bounded cost projection for a before-versus-after facility change."""

    currency: str | None = Field(default=None, pattern=r"^[A-Z]{3}$")
    before_total: float | None = Field(default=None, ge=0)
    after_total: float | None = Field(default=None, ge=0)
    delta: float | None = None
    complete: bool


class NetworkScenarioResourceRef(_ResourceRef):
    resource_schema: Literal["network_scenario.v2"] = "network_scenario.v2"


class AssignmentResultResourceRef(_ResourceRef):
    """A solved result that can be rendered as a single coverage map."""

    resource_schema: Literal[
        "network_baseline.v2",
        "network_scenario.v2",
        "facility_location_solution.v4",
    ]


class ComparableNetworkResultRef(_ResourceRef):
    resource_schema: Literal[
        "network_baseline.v2",
        "network_scenario.v2",
        "facility_location_solution.v4",
    ] = Field(description="Comparable network result schema.")


class NetworkPlanComparisonResourceRef(_ResourceRef):
    resource_schema: Literal["network_plan_comparison.v2"] = "network_plan_comparison.v2"


class NetworkPlanComparisonResource(StrictModel):
    """One complete, provenance-bound before-versus-after planning result."""

    schema_version: Literal["network_plan_comparison.v2"] = "network_plan_comparison.v2"
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    input_identity: PlanningInputIdentity
    before_ref: ComparableNetworkResultRef
    after_ref: ComparableNetworkResultRef
    comparison: AssignmentComparison


class FacilityChangeAssessmentToolResult(StrictModel):
    """Bounded result of one deterministic facility-change assessment."""

    summary: str = Field(min_length=1, max_length=2048)
    scenario_ref: NetworkScenarioResourceRef
    plan_comparison_ref: NetworkPlanComparisonResourceRef
    active_warehouse_count: int = Field(ge=0)
    added_warehouse_ids: list[str] = Field(max_length=256)
    removed_warehouse_ids: list[str] = Field(max_length=256)
    cost: FacilityChangeCostComparison
    coverage: list[CoverageComparison] = Field(max_length=32)
    affected_city_count: int = Field(ge=0)
    affected_city_ids: list[str] = Field(max_length=10)
    affected_city_ids_truncated: bool
    affected_city_changes: list[CityAssignmentChange] = Field(max_length=10)
    affected_city_changes_truncated: bool
    reassigned_city_count: int = Field(ge=0)
    reassigned_city_ids: list[str] = Field(max_length=10)
    reassigned_city_ids_truncated: bool


class NetworkFinalArtifactDescriptor(StrictModel):
    """Platform-readable descriptor for one explicit final domain deliverable."""

    artifact_schema: Literal["network_planning_report_markdown.v2"] = Field(alias="schema")
    display_name: str = Field(min_length=1, max_length=256, alias="displayName")
    mime_type: Literal["text/markdown"] = Field(alias="mimeType")
    workspace_relative_path: str = Field(
        min_length=1,
        max_length=1024,
        alias="workspaceRelativePath",
    )
    byte_size: int = Field(ge=1, alias="byteSize")


class NetworkFinalArtifactToolResult(StrictModel):
    summary: str = Field(min_length=1, max_length=512)
    artifact: NetworkFinalArtifactDescriptor


class NetworkBaselineReportInput(StrictModel):
    """Exact typed inputs for a single current-network assessment brief."""

    mode: Literal["baseline"] = "baseline"
    prepared_input_relative_path: str = Field(min_length=1, max_length=1024)
    baseline_ref: _ResourceRef


class NetworkComparisonReportInput(StrictModel):
    """Exact typed inputs for a generic before-versus-after comparison brief."""

    mode: Literal["comparison"] = "comparison"
    plan_comparison_ref: Annotated[
        NetworkPlanComparisonResourceRef,
        Field(description="The exact provenance-bound planning comparison to report."),
    ]

    @field_validator("plan_comparison_ref", mode="before")
    @classmethod
    def validate_plan_comparison_ref(cls, value: object) -> NetworkPlanComparisonResourceRef:
        if isinstance(value, _ResourceRef):
            value = value.model_dump(mode="json")
        return NetworkPlanComparisonResourceRef.model_validate(value)


NetworkReportInput = Annotated[
    NetworkBaselineReportInput | NetworkComparisonReportInput,
    Field(discriminator="mode"),
]
