"""Deterministic route and cost matrix builders for composable cases."""

from __future__ import annotations

import math
from collections.abc import Sequence
from decimal import Decimal
from typing import Literal

from supply_chain_planner.network.geo import haversine_km
from supply_chain_planner.network.matrix_models import (
    AllWarehousesScope,
    CostCalculationPolicy,
    CostMatrix,
    CostMatrixRow,
    CostMatrixStats,
    DemandUnitCostRule,
    ExistingPlusCandidatesWarehouseScope,
    HaversineRouteMatrixStats,
    NavigationMatrixRequest,
    NavigationRouteMatrixStats,
    NavigationRouteRequest,
    NetworkLayer,
    ObservedQuoteMeanCostEvidence,
    ProvidedRouteMatrixStats,
    QuoteMeanCostRuleEvidence,
    RouteCostQuote,
    RouteFactProvenance,
    RouteMatrix,
    RouteMatrixPlan,
    RouteMatrixRow,
    RouteMatrixValidation,
    WarehouseScope,
)
from supply_chain_planner.network.models import (
    DemandCityRecord,
    PlanningInputIdentity,
    ProvidedRouteFactRecord,
    RouteQuoteRecord,
    WarehouseRecord,
)

HAVERSINE_TOOL_VERSION = "haversine.v1"
QUOTE_UNIT_COST_TOOL_VERSION = "quote-unit-cost.v1"
DISTANCE_UNIT_COST_TOOL_VERSION = "distance-unit-cost.v1"
OBSERVED_QUOTE_MEAN_TOOL_VERSION = "observed-quote-mean.v1"
PROVIDED_INPUT_TOOL_VERSION = "provided-input.v1"


def resolve_warehouse_scope(
    warehouses: list[WarehouseRecord],
    scope: WarehouseScope,
) -> list[WarehouseRecord]:
    """Resolve one typed scope against the complete prepared warehouse set."""
    warehouse_by_id = {warehouse.warehouse_id: warehouse for warehouse in warehouses}
    if len(warehouse_by_id) != len(warehouses):
        raise ValueError("warehouse_id_duplicate")
    existing = {warehouse.warehouse_id for warehouse in warehouses if warehouse.is_existing}
    if scope.kind == "existing_only":
        selected = existing
    elif scope.kind == "all_warehouses":
        selected = set(warehouse_by_id)
    else:
        candidates = set(scope.candidate_ids)
        unknown = candidates - set(warehouse_by_id)
        if unknown:
            raise ValueError("warehouse_scope_candidate_unknown:" + ",".join(sorted(unknown)))
        existing_candidates = candidates & existing
        if existing_candidates:
            raise ValueError(
                "warehouse_scope_candidate_must_be_non_existing:"
                + ",".join(sorted(existing_candidates))
            )
        selected = existing | candidates
    if not selected:
        raise ValueError("warehouse_scope_resolves_empty")
    return [warehouse_by_id[warehouse_id] for warehouse_id in sorted(selected)]


def canonical_warehouse_scope(
    warehouses: list[WarehouseRecord],
    scope: WarehouseScope,
) -> WarehouseScope:
    """Return the single canonical scope for the warehouse set ``scope`` selects.

    An explicit candidate list that names every non-existing warehouse selects the
    same set as ``all_warehouses``; matrices record the canonical form so every
    downstream scope comparison is a comparison of warehouse sets.
    """
    resolve_warehouse_scope(warehouses, scope)
    if not isinstance(scope, ExistingPlusCandidatesWarehouseScope):
        return scope
    all_candidate_ids = {
        warehouse.warehouse_id for warehouse in warehouses if not warehouse.is_existing
    }
    if set(scope.candidate_ids) == all_candidate_ids:
        return AllWarehousesScope()
    return scope


