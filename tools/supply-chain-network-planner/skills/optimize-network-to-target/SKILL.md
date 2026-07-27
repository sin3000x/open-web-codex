---
name: optimize-network-to-target
description: Design a warehouse network that reaches a requested delivery-coverage target with the fewest added warehouses and lowest modeled cost. Use for requests such as reaching 90 percent 1-day coverage, deciding how many warehouses to add, or selecting from a finite set of candidate warehouse locations.
---

# Optimize Network To Target

Solve a bounded, auditable candidate-location problem. Read
[planning-contracts.md](../../references/planning-contracts.md).

## Workflow

1. Use `$prepare-network-baseline` unless a validated snapshot and complete route matrix
   already contain every existing and candidate facility.
2. Confirm the coverage target is a demand-weighted ratio in `(0, 1]`, and that the
   service policy represents the requested promise such as 1-day delivery.
3. Review the candidate set with the user when it embodies a material business choice.
   Candidate coordinates, capacity, cost, rates, and all candidate-demand routes must be
   present. The exact solver supports at most 14 candidate facilities.
4. Call `supply_chain_planner.solve_facility_location`.
5. Call `supply_chain_planner.validate_network_resource` on both `solution_ref` and the
   solution's `result_ref`.
6. If status is `infeasible`, report the best evaluated coverage and binding limitations.
   Do not reinterpret it as a successful recommendation.
7. For sensitivity, prepare separate immutable snapshots or policies and rerun the
   solver. Never overwrite assumptions inside an earlier solution.

## Objective and claim boundary

The solver keeps all existing facilities open, enumerates every subset of the finite
candidate set, and uses capacity-constrained min-cost flow for each subset. It minimizes:

1. number of added candidate facilities;
2. modeled total cost among solutions using that number;
3. higher coverage as a deterministic tie-breaker.

Demand uses integer planning units and may split across facilities. The result is exact
for the supplied candidates, route matrix, rates, capacities, and policy. It is not a
global geographic optimum and does not discover arbitrary new coordinates.

## Handoff

Report selected facilities, facility count, coverage numerator and denominator, target
gap, modeled total cost, evaluated subset count, assumptions, exclusions, and at least
one sensitivity or validation caveat. Distinguish the solver's finite-candidate
optimality from a broader strategic recommendation.
