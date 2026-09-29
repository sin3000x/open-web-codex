"""Owner module for delivery tools."""

import urllib.parse
from typing import (
    Annotated,
    Literal,
)

from mcp.server.fastmcp import (
    Context,
)
from mcp.types import (
    CallToolResult,
    TextContent,
)
from open_web_codex_provider import (
    MAX_WORKSPACE_FILE_BYTES,
    GeoJsonResourceRef,
    ResourceRef,
    derive_geojson_profile,
)
from pydantic import (
    Field,
)
from supply_chain_planner.delivery.map_service import (
    NetworkComparisonGeoJson,
    NetworkCoverageGeoJson,
    NetworkDistributionGeoJson,
    build_network_comparison_geojson,
    build_network_coverage_geojson,
    build_network_distribution_geojson,
)
from supply_chain_planner.delivery.report_service import (
    NETWORK_PLANNING_MARKDOWN_SCHEMA,
    build_network_baseline_assessment_report_bundle,
    build_network_planning_report_bundle,
    render_network_baseline_assessment_markdown,
    render_network_planning_report_markdown,
)
from supply_chain_planner.network.models import (
    NormalizedInputBatch,
    PlanningInputIdentity,
)
from supply_chain_planner.network.optimization_models import (
    AssignmentComparison,
    AssignmentResult,
    BaselineResult,
    ComparableNetworkView,
    PMedianSolution,
    ScenarioResult,
)
from supply_chain_planner.shared.models import (
    AssignmentResultResourceRef,
    NetworkFinalArtifactDescriptor,
    NetworkFinalArtifactToolResult,
    NetworkPlanComparisonResource,
    NetworkPlanComparisonResourceRef,
    NetworkReportInput,
    PreparedNetworkResource,
)
from supply_chain_planner.shared.planning_input import (
    require_matching_input,
)
from supply_chain_planner.shared.workspace_outputs import (
    WorkspaceOutputKind,
    prepare_workspace_output_path,
)

from .tool_runtime import (
    CONTENT_ADDRESSED_RESOURCE_TOOL,
    FINAL_WORKSPACE_DELIVERY_TOOL,
    McpResourceContractError,
    _load_comparable_resource,
    _load_ready_network,
    _runtime,
)


def _load_assignment_coverage_result(
    resource_ref: AssignmentResultResourceRef,
) -> tuple[
    AssignmentResult,
    list[str],
    str,
    Literal["before", "scenario", "after"],
    PlanningInputIdentity,
    list[float],
]:
    """Adapt one solved domain result without choosing or recomputing it."""
    if resource_ref.resource_schema == "network_baseline.v2":
        baseline = _runtime().load_model(resource_ref, "network_baseline.v2", BaselineResult)
        return (
            baseline.assignment,
            baseline.active_warehouse_ids,
            baseline.label,
            "before",
            baseline.input_identity,
            [metric.target_hours for metric in baseline.coverage],
        )
    if resource_ref.resource_schema == "network_scenario.v2":
        scenario = _runtime().load_model(resource_ref, "network_scenario.v2", ScenarioResult)
        return (
            scenario.assignment,
            scenario.active_warehouse_ids,
            "scenario",
            "scenario",
            scenario.input_identity,
            [metric.target_hours for metric in scenario.service],
        )
    facility = _runtime().load_model(resource_ref, "facility_location_solution.v4", PMedianSolution)
    if facility.assignment is None:
        raise McpResourceContractError("coverage_assignment_required")
    if facility.status not in {"optimal", "feasible"}:
        raise McpResourceContractError("coverage_solution_not_deliverable")
    return (
        facility.assignment,
        facility.active_warehouse_ids,
        facility.status,
        "after",
        facility.input_identity,
        [metric.target_hours for metric in facility.service],
    )