def plan_route_matrix(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    method: Literal["haversine", "navigation", "provided"],
    detour_coefficient: float | None,
    average_speed_kph: float | None,
    *,
    warehouse_scope: WarehouseScope,
    input_identity: PlanningInputIdentity,
) -> RouteMatrixPlan:
    if method == "haversine" and (detour_coefficient is None or average_speed_kph is None):
        raise ValueError("haversine_requires_detour_coefficient_and_average_speed")
    scoped_warehouses = resolve_warehouse_scope(warehouses, warehouse_scope)
    expected = _expected_route_pairs(demand_cities, scoped_warehouses)
    return RouteMatrixPlan(
        origin_count=len({origin for origin, _, _ in expected}),
        destination_count=len({destination for _, destination, _ in expected}),
        route_count=len(expected),
        method=method,
        detour_coefficient=detour_coefficient,
        average_speed_kph=average_speed_kph,
        estimated_billable_calls=(len(expected) if method == "navigation" else 0),
        warehouse_scope=warehouse_scope,
        warehouse_ids=[warehouse.warehouse_id for warehouse in scoped_warehouses],
        input_identity=input_identity,
    )


def build_navigation_matrix_request(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    *,
    warehouse_scope: WarehouseScope,
    input_identity: PlanningInputIdentity,
) -> NavigationMatrixRequest:
    """Materialize exactly the lanes that one navigation provider may bill for."""
    warehouses = resolve_warehouse_scope(warehouses, warehouse_scope)
    warehouse_by_id = {warehouse.warehouse_id: warehouse for warehouse in warehouses}
    demand_by_id = {demand.city_id: demand for demand in demand_cities}
    routes: list[NavigationRouteRequest] = []
    for origin_id, destination_id, layer in _expected_route_pairs(demand_cities, warehouses):
        origin = warehouse_by_id[origin_id]
        destination = (
            demand_by_id[destination_id]
            if layer == "last_mile"
            else warehouse_by_id[destination_id]
        )
        if (
            origin.longitude is None
            or origin.latitude is None
            or destination.longitude is None
            or destination.latitude is None
        ):
            raise ValueError("navigation_request_coordinates_missing")
        routes.append(
            NavigationRouteRequest(
                origin_id=origin_id,
                destination_id=destination_id,
                layer=layer,
                origin_longitude=origin.longitude,
                origin_latitude=origin.latitude,
                destination_longitude=destination.longitude,
                destination_latitude=destination.latitude,
            )
        )
    return NavigationMatrixRequest(
        input_identity=input_identity,
        warehouse_scope=warehouse_scope,
        warehouse_ids=[warehouse.warehouse_id for warehouse in warehouses],
        routes=routes,
        estimated_billable_elements=len(routes),
    )


def build_haversine_route_matrix(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    detour_coefficient: float,
    average_speed_kph: float,
    *,
    warehouse_scope: WarehouseScope = AllWarehousesScope(),
    input_identity: PlanningInputIdentity,
) -> RouteMatrix:
    return build_route_matrix_with_reuse(
        demand_cities,
        warehouses,
        [],
        detour_coefficient,
        average_speed_kph,
        warehouse_scope=warehouse_scope,
        input_identity=input_identity,
    )


def build_provided_route_matrix(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    provided_route_facts: list[ProvidedRouteFactRecord],
    *,
    warehouse_scope: WarehouseScope,
    input_identity: PlanningInputIdentity,
) -> RouteMatrix:
    """Materialize exact uploaded distance/duration facts for expected route pairs."""

    warehouses = resolve_warehouse_scope(warehouses, warehouse_scope)
    expected = _expected_route_pairs(demand_cities, warehouses)
    expected_set = set(expected)
    supplied: dict[tuple[str, str, NetworkLayer], ProvidedRouteFactRecord] = {}
    ignored = 0
    source_methods: dict[str, int] = {}
    for fact in provided_route_facts:
        key = (fact.origin_id, fact.destination_id, fact.layer)
        if key not in expected_set:
            ignored += 1
            continue
        if key in supplied:
            raise ValueError(
                f"provided_route_fact_duplicate_pair:{fact.origin_id}:"
                f"{fact.destination_id}:{fact.layer}"
            )
        supplied[key] = fact
        source_methods[fact.source_method] = source_methods.get(fact.source_method, 0) + 1
    missing = sorted(expected_set - set(supplied))
    rows = [
        RouteMatrixRow(
            origin_id=fact.origin_id,
            destination_id=fact.destination_id,
            layer=fact.layer,
            distance_km=fact.distance_km,
            duration_hours=fact.duration_hours,
            method="provided",
            tool_version=PROVIDED_INPUT_TOOL_VERSION,
        )
        for key in expected
        if (fact := supplied.get(key)) is not None
    ]
    return RouteMatrix(
        method="provided",
        warehouse_scope=warehouse_scope,
        warehouse_ids=[warehouse.warehouse_id for warehouse in warehouses],
        rows=rows,
        missing_routes=missing,
        stats=ProvidedRouteMatrixStats(
            expected_pair_count=len(expected),
            provided_pair_count=len(rows),
            ignored_input_pair_count=ignored,
            missing_pair_count=len(missing),
            complete=not missing,
            source_method_counts=dict(sorted(source_methods.items())),
        ),
        input_identity=input_identity,
    )


