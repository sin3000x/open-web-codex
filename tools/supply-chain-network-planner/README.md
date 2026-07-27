# Supply Chain Network Planner

This tools-only Codex plugin adds read-only supply-chain data preparation and
deterministic network planning without changing the Web app, Platform Server, or Codex
Runtime.

## Capability layout

- Codex and the five Skills orchestrate data preparation, mapping, scenario analysis,
  optimization, validation, and explanation.
- `supply_chain_data` is a separate read-only MCP boundary. It accepts bounded source
  IDs and publishes `planning-dataset.v1`; it exposes no SQL or source-write tool.
- `map_utils` remains the owner of address geocoding and provider navigation calls.
- `supply_chain_planner` owns typed planning Resources and deterministic coverage, cost,
  allocation, comparison, and finite-candidate location calculations.
- MCP Resources are immutable handoffs. A later platform Artifact layer can persist and
  authorize the same versioned contracts without changing the calculation semantics.

## MCP tools

Data Agent:

- `inspect_planning_source`
- `build_planning_dataset`
- `validate_planning_dataset`

Network Planning Agent:

- `prepare_network_snapshot`
- `register_route_matrix`
- `evaluate_current_coverage`
- `evaluate_network_scenario`
- `compare_network_scenarios`
- `solve_facility_location`
- `validate_network_resource`

The Data Agent first produces the common planning dataset. The three network workflows
are represented by:

1. Prepare snapshot and routes, then call `evaluate_current_coverage`.
2. Evaluate like-for-like baseline and added-warehouse scenarios, then compare them.
3. Prepare all candidates and routes, then call `solve_facility_location`.

## Local setup

The platform normally provisions shared tool environments. For manual development:

```bash
./bin/setup-env
```

The MCP launcher honors:

- `SUPPLY_CHAIN_DATA_ROOT`: authorized root for JSON `source_path` inputs.
- `SUPPLY_CHAIN_READONLY_DATA_ROOT`: deployment-bound Data Agent source catalog.
- `SUPPLY_CHAIN_DATA_RESOURCE_DIR`: optional Data Agent Resource directory.
- `SUPPLY_CHAIN_RESOURCE_DIR`: directory for immutable Resource payloads.
- `CODEX_HOME`: Profile-scoped state root used by default for Resources.
- `OPEN_WEB_CODEX_SUPPLY_CHAIN_MCP_VENV`: shared Python environment.
- `SUPPLY_CHAIN_MCP_AUTO_INSTALL=1`: allow manual first-start environment setup.

No provider API key is owned or stored by this plugin. Navigation credentials remain
inside the existing maps capability.

## Example data and tests

`examples/data-sources/warehouse-network-fixture.json` is the read-only Data Agent
source. `examples/network-input.json` and `examples/route-matrix-input.json` form a
complete small network scenario. After installing the package:

```bash
python -m pytest -q
```

The exact data contract, service-level formula, objective order, and MVP exclusions are
documented in `references/planning-contracts.md`.
