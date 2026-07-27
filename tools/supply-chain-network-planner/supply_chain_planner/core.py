"""Deterministic coverage, cost, and finite-candidate location algorithms."""

from __future__ import annotations

import hashlib
import heapq
import json
from dataclasses import dataclass
from decimal import Decimal, ROUND_HALF_UP
from itertools import combinations
from typing import Iterable

from .models import (
    Allocation,
    DemandPoint,
    Facility,
    NetworkInput,
    NetworkMetrics,
    NetworkScenarioResult,
    NetworkSnapshot,
    RouteEntry,
    RouteMatrix,
    ScenarioComparison,
    TransportRate,
    utc_now,
)

COST_SCALE = Decimal("1000000")
MAX_EXACT_CANDIDATES = 14


def _content_id(prefix: str, payload: object) -> str:
    encoded = json.dumps(
        payload,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        default=str,
    ).encode("utf-8")
    return f"{prefix}_{hashlib.sha256(encoded).hexdigest()[:24]}"


def create_snapshot(network: NetworkInput, source_name: str | None = None) -> NetworkSnapshot:
    payload = network.model_dump(mode="json")
    payload.pop("schema_version", None)
    return NetworkSnapshot(
        **payload,
        schema_version="network_snapshot.v1",
        snapshot_id=_content_id("snapshot", payload),
        created_at=utc_now(),
        source_name=source_name,
    )


def create_route_matrix(
    snapshot: NetworkSnapshot,
    *,
    provider: str,
    method: str,
    entries: list[RouteEntry],
) -> RouteMatrix:
    facility_ids = {facility.facility_id for facility in snapshot.facilities}
    demand_ids = {demand.demand_id for demand in snapshot.demand_points}
    pairs: set[tuple[str, str]] = set()
    for entry in entries:
        if entry.origin_facility_id not in facility_ids:
            raise ValueError(
                f"route references unknown facility {entry.origin_facility_id!r}"
            )
        if entry.destination_demand_id not in demand_ids:
            raise ValueError(
                f"route references unknown demand {entry.destination_demand_id!r}"
            )
        pair = (entry.origin_facility_id, entry.destination_demand_id)
        if pair in pairs:
            raise ValueError(f"duplicate route pair {pair!r}")
        pairs.add(pair)
    payload = {
        "snapshot_id": snapshot.snapshot_id,
        "provider": provider,
        "method": method,
        "entries": [entry.model_dump(mode="json") for entry in entries],
    }
    return RouteMatrix(
        route_matrix_id=_content_id("routes", payload),
        snapshot_id=snapshot.snapshot_id,
        provider=provider,
        method=method,
        entries=entries,
    )


def _route_index(matrix: RouteMatrix) -> dict[tuple[str, str], RouteEntry]:
    return {
        (entry.origin_facility_id, entry.destination_demand_id): entry
        for entry in matrix.entries
    }


def _resolve_rate(
    snapshot: NetworkSnapshot,
    facility_id: str,
    demand: DemandPoint,
) -> TransportRate:
    candidates = [
        rate
        for rate in snapshot.transport_rates
        if rate.origin_facility_id == facility_id
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
            f"missing transport rate for facility {facility_id!r} and demand "
            f"{demand.demand_id!r}"
        )
    return max(
        candidates,
        key=lambda rate: (
            rate.destination_demand_id is not None,
            rate.destination_region is not None,
        ),
    )


def _unit_cost(
    snapshot: NetworkSnapshot,
    facility: Facility,
    demand: DemandPoint,
    route: RouteEntry,
) -> tuple[Decimal, str]:
    if route.distance_meters is None:
        raise ValueError("ready route is missing distance")
    rate = _resolve_rate(snapshot, facility.facility_id, demand)
    distance_km = Decimal(route.distance_meters) / Decimal("1000")
    cost = (
        facility.handling_cost_per_unit
        + rate.base_cost_per_unit
        + rate.distance_cost_per_km_per_unit * distance_km
    )
    return cost.quantize(Decimal("0.000001"), rounding=ROUND_HALF_UP), rate.rate_id