def build_route_matrix_with_reuse(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    existing_rows: list[RouteMatrixRow],
    detour_coefficient: float,
    average_speed_kph: float,
    *,
    warehouse_scope: WarehouseScope,
    input_identity: PlanningInputIdentity,
    provided_route_facts: Sequence[ProvidedRouteFactRecord] = (),
) -> RouteMatrix:
    """Keep uploaded route facts, reuse exact estimates and compute only the rest.

    Haversine completes the gaps of the provided facts: an expected pair with an
    uploaded fact keeps that fact, so choosing an estimate never overwrites
    route data the user supplied.
    """

    if detour_coefficient <= 0 or average_speed_kph <= 0:
        raise ValueError("route_parameters_must_be_positive")
    warehouses = resolve_warehouse_scope(warehouses, warehouse_scope)
    expected = _expected_route_pairs(demand_cities, warehouses)
    expected_set = set(expected)
    provided_index: dict[tuple[str, str, NetworkLayer], ProvidedRouteFactRecord] = {}
    for fact in provided_route_facts:
        key = (fact.origin_id, fact.destination_id, fact.layer)
        if key not in expected_set:
            continue
        if key in provided_index:
            raise ValueError(
                f"provided_route_fact_duplicate_pair:{fact.origin_id}:"
                f"{fact.destination_id}:{fact.layer}"
            )
        provided_index[key] = fact
    existing_index: dict[tuple[str, str, NetworkLayer], list[RouteMatrixRow]] = {}
    ignored_prior_rows = 0
    for row in existing_rows:
        key = (row.origin_id, row.destination_id, row.layer)
        if key not in expected_set:
            ignored_prior_rows += 1
            continue
        existing_index.setdefault(key, []).append(row)
    warehouse_by_id = {warehouse.warehouse_id: warehouse for warehouse in warehouses}
    demand_by_id = {demand.city_id: demand for demand in demand_cities}
    rows: list[RouteMatrixRow] = []
    missing: list[tuple[str, str, NetworkLayer]] = []
    reused = 0
    computed = 0
    stale = 0
    for origin, destination, layer in expected:
        fact = provided_index.get((origin, destination, layer))
        if fact is not None:
            rows.append(
                RouteMatrixRow(
                    origin_id=origin,
                    destination_id=destination,
                    layer=layer,
                    distance_km=fact.distance_km,
                    duration_hours=fact.duration_hours,
                    method="provided",
                    tool_version=PROVIDED_INPUT_TOOL_VERSION,
                )
            )
            continue
        origin_warehouse = warehouse_by_id[origin]
        destination_point = (
            demand_by_id[destination] if layer == "last_mile" else warehouse_by_id[destination]
        )
        candidates = existing_index.get((origin, destination, layer), [])
        if len(candidates) > 1:
            raise ValueError(f"route_fact_duplicate_pair:{origin}:{destination}:{layer}")
        exact = [
            row
            for row in candidates
            if _is_exact_haversine_fact(
                row,
                origin_warehouse,
                destination_point,
                detour_coefficient,
                average_speed_kph,
            )
        ]
        if exact:
            rows.append(exact[0])
            reused += 1
            continue
        if candidates:
            stale += 1
        if (
            origin_warehouse.longitude is None
            or origin_warehouse.latitude is None
            or destination_point.longitude is None
            or destination_point.latitude is None
        ):
            missing.append((origin, destination, layer))
            continue
        distance = (
            haversine_km(
                (origin_warehouse.longitude, origin_warehouse.latitude),
                (destination_point.longitude, destination_point.latitude),
            )
            * detour_coefficient
        )
        rows.append(
            RouteMatrixRow(
                origin_id=origin,
                destination_id=destination,
                layer=layer,
                distance_km=round(distance, 6),
                duration_hours=round(distance / average_speed_kph, 6),
                method="haversine",
                tool_version=HAVERSINE_TOOL_VERSION,
                origin_longitude=origin_warehouse.longitude,
                origin_latitude=origin_warehouse.latitude,
                destination_longitude=destination_point.longitude,
                destination_latitude=destination_point.latitude,
                detour_coefficient=detour_coefficient,
                average_speed_kph=average_speed_kph,
            )
        )
        computed += 1
    return RouteMatrix(
        method="haversine",
        warehouse_scope=warehouse_scope,
        warehouse_ids=[warehouse.warehouse_id for warehouse in warehouses],
        rows=rows,
        missing_routes=missing,
        stats=HaversineRouteMatrixStats(
            expected_pair_count=len(expected),
            provided_pair_count=len(provided_index),
            reused_pair_count=reused,
            computed_pair_count=computed,
            stale_pair_count=stale,
            ignored_prior_row_count=ignored_prior_rows,
            missing_pair_count=len(missing),
            last_mile_pair_count=sum(1 for _, _, layer in expected if layer == "last_mile"),
            linehaul_pair_count=sum(1 for _, _, layer in expected if layer == "linehaul"),
            detour_coefficient=detour_coefficient,
            average_speed_kph=average_speed_kph,
            complete=not missing,
        ),
        input_identity=input_identity,
    )


