from __future__ import annotations

import asyncio
import inspect
import json
from pathlib import Path
from types import SimpleNamespace

import pytest
from _network_fixtures import network_case
from open_web_codex_provider import ProviderContractError, ResourceRef, ResourceStore
from supply_chain_planner.data.mapping import SourceRole
from supply_chain_planner.data.workspace_intake import source_content_sha256
from supply_chain_planner.network import analysis_tools, route_tools, server, tool_runtime
from supply_chain_planner.network.matrix import RouteMatrix as ComposableRouteMatrix
from supply_chain_planner.network.matrix_models import (
    AllWarehousesScope,
    ExistingOnlyWarehouseScope,
    NavigationMatrixResult,
    NavigationRouteMatrixStats,
    RouteMatrixRow,
)
from supply_chain_planner.network.models import CurrentAssignmentRecord
from supply_chain_planner.network.optimization_models import BaselineResult
from supply_chain_planner.shared import planning_input
from supply_chain_planner.shared.models import (
    PreparationRoleCounts,
    PreparedNetworkResource,
    PreparedSourceSelection,
)
from supply_chain_planner.shared.planning_input import (
    derive_selected_source_identity,
    load_prepared_network_input,
    validate_prepared_freshness,
)
from supply_chain_planner.shared.resources import SupplyChainResources


def _runtime(tmp_path: Path, monkeypatch) -> tuple[Path, ResourceStore]:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    resources = SupplyChainResources(workspace, tmp_path / "profile")
    monkeypatch.setattr(tool_runtime, "_supply_chain_resources", resources)
    return workspace, resources.store


def _context(workspace: Path) -> SimpleNamespace:
    meta = SimpleNamespace(
        model_extra={"codex/sandbox-state-meta": {"sandboxCwd": workspace.as_uri()}}
    )
    return SimpleNamespace(request_context=SimpleNamespace(meta=meta))


def _write_prepared_input(
    workspace: Path,
    relative_path: str,
    *,
    city_name: str = "City A",
) -> str:
    fixture = network_case()
    demand = [
        item.model_copy(update={"city_name": city_name}) if item.city_id == "city-a" else item
        for item in fixture.demand
    ]
    if Path(relative_path).parent == Path("."):
        relative_path = f"outputs/warehouse-network/prepared/{Path(relative_path).name}"
    (workspace / "fixture").mkdir(exist_ok=True)
    (workspace / "fixture/demand.csv").write_text("city_id,city_name,demand_quantity\n", encoding="utf-8")
    (workspace / "fixture/warehouses.csv").write_text("warehouse_id,warehouse_name,warehouse_type,city_id,city_name\n", encoding="utf-8")
    source_selections = [
        PreparedSourceSelection(
            relative_path="fixture/demand.csv",
            unit_ref="table",
            role="demand",
            mappings=[
                {"source_field": "city_id", "target_field": "city_id", "transform": "trim"},
                {"source_field": "city_name", "target_field": "city_name", "transform": "trim"},
                {"source_field": "demand_quantity", "target_field": "demand_quantity", "transform": "parse_integer"},
            ],
            raw_content_sha256=source_content_sha256(workspace, "fixture/demand.csv"),
        ),
        PreparedSourceSelection(
            relative_path="fixture/warehouses.csv",
            unit_ref="table",
            role="candidate_warehouse",
            mappings=[
                {"source_field": "city_id", "target_field": "city_id", "transform": "trim"},
                {"source_field": "city_name", "target_field": "city_name", "transform": "trim"},
                {"source_field": "warehouse_id", "target_field": "warehouse_id", "transform": "trim"},
                {"source_field": "warehouse_name", "target_field": "warehouse_name", "transform": "trim"},
                {"source_field": "warehouse_type", "target_field": "warehouse_type", "transform": "normalize_warehouse_type"},
            ],
            raw_content_sha256=source_content_sha256(workspace, "fixture/warehouses.csv"),
        ),
        PreparedSourceSelection(
            relative_path="fixture/warehouses.csv",
            unit_ref="table",
            role="existing_warehouse",
            mappings=[
                {"source_field": "city_id", "target_field": "city_id", "transform": "trim"},
                {"source_field": "city_name", "target_field": "city_name", "transform": "trim"},
                {"source_field": "warehouse_id", "target_field": "warehouse_id", "transform": "trim"},
                {"source_field": "warehouse_name", "target_field": "warehouse_name", "transform": "trim"},
                {"source_field": "warehouse_type", "target_field": "warehouse_type", "transform": "normalize_warehouse_type"},
            ],
            raw_content_sha256=source_content_sha256(workspace, "fixture/warehouses.csv"),
        ),
    ]
    role_counts = PreparationRoleCounts(
        demand=len(demand),
        existing_warehouse=sum(item.is_existing for item in fixture.warehouses),
        candidate_warehouse=sum(not item.is_existing for item in fixture.warehouses),
        current_assignment=2,
        route_quote=0,
        provided_route_fact=0,
    )
    prepared = PreparedNetworkResource(
        country_code="ID",
        state="ready",
        demand_cities=demand,
        warehouses=fixture.warehouses,
        current_assignments=[
            CurrentAssignmentRecord(
                demand_city_id="city-a",
                serving_warehouse_id="center-a",
            ),
            CurrentAssignmentRecord(
                demand_city_id="city-b",
                serving_warehouse_id="cross-b",
                upstream_center_id="center-a",
            ),
        ],
        route_quotes=[],
        issue_count=0,
        issues=[],
        issues_truncated=False,
        roles=["candidate_warehouse", "demand", "existing_warehouse"],
        source_selections=source_selections,
        selected_source_identity=derive_selected_source_identity("ID", source_selections, None),
        role_counts=role_counts,
    )
    target = workspace / relative_path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(
        prepared.model_dump_json(by_alias=True),
        encoding="utf-8",
    )
    return relative_path