def _end_to_end_seconds(
    snapshot: NetworkSnapshot,
    facility: Facility,
    route: RouteEntry,
) -> int:
    if route.travel_seconds is None:
        raise ValueError("ready route is missing travel time")
    policy = snapshot.service_policy
    return (
        policy.order_cutoff_wait_seconds
        + facility.handling_seconds
        + route.travel_seconds
        + policy.last_mile_buffer_seconds
    )


def _metrics(
    snapshot: NetworkSnapshot,
    facilities: Iterable[Facility],
    allocations: list[Allocation],
) -> NetworkMetrics:
    facility_list = list(facilities)
    total = sum(item.demand_units for item in snapshot.demand_points)
    covered = sum(item.units for item in allocations if item.covered)
    variable = sum((item.variable_cost for item in allocations), Decimal("0"))
    fixed = sum((item.fixed_cost for item in facility_list), Decimal("0"))
    return NetworkMetrics(
        total_demand_units=total,
        covered_demand_units=covered,
        uncovered_demand_units=max(0, total - covered),
        coverage_ratio=covered / total if total else 0,
        fixed_cost=fixed,
        variable_cost=variable.quantize(Decimal("0.000001")),
        total_cost=(fixed + variable).quantize(Decimal("0.000001")),
        active_facility_count=len(facility_list),
    )


def evaluate_current_assignment(
    snapshot: NetworkSnapshot,
    matrix: RouteMatrix,
) -> NetworkScenarioResult:
    _ensure_compatible(snapshot, matrix)
    facilities = {
        facility.facility_id: facility
        for facility in snapshot.facilities
        if facility.is_existing
    }
    routes = _route_index(matrix)
    allocations: list[Allocation] = []
    issues: list[str] = []
    used_capacity = {facility_id: 0 for facility_id in facilities}

    for demand in sorted(snapshot.demand_points, key=lambda item: item.demand_id):
        facility_id = demand.current_facility_id
        if facility_id is None:
            allocations.append(
                Allocation(
                    demand_id=demand.demand_id,
                    facility_id=None,
                    units=demand.demand_units,
                    covered=False,
                    reason="missing_current_assignment",
                )
            )
            issues.append(f"demand {demand.demand_id}: missing current facility assignment")
            continue
        facility = facilities.get(facility_id)
        if facility is None:
            allocations.append(
                Allocation(
                    demand_id=demand.demand_id,
                    facility_id=facility_id,
                    units=demand.demand_units,
                    covered=False,
                    reason="current_facility_is_not_active",
                )
            )
            issues.append(
                f"demand {demand.demand_id}: current facility {facility_id} is not existing"
            )
            continue
        used_capacity[facility_id] += demand.demand_units
        route = routes.get((facility_id, demand.demand_id))
        if route is None or route.status != "ready":
            allocations.append(
                Allocation(
                    demand_id=demand.demand_id,
                    facility_id=facility_id,
                    units=demand.demand_units,
                    covered=False,
                    reason="route_missing_or_unreachable",
                )
            )
            issues.append(
                f"demand {demand.demand_id}: route from {facility_id} is missing or unreachable"
            )
            continue
        end_to_end = _end_to_end_seconds(snapshot, facility, route)
        unit_cost, rate_id = _unit_cost(snapshot, facility, demand, route)
        covered = end_to_end <= snapshot.service_policy.max_delivery_seconds
        allocations.append(
            Allocation(
                demand_id=demand.demand_id,
                facility_id=facility_id,
                units=demand.demand_units,
                covered=covered,
                end_to_end_seconds=end_to_end,
                distance_meters=route.distance_meters,
                variable_cost=unit_cost * demand.demand_units,
                rate_id=rate_id,
                reason=None if covered else "service_level_exceeded",
            )
        )
    for facility_id, used in sorted(used_capacity.items()):
        capacity = facilities[facility_id].capacity_units
        if used > capacity:
            issues.append(
                f"facility {facility_id}: current assigned demand {used} exceeds capacity "
                f"{capacity}"
            )

    active = sorted(facilities.values(), key=lambda item: item.facility_id)
    return NetworkScenarioResult(
        result_id=_content_id(
            "result",
            {
                "snapshot": snapshot.snapshot_id,
                "routes": matrix.route_matrix_id,
                "mode": "current_assignment",
            },
        ),
        snapshot_id=snapshot.snapshot_id,
        route_matrix_id=matrix.route_matrix_id,
        service_policy_id=snapshot.service_policy.policy_id,
        scenario_id="current_assignment",
        mode="current_assignment",
        active_facility_ids=[item.facility_id for item in active],
        metrics=_metrics(snapshot, active, allocations),
        allocations=allocations,
        issues=issues,
    )