def _is_exact_haversine_fact(
    row: RouteMatrixRow,
    origin: WarehouseRecord,
    destination: DemandCityRecord | WarehouseRecord,
    detour_coefficient: float,
    average_speed_kph: float,
) -> bool:
    if (
        origin.longitude is None
        or origin.latitude is None
        or destination.longitude is None
        or destination.latitude is None
    ):
        return False
    raw_distance = (
        haversine_km(
            (origin.longitude, origin.latitude),
            (destination.longitude, destination.latitude),
        )
        * detour_coefficient
    )
    expected_distance = round(raw_distance, 6)
    expected_duration = round(raw_distance / average_speed_kph, 6)
    expected_values = (
        origin.longitude,
        origin.latitude,
        destination.longitude,
        destination.latitude,
        detour_coefficient,
        average_speed_kph,
    )
    actual_values = (
        row.origin_longitude,
        row.origin_latitude,
        row.destination_longitude,
        row.destination_latitude,
        row.detour_coefficient,
        row.average_speed_kph,
    )
    return (
        row.method == "haversine"
        and row.tool_version == HAVERSINE_TOOL_VERSION
        and row.status == "ready"
        and all(value is not None for value in expected_values)
        and all(value is not None for value in actual_values)
        and math.isclose(row.distance_km, expected_distance, rel_tol=0, abs_tol=1e-6)
        and math.isclose(
            row.duration_hours,
            expected_duration,
            rel_tol=0,
            abs_tol=1e-6,
        )
        and all(
            math.isclose(float(actual), float(expected), rel_tol=0, abs_tol=1e-9)
            for actual, expected in zip(actual_values, expected_values, strict=True)
        )
    )


def _expected_route_pairs(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
) -> list[tuple[str, str, NetworkLayer]]:
    if not demand_cities or not warehouses:
        raise ValueError("route_matrix_requires_demand_and_warehouses")
    warehouse_by_id = {warehouse.warehouse_id: warehouse for warehouse in warehouses}
    if len(warehouse_by_id) != len(warehouses):
        raise ValueError("warehouse_id_duplicate")
    pairs: set[tuple[str, str, NetworkLayer]] = {
        (warehouse.warehouse_id, demand.city_id, "last_mile")
        for warehouse in warehouses
        for demand in demand_cities
    }
    for warehouse in warehouses:
        if warehouse.warehouse_type != "cross_docking":
            continue
        if not warehouse.upstream_center_id:
            raise ValueError(f"warehouse_upstream_center_required:{warehouse.warehouse_id}")
        upstream = warehouse_by_id.get(warehouse.upstream_center_id)
        if upstream is None:
            raise ValueError(f"warehouse_upstream_center_missing:{warehouse.warehouse_id}")
        if upstream.warehouse_type != "center":
            raise ValueError(f"warehouse_upstream_must_be_center:{warehouse.warehouse_id}")
        pairs.add((upstream.warehouse_id, warehouse.warehouse_id, "linehaul"))
    return sorted(pairs)


