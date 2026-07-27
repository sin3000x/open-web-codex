# Supply-chain planning contract

## Resource lifecycle

Every calculation starts from immutable MCP Resources:

- `planning-dataset.v1`: read-only source range, demand distribution, promotion share,
  delivery baseline, data quality, and the exact `network_input` handoff projection.
- `network_snapshot.v1`: planning period, currency, service policy, demand points,
  facilities, and rate rules.
- `route_matrix.v1`: one provider/method and typed facility-demand route rows tied to
  exactly one snapshot.
- `network_scenario_result.v1`: allocations, issues, and metrics for one explicit
  facility set and calculation mode.
- `scenario_comparison.v1`: deltas between compatible scenario results.
- `facility_location_solution.v1`: target, selected candidates, exact-solver scope,
  assumptions, and its scenario-result reference.

Copy each returned `data_ref` unchanged. It uses server `supply_chain_planner` and an
opaque `supply-chain://resources/...` URI. Data Agent handoffs use server
`supply_chain_data` and `supply-chain-data://resources/...`. Publishing is
content-addressed; changing source facts or assumptions creates a new Resource.
Resource files default to the owning Profile's `CODEX_HOME`; they are not shared
application or repository state.

## Service time

For facility `f` and demand point `d`:

`end_to_end_seconds = order_cutoff_wait_seconds + facility.handling_seconds + route.travel_seconds + last_mile_buffer_seconds`

A unit is covered only when that value is less than or equal to
`max_delivery_seconds`. A “1-day” result is therefore meaningful only when the snapshot
policy explicitly encodes the intended one-day promise.

## Cost

The modeled variable unit cost is:

`facility.handling_cost_per_unit + rate.base_cost_per_unit + route.distance_km * rate.distance_cost_per_km_per_unit`

The scenario total is variable allocation cost plus fixed cost for every active
facility. Rate precedence is demand-specific, region-specific, then facility default.
All costs use the snapshot currency and planning period.

## Allocation and location

Optimized scenarios use capacity-constrained min-cost flow. Demand and capacity are
integer planning units; demand may split across facilities. The primary objective is
maximum covered demand within the service limit, followed by minimum variable cost for
the active footprint.

Facility location keeps existing facilities open and enumerates all subsets of at most
14 supplied candidates. It chooses the smallest number of candidates that reaches the
target, then the lowest total modeled cost. This is exact for the finite input set, not
for arbitrary points on a map.

## Known MVP boundary

The Data MCP accepts only deployment-bound, de-identified `planning_source.v1` fixture
or file-backed sources selected by a bounded source ID. It is not an arbitrary SQL
console and does not yet connect to a governed enterprise query gateway.

The contract does not yet model multi-echelon inventory, safety stock, SKU-specific
capacity, facility construction schedules, closure decisions, carrier step tariffs,
time-dependent traffic distributions, tax, carbon, or uncertainty. Extend the typed
contract and deterministic engine rather than embedding those semantics in a Skill
prompt.