@dataclass
class _Edge:
    to: int
    reverse: int
    capacity: int
    cost: int
    original_capacity: int


class _MinCostFlow:
    def __init__(self, node_count: int):
        self.graph: list[list[_Edge]] = [[] for _ in range(node_count)]

    def add_edge(self, source: int, target: int, capacity: int, cost: int) -> _Edge:
        forward = _Edge(target, len(self.graph[target]), capacity, cost, capacity)
        reverse = _Edge(source, len(self.graph[source]), 0, -cost, 0)
        self.graph[source].append(forward)
        self.graph[target].append(reverse)
        return forward

    def send(self, source: int, sink: int, amount: int) -> tuple[int, int]:
        node_count = len(self.graph)
        potential = [0] * node_count
        flow = 0
        total_cost = 0
        while flow < amount:
            distance = [10**30] * node_count
            previous_node = [-1] * node_count
            previous_edge = [-1] * node_count
            distance[source] = 0
            queue: list[tuple[int, int]] = [(0, source)]
            while queue:
                current_distance, node = heapq.heappop(queue)
                if current_distance != distance[node]:
                    continue
                for edge_index, edge in enumerate(self.graph[node]):
                    if edge.capacity <= 0:
                        continue
                    reduced_cost = edge.cost + potential[node] - potential[edge.to]
                    candidate = current_distance + reduced_cost
                    if candidate < distance[edge.to]:
                        distance[edge.to] = candidate
                        previous_node[edge.to] = node
                        previous_edge[edge.to] = edge_index
                        heapq.heappush(queue, (candidate, edge.to))
            if previous_node[sink] == -1:
                break
            for node in range(node_count):
                if distance[node] < 10**30:
                    potential[node] += distance[node]
            increment = amount - flow
            node = sink
            while node != source:
                previous = previous_node[node]
                edge = self.graph[previous][previous_edge[node]]
                increment = min(increment, edge.capacity)
                node = previous
            node = sink
            while node != source:
                previous = previous_node[node]
                edge = self.graph[previous][previous_edge[node]]
                edge.capacity -= increment
                self.graph[node][edge.reverse].capacity += increment
                total_cost += increment * edge.cost
                node = previous
            flow += increment
        return flow, total_cost