def register_navigation_route_matrix(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    rows: list[RouteMatrixRow],
    *,
    warehouse_scope: WarehouseScope,
    input_identity: PlanningInputIdentity,
) -> RouteMatrix:
    warehouses = resolve_warehouse_scope(warehouses, warehouse_scope)
    expected = set(_expected_route_pairs(demand_cities, warehouses))
    warehouse_by_id = {warehouse.warehouse_id: warehouse for warehouse in warehouses}
    demand_by_id = {demand.city_id: demand for demand in demand_cities}
    supplied: set[tuple[str, str, NetworkLayer]] = set()
    provenance: set[tuple[str, str, str]] = set()
    for row in rows:
        key = (row.origin_id, row.destination_id, row.layer)
        if key not in expected:
            raise ValueError(
                f"navigation_route_unknown_pair:{row.origin_id}:{row.destination_id}:{row.layer}"
            )
        if key in supplied:
            raise ValueError(
                f"navigation_route_duplicate_pair:{row.origin_id}:{row.destination_id}:{row.layer}"
            )
        supplied.add(key)
        if row.method != "navigation":
            raise ValueError("navigation_matrix_requires_navigation_method")
        if not row.navigation_provider or not row.navigation_profile:
            raise ValueError("navigation_parameters_unavailable")
        provenance.add((row.navigation_provider, row.navigation_profile, row.tool_version))
        origin = warehouse_by_id.get(row.origin_id)
        destination = (
            demand_by_id.get(row.destination_id)
            if row.layer == "last_mile"
            else warehouse_by_id.get(row.destination_id)
        )
        if (
            origin is None
            or destination is None
            or not _route_endpoints_match(row, origin, destination)
        ):
            raise ValueError(
                f"navigation_route_endpoint_mismatch:{row.origin_id}:{row.destination_id}:{row.layer}"
            )
    if len(provenance) > 1:
        raise ValueError("navigation_route_provenance_conflict")
    missing = sorted(expected - supplied)
    return RouteMatrix(
        method="navigation",
        warehouse_scope=warehouse_scope,
        warehouse_ids=[warehouse.warehouse_id for warehouse in warehouses],
        rows=rows,
        missing_routes=missing,
        stats=NavigationRouteMatrixStats(
            route_count=len(rows),
            reused_pair_count=0,
            registered_pair_count=len(rows),
            missing_pair_count=len(missing),
            complete=not missing,
        ),
        input_identity=input_identity,
    )


def _route_endpoints_match(
    row: RouteMatrixRow,
    origin: WarehouseRecord,
    destination: DemandCityRecord | WarehouseRecord,
) -> bool:
    actual = (
        row.origin_longitude,
        row.origin_latitude,
        row.destination_longitude,
        row.destination_latitude,
    )
    expected = (
        origin.longitude,
        origin.latitude,
        destination.longitude,
        destination.latitude,
    )
    return (
        all(value is not None for value in actual)
        and all(value is not None for value in expected)
        and all(
            math.isclose(float(left), float(right), rel_tol=0, abs_tol=1e-9)
            for left, right in zip(actual, expected, strict=True)
        )
    )


def validate_route_matrix(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    matrix: RouteMatrix,
) -> RouteMatrixValidation:
    errors: list[str] = []
    try:
        scoped_warehouses = resolve_warehouse_scope(warehouses, matrix.warehouse_scope)
    except ValueError as error:
        return RouteMatrixValidation(
            valid=False,
            errors=[str(error)],
            missing_routes=[],
            route_count=len(matrix.rows),
        )
    expected = set(_expected_route_pairs(demand_cities, scoped_warehouses))
    expected_warehouse_ids = [warehouse.warehouse_id for warehouse in scoped_warehouses]
    if matrix.warehouse_ids != expected_warehouse_ids:
        errors.append("warehouse_ids_mismatch")
    supplied = {(row.origin_id, row.destination_id, row.layer) for row in matrix.rows}
    unknown = sorted(supplied - expected)
    duplicate_count = len(matrix.rows) - len(supplied)
    missing = sorted(expected - supplied)
    if unknown:
        errors.append(f"unknown_routes:{len(unknown)}")
    if duplicate_count:
        errors.append(f"duplicate_routes:{duplicate_count}")
    return RouteMatrixValidation(
        valid=not errors and not missing,
        errors=errors,
        missing_routes=missing,
        route_count=len(matrix.rows),
    )


