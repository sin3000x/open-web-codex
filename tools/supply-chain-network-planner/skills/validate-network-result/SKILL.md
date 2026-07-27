---
name: validate-network-result
description: Validate supply-chain snapshots, route matrices, coverage calculations, scenario comparisons, and facility-location solutions before they are used for a decision. Use when reviewing planning outputs, checking metric consistency, diagnosing questionable coverage or cost results, or preparing a final recommendation.
---

# Validate Network Result

Treat validation as a release gate, not a narrative afterthought. Read
[planning-contracts.md](../../references/planning-contracts.md).

## Workflow

1. Call `supply_chain_planner.validate_network_resource` for every resource used in the
   conclusion: snapshot, route matrix, scenario results, comparisons, and location
   solution.
2. Confirm all compared results share the same snapshot, route matrix, service policy,
   planning period, currency, and demand grain.
3. Reconcile coverage as covered demand units divided by total demand units. Confirm
   allocation totals equal the denominator and that no demand disappears.
4. Review route completeness, unreachable lanes, missing current assignments, current
   capacity overload, and data-source timestamps.
5. Check claim labels:
   - current means recorded assignment;
   - optimized existing footprint means reallocation without new facilities;
   - scenario means an explicit active-facility set;
   - optimal means exact only over the supplied finite candidate set.
6. Classify assumptions and exclusions separately from validation errors.

## Decision gate

Do not recommend action when validation has errors, source grain is unknown, route
matrix provenance is missing, or comparison inputs are not like-for-like. Present
warnings with their likely decision impact. A resource can be structurally valid while
still being unsuitable for a high-stakes decision because inputs are stale, estimated,
or incomplete.