def evaluate_optimized_network(
    snapshot: NetworkSnapshot,
    matrix: RouteMatrix,
    *,
    active_facility_ids: list[str],
    scenario_id: str,
) -> NetworkScenarioResult:
    _ensure_compatible(snapshot, matrix)
    all_facilities = {item.facility_id: item for item in snapshot.facilities}
    unknown = sorted(set(active_facility_ids) - set(all_facilities))
    if unknown:
        raise ValueError(f"unknown active facility identifiers: {unknown}")
    if len(active_facility_ids) != len(set(active_facility_ids)):
        raise ValueError("active_facility_ids contains duplicates")

    facilities = [
        all_facilities[facility_id] for facility_id in sorted(active_facility_ids)
    ]
    demands = sorted(snapshot.demand_points, key=lambda item: item.demand_id)
    routes = _route_index(matrix)
    issues: list[str] = []
    total_demand = sum(item.demand_units for item in demands)
    max_unit_cost = 0
    eligible: dict[tuple[str, str], tuple[RouteEntry, Decimal, str, int]] = {}
    for demand in demands:
        for facility in facilities:
            route = routes.get((facility.facility_id, demand.demand_id))
            if route is None:
                issues.append(
                    f"missing route: {facility.facility_id} -> {demand.demand_id}"
                )
                continue
            if route.status != "ready":
                continue
            end_to_end = _end_to_end_seconds(snapshot, facility, route)
            if end_to_end > snapshot.service_policy.max_delivery_seconds:
                continue
            unit_cost, rate_id = _unit_cost(snapshot, facility, demand, route)
            cost_units = int(
                (unit_cost * COST_SCALE).quantize(Decimal("1"), rounding=ROUND_HALF_UP)
            )
            max_unit_cost = max(max_unit_cost, cost_units)
            eligible[(demand.demand_id, facility.facility_id)] = (
                route,
                unit_cost,
                rate_id,
                end_to_end,
            )

    source = 0
    demand_offset = 1
    facility_offset = demand_offset + len(demands)
    uncovered_node = facility_offset + len(facilities)
    sink = uncovered_node + 1
    flow = _MinCostFlow(sink + 1)
    demand_edges: dict[tuple[str, str], _Edge] = {}
    uncovered_edges: dict[str, _Edge] = {}
    facility_nodes = {
        facility.facility_id: facility_offset + index
        for index, facility in enumerate(facilities)
    }
    uncovered_penalty = (max_unit_cost + 1) * (total_demand + 1)
    for index, demand in enumerate(demands):
        demand_node = demand_offset + index
        flow.add_edge(source, demand_node, demand.demand_units, 0)
        for facility in facilities:
            key = (demand.demand_id, facility.facility_id)
            details = eligible.get(key)
            if details is None:
                continue
            unit_cost = details[1]
            cost_units = int(
                (unit_cost * COST_SCALE).quantize(Decimal("1"), rounding=ROUND_HALF_UP)
            )
            demand_edges[key] = flow.add_edge(
                demand_node,
                facility_nodes[facility.facility_id],
                demand.demand_units,
                cost_units,
            )
        uncovered_edges[demand.demand_id] = flow.add_edge(
            demand_node,
            uncovered_node,
            demand.demand_units,
            uncovered_penalty,
        )
    for facility in facilities:
        flow.add_edge(
            facility_nodes[facility.facility_id],
            sink,
            facility.capacity_units,
            0,
        )
    flow.add_edge(uncovered_node, sink, total_demand, 0)
    sent, _ = flow.send(source, sink, total_demand)
    if sent != total_demand:
        raise RuntimeError(f"allocation flow terminated early: sent {sent} of {total_demand}")

    allocations: list[Allocation] = []
    for demand in demands:
        for facility in facilities:
            key = (demand.demand_id, facility.facility_id)
            edge = demand_edges.get(key)
            if edge is None:
                continue
            units = edge.original_capacity - edge.capacity
            if units <= 0:
                continue
            route, unit_cost, rate_id, end_to_end = eligible[key]
            allocations.append(
                Allocation(
                    demand_id=demand.demand_id,
                    facility_id=facility.facility_id,
                    units=units,
                    covered=True,
                    end_to_end_seconds=end_to_end,
                    distance_meters=route.distance_meters,
                    variable_cost=unit_cost * units,
                    rate_id=rate_id,
                )
            )
        uncovered = uncovered_edges[demand.demand_id]
        uncovered_units = uncovered.original_capacity - uncovered.capacity
        if uncovered_units:
            allocations.append(
                Allocation(
                    demand_id=demand.demand_id,
                    facility_id=None,
                    units=uncovered_units,
                    covered=False,
                    reason="no_eligible_capacity_within_service_level",
                )
            )

    return NetworkScenarioResult(
        result_id=_content_id(
            "result",
            {
                "snapshot": snapshot.snapshot_id,
                "routes": matrix.route_matrix_id,
                "mode": "optimized",
                "active": sorted(active_facility_ids),
                "scenario": scenario_id,
            },
        ),
        snapshot_id=snapshot.snapshot_id,
        route_matrix_id=matrix.route_matrix_id,
        service_policy_id=snapshot.service_policy.policy_id,
        scenario_id=scenario_id,
        mode="optimized",
        active_facility_ids=sorted(active_facility_ids),
        metrics=_metrics(snapshot, facilities, allocations),
        allocations=allocations,
        issues=issues,
    )