def _publish_geojson(
    schema: str,
    value: NetworkComparisonGeoJson | NetworkCoverageGeoJson | NetworkDistributionGeoJson,
    description: str,
) -> CallToolResult:
    # A map source advertises only properties that exist for the exact result.
    # Keeping optional comparison fields as JSON null makes them look usable to
    # a style author even though every rendered feature will evaluate to the
    # Mapbox fallback.  The immutable GeoJSON and its derived profile must use
    # the same compact, non-null payload.
    payload = value.model_dump(mode="json", by_alias=True, exclude_none=True)
    if payload.get("type") != "FeatureCollection" or not isinstance(payload.get("features"), list):
        raise McpResourceContractError("geojson_feature_collection_required")
    result = _runtime().publish(
        schema,
        payload,
        description,
        mime_type="application/geo+json",
    )
    structured = result.structuredContent
    if structured is None:
        raise McpResourceContractError("map_data_result_missing")
    resource_ref = ResourceRef.model_validate(structured["resource_ref"])
    data_ref = GeoJsonResourceRef(
        server=resource_ref.server,
        uri=resource_ref.uri,
        resource_schema=resource_ref.resource_schema,
        profile=derive_geojson_profile(payload),
    )
    structured.pop("resource_ref", None)
    structured["data_ref"] = data_ref.model_dump(mode="json")
    return result


def _load_final_delivery_inputs(
    plan_comparison_ref: NetworkPlanComparisonResourceRef,
    ctx: Context,
) -> tuple[
    PreparedNetworkResource,
    NormalizedInputBatch,
    ComparableNetworkView,
    ComparableNetworkView,
    AssignmentComparison,
]:
    plan_comparison = _runtime().load_model(
        plan_comparison_ref,
        "network_plan_comparison.v2",
        NetworkPlanComparisonResource,
    )
    prepared, input_identity = _load_ready_network(
        plan_comparison.prepared_input_relative_path,
        ctx,
    )
    before_view = _load_comparable_resource(plan_comparison.before_ref)
    after_view = _load_comparable_resource(plan_comparison.after_ref)
    try:
        require_matching_input(plan_comparison.input_identity, input_identity)
        require_matching_input(input_identity, before_view.input_identity)
        require_matching_input(input_identity, after_view.input_identity)
    except ValueError as error:
        raise McpResourceContractError("delivery_input_identity_mismatch") from error
    normalized = NormalizedInputBatch(
        demand_cities=prepared.demand_cities,
        warehouses=prepared.warehouses,
        current_assignments=prepared.current_assignments,
        route_quotes=prepared.route_quotes,
        provided_route_facts=prepared.provided_route_facts,
        issues=prepared.issues,
    )
    return prepared, normalized, before_view, after_view, plan_comparison.comparison


def _write_final_delivery_markdown(
    markdown: str,
    output_relative_path: str,
    ctx: Context,
    summary: str,
) -> CallToolResult:
    output_relative_path = prepare_workspace_output_path(
        _runtime().require_workspace(ctx),
        output_relative_path,
        WorkspaceOutputKind.DELIVERY_MARKDOWN,
    )
    content = markdown.encode("utf-8")
    try:
        created = _runtime().create_workspace_file(
            ctx,
            output_relative_path,
            content,
            max_bytes=MAX_WORKSPACE_FILE_BYTES,
        )
    except McpResourceContractError:
        raise
    except (OSError, ValueError) as error:
        raise McpResourceContractError("workspace_file_invalid") from error
    structured = NetworkFinalArtifactToolResult(
        summary=summary,
        artifact=NetworkFinalArtifactDescriptor(
            schema=NETWORK_PLANNING_MARKDOWN_SCHEMA,
            displayName="Warehouse network planning report",
            mimeType="text/markdown",
            workspaceRelativePath=created.relative_path,
            byteSize=created.byte_size,
        ),
    )
    encoded_path = urllib.parse.quote(created.relative_path, safe="/-._~")
    link_text = f"正式简报已生成：[下载中文 Markdown 简报]({encoded_path})"
    return CallToolResult(
        content=[TextContent(type="text", text=link_text)],
        structuredContent=structured.model_dump(mode="json", by_alias=True),
    )