def _result_ref(result) -> ResourceRef:
    assert result.structuredContent is not None
    return ResourceRef.model_validate(result.structuredContent["resource_ref"])


def test_network_tools_accept_only_prepared_workspace_inputs() -> None:
    tools = {tool.name: tool for tool in asyncio.run(server.mcp.list_tools())}
    for tool in tools.values():
        assert "normalized_input_ref" not in tool.inputSchema["properties"]
    for name in (
        "evaluate_baseline",
        "evaluate_scenario",
        "find_sites_for_service_target",
        "create_navigation_matrix_request",
        "prepare_network_distribution_map",
    ):
        assert "prepared_input_relative_path" in tools[name].inputSchema["properties"]

    route = inspect.signature(route_tools.prepare_route_matrix).parameters
    assert "prior_route_matrix_ref" in route


def test_network_rejects_matrix_from_another_prepared_workspace_input(
    tmp_path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    first_path = _write_prepared_input(workspace, "prepared-a.json")
    second_path = _write_prepared_input(workspace, "prepared-b.json", city_name="Renamed City")

    routes = route_tools.prepare_route_matrix(
        first_path,
        "haversine",
        ctx,
        warehouse_scope=ExistingOnlyWarehouseScope(),
        detour_coefficient=1.2,
        average_speed_kph=40,
    )
    route_ref = _result_ref(routes)
    stored_routes = tool_runtime._runtime().load_model(
        route_ref, "route_matrix.v3", ComposableRouteMatrix
    )
    _prepared, expected_identity = load_prepared_network_input(workspace, first_path)
    assert stored_routes.input_identity == expected_identity

    with pytest.raises(ProviderContractError, match="baseline_input_identity_mismatch"):
        analysis_tools.evaluate_network_baseline(
            second_path,
            route_ref,
            "min_time",
            [12],
            ctx,
            coverage_mode="optimized_existing_footprint",
        )


def test_network_load_uses_self_contained_prepared_snapshot_when_raw_sources_are_gone(
    tmp_path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _write_prepared_input(workspace, "prepared-snapshot.json")
    (workspace / "fixture/demand.csv").unlink()
    (workspace / "fixture/warehouses.csv").unlink()

    prepared, identity = load_prepared_network_input(workspace, prepared_path)

    assert prepared.state == "ready"
    assert identity.content_sha256
    routes = route_tools.prepare_route_matrix(
        prepared_path,
        "haversine",
        ctx,
        warehouse_scope=ExistingOnlyWarehouseScope(),
        detour_coefficient=1.2,
        average_speed_kph=40,
    )
    assert routes.structuredContent is not None


@pytest.mark.parametrize("relative_path", ["/tmp/secret.csv", "../secret.csv"])
def test_network_loader_rejects_malicious_prepared_provenance_path(
    tmp_path, monkeypatch, relative_path: str
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    prepared_path = _write_prepared_input(workspace, "prepared-malicious.json")
    payload = json.loads((workspace / prepared_path).read_text(encoding="utf-8"))
    payload["source_selections"][0]["relative_path"] = relative_path
    selections = [PreparedSourceSelection.model_validate(item) for item in payload["source_selections"]]
    payload["selected_source_identity"] = derive_selected_source_identity(
        payload["country_code"], selections, None
    )
    (workspace / prepared_path).write_text(json.dumps(payload), encoding="utf-8")

    with pytest.raises(ValueError, match="prepared_provenance_path_invalid"):
        load_prepared_network_input(workspace, prepared_path)


def test_prepared_freshness_hashes_and_inspects_each_unique_raw_file_once(
    tmp_path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    prepared_path = _write_prepared_input(workspace, "prepared-fresh.json")
    prepared, _identity = load_prepared_network_input(workspace, prepared_path)
    original_hash = planning_input.source_content_sha256
    original_inspect = planning_input.inspect
    calls = {"hash": [], "inspect": []}

    def counted_hash(root, relative_path):
        calls["hash"].append(relative_path)
        return original_hash(root, relative_path)

    def counted_inspect(root, relative_path):
        calls["inspect"].append(relative_path)
        return original_inspect(root, relative_path)

    monkeypatch.setattr(planning_input, "source_content_sha256", counted_hash)
    monkeypatch.setattr(planning_input, "inspect", counted_inspect)
    fresh, reason = validate_prepared_freshness(
        workspace,
        prepared,
        required_roles=[SourceRole.DEMAND, SourceRole.EXISTING_WAREHOUSE],
        country_code="ID",
    )

    assert fresh is True
    assert reason == "fresh"
    assert calls["inspect"] == ["fixture/demand.csv", "fixture/warehouses.csv"]
    assert calls["hash"] == [
        "fixture/demand.csv",
        "fixture/demand.csv",
        "fixture/warehouses.csv",
        "fixture/warehouses.csv",
    ]


def test_prepared_freshness_rejects_file_changed_between_hashes(
    tmp_path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    prepared_path = _write_prepared_input(workspace, "prepared-race.json")
    prepared, _identity = load_prepared_network_input(workspace, prepared_path)
    original_hash = planning_input.source_content_sha256
    demand_hash_calls = 0

    def changing_hash(root, relative_path):
        nonlocal demand_hash_calls
        value = original_hash(root, relative_path)
        if relative_path == "fixture/demand.csv":
            demand_hash_calls += 1
            if demand_hash_calls == 2:
                return "f" * 64
        return value

    monkeypatch.setattr(planning_input, "source_content_sha256", changing_hash)
    fresh, reason = validate_prepared_freshness(
        workspace,
        prepared,
        required_roles=[SourceRole.DEMAND],
        country_code="ID",
    )

    assert fresh is False
    assert reason == "source_changed"
    assert demand_hash_calls == 2


def test_baseline_auto_uses_optimized_existing_when_assignments_are_absent(
    tmp_path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _write_prepared_input(workspace, "prepared-auto.json")
    payload = json.loads((workspace / prepared_path).read_text(encoding="utf-8"))
    payload["current_assignments"] = []
    payload["role_counts"]["current_assignment"] = 0
    (workspace / prepared_path).write_text(json.dumps(payload), encoding="utf-8")
    routes = route_tools.prepare_route_matrix(
        prepared_path,
        "haversine",
        ctx,
        warehouse_scope=ExistingOnlyWarehouseScope(),
        detour_coefficient=1.2,
        average_speed_kph=40,
    )
    baseline = analysis_tools.evaluate_network_baseline(
        prepared_path,
        _result_ref(routes),
        "min_time",
        [12],
        ctx,
    )
    stored = tool_runtime._runtime().load_model(
        _result_ref(baseline), "network_baseline.v2", BaselineResult
    )
    assert stored.label == "optimized_existing_footprint"


def test_workspace_input_mutation_invalidates_existing_matrix(tmp_path, monkeypatch) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _write_prepared_input(workspace, "prepared.json")
    routes = route_tools.prepare_route_matrix(
        prepared_path,
        "haversine",
        ctx,
        warehouse_scope=ExistingOnlyWarehouseScope(),
        detour_coefficient=1.2,
        average_speed_kph=40,
    )
    route_ref = _result_ref(routes)

    payload = (workspace / prepared_path).read_text(encoding="utf-8")
    (workspace / prepared_path).write_text(
        payload.replace("City A", "Changed City", 1), encoding="utf-8"
    )
    with pytest.raises(ProviderContractError, match="baseline_input_identity_mismatch"):
        analysis_tools.evaluate_network_baseline(
            prepared_path,
            route_ref,
            "min_time",
            [12],
            ctx,
            coverage_mode="optimized_existing_footprint",
        )


def test_navigation_request_and_import_use_exact_workspace_contract(tmp_path, monkeypatch) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _write_prepared_input(workspace, "prepared.json")
    request_result = route_tools.create_navigation_matrix_request(
        prepared_path,
        ExistingOnlyWarehouseScope(),
        "outputs/warehouse-network/requests/navigation-request.json",
        ctx,
    )
    request_payload = json.loads(
        (workspace / request_result.navigation_request_relative_path).read_text()
    )
    assert request_payload["schema_version"] == "navigation_matrix_request.v2"
    assert request_payload["warehouse_scope"] == {"kind": "existing_only"}
    assert request_payload["warehouse_ids"] == sorted(
        item["warehouse_id"] for item in json.loads((workspace / prepared_path).read_text())["warehouses"] if item["is_existing"]
    )
    result = NavigationMatrixResult(
        input_identity=request_result.input_identity,
        warehouse_scope=ExistingOnlyWarehouseScope(),
        warehouse_ids=request_payload["warehouse_ids"],
        rows=[
            RouteMatrixRow(
                origin_id=item["origin_id"],
                destination_id=item["destination_id"],
                layer=item["layer"],
                distance_km=1,
                duration_hours=0.1,
                method="navigation",
                tool_version="test-navigation.v1",
                origin_longitude=item["origin_longitude"],
                origin_latitude=item["origin_latitude"],
                destination_longitude=item["destination_longitude"],
                destination_latitude=item["destination_latitude"],
                navigation_provider="test-provider",
                navigation_profile="driving",
            )
            for item in request_payload["routes"]
        ],
    )
    (workspace / "navigation-result.json").write_text(result.model_dump_json(), encoding="utf-8")
    imported = route_tools.import_navigation_matrix(
        prepared_path,
        "navigation-result.json",
        ctx,
    )
    route_ref = _result_ref(imported)
    route_matrix = tool_runtime._runtime().load_model(
        route_ref, "route_matrix.v3", ComposableRouteMatrix
    )
    assert route_matrix.method == "navigation"
    assert route_matrix.input_identity == request_result.input_identity

    partial_prior = route_matrix.model_copy(
        update={
            "rows": [
                route_matrix.rows[0],
                route_matrix.rows[1].model_copy(update={"status": "error"}),
            ],
            "stats": route_matrix.stats.model_copy(
                update={"reused_pair_count": 1, "registered_pair_count": 1, "complete": False}
            ),
        }
    )
    partial_prior_ref = _result_ref(
        tool_runtime._runtime().publish(partial_prior.schema_version, partial_prior, "partial prior")
    )
    request_again = route_tools.create_navigation_matrix_request(
        prepared_path,
        ExistingOnlyWarehouseScope(),
        "outputs/warehouse-network/requests/navigation-request-again.json",
        ctx,
        prior_route_matrix_ref=partial_prior_ref,
    )
    request_again_payload = json.loads(
        (workspace / request_again.navigation_request_relative_path).read_text()
    )
    supplied_again = NavigationMatrixResult(
        input_identity=request_again.input_identity,
        warehouse_scope=ExistingOnlyWarehouseScope(),
        warehouse_ids=request_again_payload["warehouse_ids"],
        rows=[
            RouteMatrixRow(
                origin_id=item["origin_id"],
                destination_id=item["destination_id"],
                layer=item["layer"],
                distance_km=1,
                duration_hours=0.1,
                method="navigation",
                tool_version="test-navigation.v1",
                origin_longitude=item["origin_longitude"],
                origin_latitude=item["origin_latitude"],
                destination_longitude=item["destination_longitude"],
                destination_latitude=item["destination_latitude"],
                navigation_provider="test-provider",
                navigation_profile="driving",
            )
            for item in request_again_payload["routes"]
        ],
    )
    (workspace / "navigation-result-again.json").write_text(
        supplied_again.model_dump_json(), encoding="utf-8"
    )
    imported_again = route_tools.import_navigation_matrix(
        prepared_path,
        "navigation-result-again.json",
        ctx,
        prior_route_matrix_ref=partial_prior_ref,
    )
    imported_again_matrix = tool_runtime._runtime().load_model(
        _result_ref(imported_again), "route_matrix.v3", ComposableRouteMatrix
    )
    assert imported_again_matrix.stats.reused_pair_count == 1
    assert imported_again_matrix.stats.registered_pair_count == len(request_again_payload["routes"])


def test_navigation_prior_requires_navigation_identity_and_warehouse_set(
    tmp_path, monkeypatch
) -> None:
    workspace, _store = _runtime(tmp_path, monkeypatch)
    ctx = _context(workspace)
    prepared_path = _write_prepared_input(workspace, "prepared.json")
    prior = _result_ref(
        route_tools.prepare_route_matrix(
            prepared_path,
            "haversine",
            ctx,
            warehouse_scope=AllWarehousesScope(),
            detour_coefficient=1.2,
            average_speed_kph=42,
        )
    )
    with pytest.raises(ProviderContractError, match="navigation_prior_method_mismatch"):
        route_tools.create_navigation_matrix_request(
            prepared_path,
            AllWarehousesScope(),
            "outputs/warehouse-network/requests/navigation-prior-method.json",
            ctx,
            prior_route_matrix_ref=prior,
        )
    prior_matrix = tool_runtime._runtime().load_model(prior, "route_matrix.v3", ComposableRouteMatrix)

    def publish_prior(value: ComposableRouteMatrix) -> ResourceRef:
        return _result_ref(tool_runtime._runtime().publish(value.schema_version, value, "test prior"))

    identity_mismatch = publish_prior(
        prior_matrix.model_copy(
            update={
                "input_identity": prior_matrix.input_identity.model_copy(
                    update={"content_sha256": "e" * 64}
                )
            }
        )
    )
    with pytest.raises(ProviderContractError, match="navigation_prior_input_identity_mismatch"):
        route_tools.create_navigation_matrix_request(
            prepared_path,
            AllWarehousesScope(),
            "outputs/warehouse-network/requests/navigation-prior-identity.json",
            ctx,
            prior_route_matrix_ref=identity_mismatch,
        )
    warehouse_set_mismatch = publish_prior(
        prior_matrix.model_copy(
            update={
                "method": "navigation",
                "stats": NavigationRouteMatrixStats(
                    route_count=len(prior_matrix.rows),
                    reused_pair_count=0,
                    registered_pair_count=len(prior_matrix.rows),
                    missing_pair_count=0,
                    complete=True,
                ),
                "warehouse_ids": prior_matrix.warehouse_ids[:-1],
            }
        )
    )
    with pytest.raises(ProviderContractError, match="navigation_route_matrix_warehouse_set_mismatch"):
        route_tools.create_navigation_matrix_request(
            prepared_path,
            AllWarehousesScope(),
            "outputs/warehouse-network/requests/navigation-prior-set.json",
            ctx,
            prior_route_matrix_ref=warehouse_set_mismatch,
        )
