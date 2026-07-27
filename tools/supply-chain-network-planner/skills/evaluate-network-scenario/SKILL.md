---
name: evaluate-network-scenario
description: Calculate current delivery coverage and evaluate how coverage and cost change when warehouses are added, removed, or activated. Use for questions such as current 1-day coverage, best coverage on the existing footprint, or what happens if a warehouse is added in a named location.
---

# Evaluate Network Scenario

Use deterministic planning tools for calculations and keep the model responsible only
for orchestration and interpretation. Read
[planning-contracts.md](../../references/planning-contracts.md).

## Current coverage

1. If valid snapshot and route-matrix references are not already available, use
   `$prepare-network-baseline`.
2. Call `supply_chain_planner.evaluate_current_coverage`.
3. Present both values with unambiguous labels:
   - **Actual current coverage** preserves recorded `current_facility_id` relationships.
   - **Optimized existing-footprint coverage** reallocates demand within existing
     capacity and shows improvement possible without opening a warehouse.
4. Validate both returned scenario result references before presenting conclusions.
5. Explain capacity overload, missing assignments, missing routes, or unreachable lanes.

## Add-a-warehouse scenario

1. Confirm the proposed facility, capacity, handling time, fixed cost, handling cost,
   transport rates, coordinates, and routes are in the immutable snapshot and route
   matrix. If any fact is new, prepare a new baseline rather than mutating an old one.
2. Evaluate a like-for-like optimized baseline using all existing facility IDs.
3. Evaluate the candidate scenario using the same existing IDs plus the proposed
   candidate ID.
4. Call `supply_chain_planner.compare_network_scenarios` with those two result
   references.
5. Validate the baseline, candidate, and comparison resources.

## Reporting rules

State the planning period, demand grain, currency, SLA formula, active facilities,
coverage numerator and denominator, total cost, and all deltas. The implemented cost
scope is active-facility fixed cost plus handling and distance-based transport cost.
It excludes inventory, tax, construction, shutdown, service-failure, and carbon cost
unless the source model explicitly adds them in a later contract version.

Do not compare an actual-assignment baseline with an optimized candidate as if the
difference came only from the new warehouse. Use optimized-to-optimized for the
warehouse effect and show actual current coverage separately as operational context.