def prepare_network_distribution_map(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    ctx: Context,
    include_candidates: bool = False,
    baseline_ref: Annotated[
        ResourceRef | None,
        Field(
            description=(
                "Optional exact network_baseline.v2 result for adding raw per-city "
                "assignment, distance, duration, and unit-cost properties."
            )
        ),
    ] = None,
) -> CallToolResult:
    """Publish raw GeoJSON facts for a separately authored map presentation.

    如需展示时效达标/未达标（SLA attainment），请改用
    prepare_network_coverage_map（需 assignment_result_ref 和 service_target_hours 参数）。
    """
    prepared, input_identity = _load_ready_network(prepared_input_relative_path, ctx)
    normalized = NormalizedInputBatch(
        demand_cities=prepared.demand_cities,
        warehouses=prepared.warehouses,
        current_assignments=prepared.current_assignments,
        route_quotes=prepared.route_quotes,
        provided_route_facts=prepared.provided_route_facts,
        issues=prepared.issues,
    )
    baseline = (
        _runtime().load_model(baseline_ref, "network_baseline.v2", BaselineResult)
        if baseline_ref is not None
        else None
    )
    if baseline is not None:
        try:
            require_matching_input(input_identity, baseline.input_identity)
        except ValueError as error:
            raise McpResourceContractError("map_input_identity_mismatch") from error
    geojson = build_network_distribution_geojson(
        normalized,
        include_candidates=include_candidates,
        baseline=baseline,
    )
    demand_count = len(prepared.demand_cities)
    existing_count = sum(warehouse.is_existing for warehouse in prepared.warehouses)
    candidate_count = (
        sum(not warehouse.is_existing for warehouse in prepared.warehouses)
        if include_candidates
        else 0
    )
    summary = (
        f"Prepared interactive map data with {demand_count} demand cities, "
        f"{existing_count} existing warehouses, and {candidate_count} candidates."
    )
    result = _publish_geojson(geojson.schema_version, geojson, summary)
    structured = result.structuredContent
    if structured is None:
        raise McpResourceContractError("map_data_result_missing")
    structured.update(
        {
            "feature_count": len(geojson.features),
            "feature_counts": {
                "demand": demand_count,
                "existing_warehouses": existing_count,
                "candidate_warehouses": candidate_count,
            },
        }
    )
    return result


def prepare_network_comparison_map(
    plan_comparison_ref: NetworkPlanComparisonResourceRef,
    service_target_hours: Annotated[float, Field(gt=0)],
    ctx: Context,
) -> CallToolResult:
    """Publish raw before-versus-after GeoJSON at one compared service target."""
    _prepared, normalized, before, after, comparison = _load_final_delivery_inputs(
        plan_comparison_ref,
        ctx,
    )
    if service_target_hours not in comparison.requested_service_targets:
        raise McpResourceContractError("comparison_map_service_target_unavailable")
    geojson = build_network_comparison_geojson(
        normalized,
        before,
        after,
        comparison,
        service_target_hours=service_target_hours,
    )
    summary = (
        f"Prepared an interactive comparison map with {len(geojson.features)} "
        "features from the validated before and after results."
    )
    result = _publish_geojson(geojson.schema_version, geojson, summary)
    structured = result.structuredContent
    if structured is None:
        raise McpResourceContractError("map_data_result_missing")
    structured.update(
        {
            "feature_count": len(geojson.features),
            "service_target_hours": service_target_hours,
        }
    )
    return result