def build_cost_matrix(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    route_quotes: list[RouteCostQuote | RouteQuoteRecord],
    fallback_rule: CostCalculationPolicy | None,
    route_matrix: RouteMatrix | None = None,
    prior_rows: list[CostMatrixRow] | None = None,
    *,
    warehouse_scope: WarehouseScope,
    input_identity: PlanningInputIdentity,
    calculation_rule_source: Literal["explicit", "observed_quote_mean"] | None = None,
    calculation_rule_evidence: ObservedQuoteMeanCostEvidence | None = None,
    calculation_rule_evidence_path: str | None = None,
) -> CostMatrix:
    warehouses = resolve_warehouse_scope(warehouses, warehouse_scope)
    expected = _expected_route_pairs(demand_cities, warehouses)
    expected_set = set(expected)
    quote_index: dict[tuple[str, str, NetworkLayer], RouteCostQuote | RouteQuoteRecord] = {}
    quote_currencies: set[str] = set()
    ignored_quotes = 0
    for quote in route_quotes:
        key = (quote.origin_id, quote.destination_id, quote.layer)
        if key not in expected_set:
            ignored_quotes += 1
            continue
        if key in quote_index:
            raise ValueError(f"cost_quote_duplicate_pair:{key}")
        quote_index[key] = quote
        quote_currencies.add(str(quote.currency).upper())
    policy = None if fallback_rule is None else CostCalculationPolicy.model_validate(fallback_rule)
    rules = _cost_rules_by_layer(policy)
    rule_currencies = {rule.currency for rule in rules.values()}
    currencies = quote_currencies | rule_currencies
    if len(currencies) > 1:
        raise ValueError("cost_currency_mismatch")
    if not currencies:
        raise ValueError("cost_currency_required")
    currency = next(iter(currencies))
    if len(currency) != 3 or not currency.isalpha():
        raise ValueError("cost_currency_required")
    rows: list[CostMatrixRow] = []
    missing: list[tuple[str, str, NetworkLayer]] = []
    route_index: dict[tuple[str, str, NetworkLayer], RouteMatrixRow] = {}
    for route in route_matrix.rows if route_matrix is not None else []:
        if route.status != "ready":
            continue
        key = (route.origin_id, route.destination_id, route.layer)
        if key in route_index:
            raise ValueError(f"route_matrix_duplicate_pair:{key}")
        route_index[key] = route
    prior_index: dict[tuple[str, str, NetworkLayer], list[CostMatrixRow]] = {}
    ignored_prior_rows = 0
    for row in prior_rows or []:
        key = (row.origin_id, row.destination_id, row.layer)
        if key not in expected_set:
            ignored_prior_rows += 1
            continue
        prior_index.setdefault(key, []).append(row)
    reused = 0
    computed = 0
    stale = 0
    for origin, destination, layer in expected:
        key = (origin, destination, layer)
        quote = quote_index.get(key)
        prior_candidates = prior_index.get(key, [])
        if len(prior_candidates) > 1:
            raise ValueError(f"cost_prior_duplicate_pair:{key}")
        if quote is not None:
            if str(quote.currency).upper() != currency:
                raise ValueError("cost_currency_mismatch")
            exact = [row for row in prior_candidates if _is_exact_quoted_cost(row, quote, currency)]
            if exact:
                rows.append(exact[0])
                reused += 1
                continue
            if prior_candidates:
                stale += 1
            rows.append(
                CostMatrixRow(
                    origin_id=origin,
                    destination_id=destination,
                    layer=layer,
                    cost_per_demand_unit=float(quote.price_per_vehicle)
                    / float(quote.vehicle_capacity),
                    currency=currency,
                    source="quote",
                    tool_version=QUOTE_UNIT_COST_TOOL_VERSION,
                    quote_price_per_vehicle=float(quote.price_per_vehicle),
                    quote_vehicle_capacity=float(quote.vehicle_capacity),
                )
            )
            computed += 1
            continue
        rule = rules.get(layer)
        if rule is None:
            missing.append((origin, destination, layer))
            continue
        route = route_index.get(key)
        if route is None:
            missing.append((origin, destination, layer))
            continue
        route_fact = _route_fact_provenance(route)
        if route_fact is None:
            missing.append((origin, destination, layer))
            continue
        exact = [row for row in prior_candidates if _is_exact_calculated_cost(row, route, rule)]
        if exact:
            rows.append(exact[0])
            reused += 1
            continue
        if prior_candidates:
            stale += 1
        rows.append(
            CostMatrixRow(
                origin_id=origin,
                destination_id=destination,
                layer=layer,
                cost_per_demand_unit=(
                    rule.fixed_cost_per_demand_unit
                    + rule.cost_per_km_per_demand_unit * route.distance_km
                ),
                currency=currency,
                source="calculated",
                tool_version=DISTANCE_UNIT_COST_TOOL_VERSION,
                route_distance_km=route.distance_km,
                fixed_cost_per_demand_unit=rule.fixed_cost_per_demand_unit,
                cost_per_km_per_demand_unit=rule.cost_per_km_per_demand_unit,
                route_fact=route_fact,
            )
        )
        computed += 1
    return CostMatrix(
        currency=currency,
        warehouse_scope=warehouse_scope,
        warehouse_ids=[warehouse.warehouse_id for warehouse in warehouses],
        rows=rows,
        missing_routes=missing,
        calculation_rule=policy,
        calculation_rule_source=(
            calculation_rule_source
            if calculation_rule_source is not None
            else ("explicit" if policy is not None else None)
        ),
        calculation_rule_evidence=calculation_rule_evidence,
        calculation_rule_evidence_path=calculation_rule_evidence_path,
        stats=CostMatrixStats(
            expected_pair_count=len(expected),
            reused_pair_count=reused,
            computed_pair_count=computed,
            missing_pair_count=len(missing),
            ignored_quote_count=ignored_quotes,
            ignored_prior_row_count=ignored_prior_rows,
            stale_pair_count=stale,
            complete=not missing,
        ),
        input_identity=input_identity,
    )


