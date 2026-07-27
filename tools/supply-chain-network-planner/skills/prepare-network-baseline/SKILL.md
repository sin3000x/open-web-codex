---
name: prepare-network-baseline
description: Prepare an immutable, validated supply-chain network snapshot and a complete facility-to-demand route matrix. Use before calculating current 1-day coverage, comparing a new warehouse scenario, or optimizing warehouse locations when source demand, warehouse, transport-rate, address, coordinate, route, or service-policy data must be assembled or refreshed.
---

# Prepare Network Baseline

Create one auditable input state before any coverage, cost, or location calculation.
Read [planning-contracts.md](../../references/planning-contracts.md) before using the
planning tools.

## Workflow

1. Identify the planning period, currency, demand unit, service promise, and the exact
   end-to-end SLA formula. Do not infer a 1-day threshold from route duration alone.
2. Prefer the validated `planning-dataset.v1` produced by
   `$prepare-planning-dataset`. Read its Resource and use the exact `network_input`
   projection. Add candidate facilities and their rates explicitly for scenario or
   location work. When no planning dataset exists, load demand points, existing
   facilities, candidate facilities, and transport rates from the authorized source.
   Preserve stable business identifiers.
3. Ensure every demand point and facility has coordinates. If only addresses are present,
   use `map_utils.batch_geocode`, review failed or ambiguous matches, and then assemble
   the typed `network_input.v1` payload.
4. Call `supply_chain_planner.prepare_network_snapshot`. Carry its `data_ref` unchanged.
   Never mutate a published snapshot; prepare a new one when source facts or policy
   assumptions change.
5. Generate route distance and duration for every facility-demand pair needed by the
   analysis. Use `map_utils.distance_matrix` in deterministic batches of at most 2,500
   origin-destination elements, retaining input identifier order.
6. Convert the navigation response into `RouteEntry` rows and call
   `supply_chain_planner.register_route_matrix` with `method="navigation"`. Use
   `haversine_estimate` only when the user explicitly accepts a rough estimate and label
   every result accordingly.
7. Call `supply_chain_planner.validate_network_resource` for both snapshot and route
   matrix. Stop on validation errors. Report missing or unreachable route pairs rather
   than silently substituting straight-line distance.

## Input rules

- `demand_units` and `capacity_units` are nonnegative integer planning units.
- `currency` is one ISO-style three-letter currency and all costs use it.
- Every facility-demand pair must resolve to exactly one transport rate, using precedence:
  demand-specific, then region-specific, then facility default.
- Existing demand relationships use `current_facility_id`; missing relationships remain
  missing and are not imputed.
- Include candidate facilities and their routes in the same snapshot when the next task
  is location optimization. Otherwise the solver cannot evaluate them.

## Handoff

Return the snapshot and route-matrix `data_ref` objects, source counts, planning period,
currency, service-policy formula, route completeness, geocoding exceptions, and material
data-quality caveats. Do not paste Resource JSON into the answer.