def prepare_network_coverage_map(
    prepared_input_relative_path: Annotated[str, Field(min_length=1, max_length=1024)],
    assignment_result_ref: AssignmentResultResourceRef,
    service_target_hours: Annotated[float, Field(gt=0)],
    ctx: Context,
) -> CallToolResult:
    """Publish all straight-line coverage facts for one exact solved result.

    如只需展示仓库和需求点位置（不含 SLA 达标信息），请改用
    prepare_network_distribution_map。
    """
    prepared, input_identity = _load_ready_network(prepared_input_relative_path, ctx)
    assignment, active_ids, result_label, scenario, result_identity, available_targets = (
        _load_assignment_coverage_result(assignment_result_ref)
    )
    if service_target_hours not in available_targets:
        raise McpResourceContractError("coverage_map_service_target_unavailable")
    try:
        require_matching_input(input_identity, result_identity)
    except ValueError as error:
        raise McpResourceContractError("coverage_map_input_identity_mismatch") from error
    normalized = NormalizedInputBatch(
        demand_cities=prepared.demand_cities,
        warehouses=prepared.warehouses,
        current_assignments=prepared.current_assignments,
        route_quotes=prepared.route_quotes,
        provided_route_facts=prepared.provided_route_facts,
        issues=prepared.issues,
    )
    geojson = build_network_coverage_geojson(
        normalized,
        assignment,
        active_ids,
        result_label=result_label,
        scenario=scenario,
        service_target_hours=service_target_hours,
    )
    result = _publish_geojson(
        geojson.schema_version,
        geojson,
        f"Prepared {len(geojson.features)} map features, including every assigned city coverage line.",
    )
    structured = result.structuredContent
    if structured is None:
        raise McpResourceContractError("map_data_result_missing")
    structured.update(
        {
            "feature_count": len(geojson.features),
            "service_target_hours": service_target_hours,
        }
    )
    return result


def publish_network_planning_report(
    report_input: NetworkReportInput,
    output_relative_path: Annotated[
        str,
        Field(
            min_length=1,
            max_length=1024,
            description=(
                "Create-new Markdown path directly under "
                "outputs/warehouse-network/deliverables/."
            ),
        ),
    ],
    ctx: Context,
) -> CallToolResult:
    """Create a Markdown brief for a baseline assessment or plan comparison."""
    if report_input.mode == "baseline":
        prepared, input_identity = _load_ready_network(
            report_input.prepared_input_relative_path,
            ctx,
        )
        normalized = NormalizedInputBatch(
            demand_cities=prepared.demand_cities,
            warehouses=prepared.warehouses,
            current_assignments=prepared.current_assignments,
            route_quotes=prepared.route_quotes,
            provided_route_facts=prepared.provided_route_facts,
            issues=prepared.issues,
        )
        baseline = _runtime().load_model(
            report_input.baseline_ref,
            "network_baseline.v2",
            BaselineResult,
        )
        try:
            require_matching_input(input_identity, baseline.input_identity)
        except ValueError as error:
            raise McpResourceContractError("report_input_identity_mismatch") from error
        bundle = build_network_baseline_assessment_report_bundle(
            normalized,
            baseline,
            country_code=prepared.country_code,
        )
        markdown = render_network_baseline_assessment_markdown(bundle)
    else:
        prepared, normalized, before, after, comparison = _load_final_delivery_inputs(
            report_input.plan_comparison_ref,
            ctx,
        )
        bundle = build_network_planning_report_bundle(
            normalized,
            before,
            after,
            comparison,
            country_code=prepared.country_code,
        )
        markdown = render_network_planning_report_markdown(bundle)
    return _write_final_delivery_markdown(
        markdown,
        output_relative_path,
        ctx,
        "已生成仓网分析中文 Markdown 简报。",
    )


def register_tools(mcp, *, phase: str = "all") -> None:
    if phase in ("all", "distribution"):
        mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(prepare_network_distribution_map)
    if phase in ("all", "final"):
        mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(prepare_network_comparison_map)
    if phase in ("all", "final"):
        mcp.tool(structured_output=True, annotations=CONTENT_ADDRESSED_RESOURCE_TOOL)(prepare_network_coverage_map)
    if phase in ("all", "final"):
        mcp.tool(structured_output=True, annotations=FINAL_WORKSPACE_DELIVERY_TOOL)(publish_network_planning_report)