def derive_observed_quote_mean_cost_policy(
    demand_cities: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    route_quotes: list[RouteCostQuote | RouteQuoteRecord],
    *,
    prepared_input_relative_path: str,
    input_identity: PlanningInputIdentity,
) -> tuple[CostCalculationPolicy, ObservedQuoteMeanCostEvidence]:
    """Derive deterministic per-layer fallback costs from complete normalized quotes."""

    required_layers = {
        layer for _, _, layer in _expected_route_pairs(demand_cities, warehouses)
    }
    quote_values: dict[NetworkLayer, list[Decimal]] = {}
    currencies: set[str] = set()
    seen: set[tuple[str, str, NetworkLayer]] = set()
    ignored = 0
    for quote in route_quotes:
        key = (quote.origin_id, quote.destination_id, quote.layer)
        if quote.layer not in required_layers:
            ignored += 1
            continue
        if key in seen:
            raise ValueError(f"cost_quote_duplicate_pair:{key}")
        seen.add(key)
        currency = str(quote.currency).upper()
        currencies.add(currency)
        quote_values.setdefault(quote.layer, []).append(
            Decimal(str(quote.price_per_vehicle)) / Decimal(str(quote.vehicle_capacity))
        )
    if len(currencies) > 1:
        raise ValueError("cost_currency_mismatch")
    if not currencies:
        raise ValueError("observed_quote_mean_requires_matching_quotes")
    missing_layers = sorted(required_layers - set(quote_values))
    if missing_layers:
        raise ValueError(
            "observed_quote_mean_missing_layers:" + ",".join(missing_layers)
        )
    currency = next(iter(currencies))
    evidence_rules: list[QuoteMeanCostRuleEvidence] = []
    calculation_rules: list[DemandUnitCostRule] = []
    for layer in sorted(required_layers):
        values = quote_values[layer]
        mean = float(
            (sum(values, Decimal(0)) / Decimal(len(values))).quantize(Decimal("0.000001"))
        )
        evidence_rules.append(
            QuoteMeanCostRuleEvidence(
                layer=layer,
                currency=currency,
                quote_count=len(values),
                mean_cost_per_demand_unit=mean,
            )
        )
        calculation_rules.append(
            DemandUnitCostRule(
                layer=layer,
                currency=currency,
                fixed_cost_per_demand_unit=mean,
                cost_per_km_per_demand_unit=0,
            )
        )
    evidence = ObservedQuoteMeanCostEvidence(
        schema_version="warehouse_quote_mean_calculation.v1",
        prepared_input_relative_path=prepared_input_relative_path,
        input_identity=input_identity,
        total_quote_count=len(route_quotes),
        method="observed_quote_mean",
        tool_version=OBSERVED_QUOTE_MEAN_TOOL_VERSION,
        formula="arithmetic_mean(price_per_vehicle / vehicle_capacity)",
        considered_quote_count=sum(len(values) for values in quote_values.values()),
        ignored_quote_count=ignored,
        rules=evidence_rules,
    )
    return CostCalculationPolicy(rules=calculation_rules), evidence


