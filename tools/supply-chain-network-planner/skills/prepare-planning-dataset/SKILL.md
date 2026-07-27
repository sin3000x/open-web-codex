---
name: prepare-planning-dataset
description: Act as the supply-chain Data Agent to inspect an authorized read-only order source, aggregate demand distribution and delivery performance, detect promotion and data-quality effects, and publish planning-dataset.v1 for downstream network planning. Use when a warehouse-network task needs orders, regions, current fulfillment relationships, service baselines, existing facilities, or a versioned data handoff before scenario simulation.
---

# Prepare Planning Dataset

Produce an evidence-bearing data handoff; do not select a warehouse-network solution.
Read [planning-contracts.md](../../references/planning-contracts.md) before publishing.

## Workflow

1. Confirm the requested planning period, demand unit, service promise, and authorized
   source ID. Never ask for organization IDs, Profile IDs, credentials, arbitrary SQL,
   filesystem paths, or write access.
2. Call `supply_chain_data.inspect_planning_source`. Review source timestamp, date range,
   row count, demand units, node count, facility count, blocking errors, and warnings.
3. If the source is appropriate, call `supply_chain_data.build_planning_dataset`. This
   reads the bound source and writes only an immutable Profile-scoped MCP Resource; it
   never modifies source data.
4. Call `supply_chain_data.validate_planning_dataset` on the returned `data_ref`.
   Do not hand off a dataset with validation errors as decision-ready.
5. Return the unchanged `data_ref`, bounded summary, demand distribution, promotion
   share, observed delivery baseline, missing observations, source range, units, and
   material quality limitations.

## Network Planning handoff

The published Resource contains a typed `network_input` projection with aggregated
demand, current facility relationships, existing facilities, rates, currency, planning
period, and service policy. The Network Planning Agent must read this same Resource and
use that projection as the baseline input to `supply_chain_planner`; it may add explicit
candidate facilities and candidate rate facts in a new immutable snapshot.

Do not copy raw order rows into messages, invent missing assignments, silently treat
promotion demand as recurring growth, or replace the Resource with a prose summary.

## Responsibility boundary

- Own source inspection, aggregation, delivery-baseline calculation, provenance, and
  data-quality reporting.
- Do not modify enterprise source data.
- Do not geocode unapproved addresses or expose direct personal identifiers.
- Do not choose candidate sites, run facility-location optimization, or recommend the
  final network.