def compare_scenarios(
    baseline: NetworkScenarioResult,
    candidate: NetworkScenarioResult,
) -> ScenarioComparison:
    if baseline.snapshot_id != candidate.snapshot_id:
        raise ValueError("scenario results use different network snapshots")
    if baseline.route_matrix_id != candidate.route_matrix_id:
        raise ValueError("scenario results use different route matrices")
    if baseline.service_policy_id != candidate.service_policy_id:
        raise ValueError("scenario results use different service policies")
    return ScenarioComparison(
        baseline_result_id=baseline.result_id,
        candidate_result_id=candidate.result_id,
        coverage_ratio_delta=(
            candidate.metrics.coverage_ratio - baseline.metrics.coverage_ratio
        ),
        covered_demand_units_delta=(
            candidate.metrics.covered_demand_units
            - baseline.metrics.covered_demand_units
        ),
        total_cost_delta=candidate.metrics.total_cost - baseline.metrics.total_cost,
        fixed_cost_delta=candidate.metrics.fixed_cost - baseline.metrics.fixed_cost,
        variable_cost_delta=(
            candidate.metrics.variable_cost - baseline.metrics.variable_cost
        ),
    )


def candidate_subsets(
    snapshot: NetworkSnapshot,
) -> tuple[list[str], list[str]]:
    existing = sorted(
        item.facility_id for item in snapshot.facilities if item.is_existing
    )
    candidates = sorted(
        item.facility_id for item in snapshot.facilities if not item.is_existing
    )
    if len(candidates) > MAX_EXACT_CANDIDATES:
        raise ValueError(
            f"exact location solver supports at most {MAX_EXACT_CANDIDATES} candidate "
            f"facilities; received {len(candidates)}"
        )
    return existing, candidates


def solve_location_candidates(
    snapshot: NetworkSnapshot,
    matrix: RouteMatrix,
    *,
    target_coverage_ratio: float,
) -> tuple[NetworkScenarioResult, list[str], int, bool]:
    if not 0 < target_coverage_ratio <= 1:
        raise ValueError("target_coverage_ratio must be greater than 0 and at most 1")
    existing, candidates = candidate_subsets(snapshot)
    evaluated = 0
    best_feasible: NetworkScenarioResult | None = None
    best_feasible_candidates: list[str] = []
    best_infeasible: NetworkScenarioResult | None = None
    best_infeasible_candidates: list[str] = []

    for candidate_count in range(len(candidates) + 1):
        feasible_at_count: list[tuple[NetworkScenarioResult, list[str]]] = []
        for subset_tuple in combinations(candidates, candidate_count):
            subset = list(subset_tuple)
            result = evaluate_optimized_network(
                snapshot,
                matrix,
                active_facility_ids=existing + subset,
                scenario_id=f"location_{candidate_count}_{'_'.join(subset) or 'none'}",
            )
            evaluated += 1
            if (
                best_infeasible is None
                or result.metrics.coverage_ratio
                > best_infeasible.metrics.coverage_ratio
                or (
                    result.metrics.coverage_ratio
                    == best_infeasible.metrics.coverage_ratio
                    and (
                        len(subset),
                        result.metrics.total_cost,
                        subset,
                    )
                    < (
                        len(best_infeasible_candidates),
                        best_infeasible.metrics.total_cost,
                        best_infeasible_candidates,
                    )
                )
            ):
                best_infeasible = result
                best_infeasible_candidates = subset
            if result.metrics.coverage_ratio + 1e-12 >= target_coverage_ratio:
                feasible_at_count.append((result, subset))
        if feasible_at_count:
            best_feasible, best_feasible_candidates = min(
                feasible_at_count,
                key=lambda item: (
                    item[0].metrics.total_cost,
                    -item[0].metrics.coverage_ratio,
                    item[1],
                ),
            )
            break

    if best_feasible is not None:
        return best_feasible, best_feasible_candidates, evaluated, True
    if best_infeasible is None:
        raise RuntimeError("location solver evaluated no candidate subsets")
    return best_infeasible, best_infeasible_candidates, evaluated, False


def _ensure_compatible(snapshot: NetworkSnapshot, matrix: RouteMatrix) -> None:
    if matrix.snapshot_id != snapshot.snapshot_id:
        raise ValueError(
            f"route matrix snapshot {matrix.snapshot_id!r} does not match "
            f"{snapshot.snapshot_id!r}"
        )