def _cost_rules_by_layer(
    policy: CostCalculationPolicy | None,
) -> dict[NetworkLayer, DemandUnitCostRule]:
    result: dict[NetworkLayer, DemandUnitCostRule] = {}
    for rule in policy.rules if policy is not None else []:
        if rule.layer in result:
            raise ValueError(f"cost_rule_duplicate_layer:{rule.layer}")
        result[rule.layer] = rule
    return result


def _is_exact_quoted_cost(
    row: CostMatrixRow,
    quote: RouteCostQuote | RouteQuoteRecord,
    currency: str,
) -> bool:
    expected_cost = float(quote.price_per_vehicle) / float(quote.vehicle_capacity)
    return (
        row.source == "quote"
        and row.tool_version == QUOTE_UNIT_COST_TOOL_VERSION
        and row.currency == currency
        and row.quote_price_per_vehicle is not None
        and row.quote_vehicle_capacity is not None
        and math.isclose(
            row.cost_per_demand_unit,
            expected_cost,
            rel_tol=0,
            abs_tol=1e-9,
        )
        and math.isclose(
            row.quote_price_per_vehicle,
            float(quote.price_per_vehicle),
            rel_tol=0,
            abs_tol=1e-9,
        )
        and math.isclose(
            row.quote_vehicle_capacity,
            float(quote.vehicle_capacity),
            rel_tol=0,
            abs_tol=1e-9,
        )
    )


def _is_exact_calculated_cost(
    row: CostMatrixRow,
    route: RouteMatrixRow,
    rule: DemandUnitCostRule,
) -> bool:
    route_fact = _route_fact_provenance(route)
    expected_cost = (
        rule.fixed_cost_per_demand_unit + rule.cost_per_km_per_demand_unit * route.distance_km
    )
    return (
        row.source == "calculated"
        and row.tool_version == DISTANCE_UNIT_COST_TOOL_VERSION
        and row.currency == rule.currency
        and row.route_distance_km is not None
        and row.fixed_cost_per_demand_unit is not None
        and row.cost_per_km_per_demand_unit is not None
        and route_fact is not None
        and row.route_fact == route_fact
        and math.isclose(
            row.cost_per_demand_unit,
            expected_cost,
            rel_tol=0,
            abs_tol=1e-9,
        )
        and math.isclose(row.route_distance_km, route.distance_km, rel_tol=0, abs_tol=1e-9)
        and math.isclose(
            row.fixed_cost_per_demand_unit,
            rule.fixed_cost_per_demand_unit,
            rel_tol=0,
            abs_tol=1e-9,
        )
        and math.isclose(
            row.cost_per_km_per_demand_unit,
            rule.cost_per_km_per_demand_unit,
            rel_tol=0,
            abs_tol=1e-9,
        )
    )


def _route_fact_provenance(route: RouteMatrixRow) -> RouteFactProvenance | None:
    coordinates = (
        route.origin_longitude,
        route.origin_latitude,
        route.destination_longitude,
        route.destination_latitude,
    )
    if any(value is None for value in coordinates):
        return None
    return RouteFactProvenance(
        method=route.method,
        tool_version=route.tool_version,
        origin_longitude=float(route.origin_longitude),
        origin_latitude=float(route.origin_latitude),
        destination_longitude=float(route.destination_longitude),
        destination_latitude=float(route.destination_latitude),
        detour_coefficient=route.detour_coefficient,
        average_speed_kph=route.average_speed_kph,
        navigation_provider=route.navigation_provider,
        navigation_profile=route.navigation_profile,
    )
