# Supply-chain planning contract

## Prepared input, Resource and Artifact lifecycle

Every calculation starts from one create-new `prepared_network_input.v2` file under
`outputs/warehouse-network/prepared/`. The Data Agent hands off only its exact Workspace-relative
path, `input_identity`, readiness and bounded quality summaries. The file contains the confirmed
demand, warehouse, assignment, route-quote and optional provided-route facts; it is not an MCP
Resource and is never copied into the Agent conversation.

Network calculations derived from that exact prepared identity are immutable MCP Resources:

- `route_matrix.v3`: one provider/method, discriminated warehouse scope and exact sorted
  `warehouse_ids`, plus typed origin-city to destination-city
  route rows for an explicit warehouse scope. Multiple demand points in one city reuse
  the same lane.
- `cost_matrix.v3`: quote-first or explicitly derived costs for the same exact warehouse set,
  calculation policy and prepared identity.
- `network_baseline.v2` and `network_scenario.v2`: allocations, issues and metrics for
  one explicit current or candidate facility set.
- `network_plan_comparison.v2`: deltas between any compatible baseline, scenario and
  facility-location before/after results.
- `facility_location_solution.v4`: target, selected candidates, two-stage solver stages,
  assumptions and its assignment reference.
- `network_planning_report_markdown.v2`: the final Workspace report delivery contract.
  Comparison map data remains provider-owned GeoJSON for the map-card Tool.

Copy each returned Network `resource_ref` unchanged. GeoJSON map-data tools return the narrower
`data_ref`; copy that object unchanged into the Maps Tool. Both use server `supply_chain` and an
opaque `supply-chain://resources/...` URI. Publishing is content-addressed; changing source facts,
scope or assumptions creates a new Resource.
Resource files default to the owning Profile's `CODEX_HOME`; they are not shared
application or repository state.

For Data intake, `inspect_workspace_sources` profiles raw sources only and rejects a prepared
input path with `prepared_input_not_raw_source`; an administrative catalog is included in inspected
paths but is passed separately to preparation.
`country_code` is optional: when omitted, the Tool derives one only from a uniquely identified
administrative catalog metadata record; missing or conflicting metadata leaves it null with a bounded
warning. The Tool returns a bounded inline
`workspace_source_profile.v2`, exact source units, `preview_sample_count`, `total_count`,
`total_count_exact`, `inspection_identity` and inspected relative paths. Preview rows are schema
examples only; `total_count` is the complete unit count. Each unit carries `role_assessments`
(`complete`, `partial` or `ambiguous`); preparation auto-resolves mappings only from a single
`complete` assessment for the selected role.

`resolve_prepared_network_input` owns prepared-input reuse. It accepts exact `prepared_candidate`
paths from discovery and `required_roles` from the five planning roles `demand`,
`existing_warehouse`, `candidate_warehouse`, `current_assignment` and `route_quote`. Each file must
be ready, contain the required roles, match an explicit `country_code` or otherwise its recorded
country, and keep fresh raw/admin provenance. Fresh files are compared by their normalized records
for the requested roles only (existing and candidate warehouses split by `is_existing`; route quotes
include provided route facts). Identical records resolve to one `prepared_ready` result that reuses
the file with the most prepared roles, then the canonical path, and reports `equivalent_count`;
differing records return `prepared_selection_required` with one summary per distinct group; no fresh
file returns `prepared_unavailable` with bounded per-file reasons and
`next_action="inspect_sources"`. `prepare_network_input`
rereads the selected complete units and recomputes the identity before writing; a path, byte or
selected-set change returns `source_changed` without creating output. `needs_input` is a terminal
business handoff, not a retry loop. Data registers no Resource or Resource template, so callers must
not use `read_mcp_resource` for the inspection result.

`query_prepared_network_input` is the read-only fact query over one exact prepared input. It
evaluates typed filters, up to two `group_by` fields, sum/min/max/mean measures and an ordered row
list of at most 100 records over every record of the selected dataset, and returns
`prepared_network_query.v1` with exact `record_count`, `matched_count`, groups and measures. A changed
selected source returns `prepared_input_stale` with `next_action="reinspect"` and no facts. Business
fact questions are answered by this Tool directly on a discovered prepared candidate, without a prior
inspection, and never by inspection preview rows. Each group carries its exact record count.

`prepare_route_matrix` returns a discriminated `ready | needs_input` result. `ready` always has
zero missing pairs and the exact `route_matrix.v3` reference. An incomplete provided scope returns
bounded missing-pair evidence and executable completion choices without publishing a consumable
route reference. Baseline, facility-change and location Tools independently reject incomplete
route matrices. Only Root may turn the child handoff into the native `request_user_input` card.

`prepare_network_coverage_map` requires one `service_target_hours` value already present in the
source result. Planner assigns `attained`, `missed` or `unassigned` to every demand point and the
corresponding status to each available last-mile line. Maps consumes these values for presentation
and does not derive service attainment from display-layer expressions.

Every Network Resource-producing Tool also returns `resource_name` in its structured result.
Use that exact stable name when citing evidence; never expose or relabel the opaque
Resource URI as a human-readable name. `resource_ref` is the calculation handoff identity,
`data_ref` is the GeoJSON-specific handoff identity, and `resource_name` is the report citation
identity.

The Resource is the Runtime handoff. Intermediate Resource links remain provider-owned
and are never registered as Task-owned Artifacts. Only an allowlisted final map or report
Tool may register its explicit Workspace-relative delivery descriptor:

1. the Artifact receives an independent ID and Task read grant;
2. Run, Thread, Turn and Item IDs are recorded only as producer provenance;
3. content is materialized from the authorized Workspace create-new file;
4. the exact report delivery contract, MIME type and size are checked for bounded,
   browser-safe `text/markdown`;
5. browser DTOs expose the Artifact identity and authorized content URL, not the
   internal MCP Resource URI.

Artifact registration does not change the calculation contract. A child Agent still
passes the original `resource_ref` or GeoJSON `data_ref` unchanged inside Runtime; the Platform Artifact provides
durable product identity, authorization and recovery only for the explicit final file.
Deleting a producing Run must not delete the Artifact, its Task grant or provenance
identity. Interactive maps use the separate exact map-card handoff and do not become
Workspace files or Artifacts unless the user explicitly asks to export one.

Current lifecycle states are:

```text
pending -> materializing -> ready
pending -> materializing -> failed
```

Replacement, invalidation, deletion, retention policy and authorized cross-Run reuse
remain separate Platform lifecycle work.

## Service time

For facility `f` in origin city `o` and demand point `d` in destination city `c`:

`end_to_end_seconds = order_cutoff_wait_seconds + facility.handling_seconds + route.travel_seconds + last_mile_buffer_seconds`

A unit is covered only when that value is less than or equal to
`max_delivery_seconds`. A “1-day” result is therefore meaningful only when the snapshot
policy explicitly encodes the intended one-day promise.

## Cost

The modeled variable unit cost is:

`facility.handling_cost_per_unit + lane.base_cost_per_unit + lane.distance_km * lane.distance_cost_per_km_per_unit`

The lane is selected exactly by `(o.city_id, c.city_id)`; lanes never use a demand-point
identifier. The scenario total is variable allocation cost plus fixed cost for every active
facility.
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

The Data MCP discovers only supported files in the trusted Turn Workspace and accepts
opaque source references from that discovery. It has no packaged source catalog, source-ID
fallback or arbitrary SQL interface. There is no separate Demo MCP; the checked-in synthetic
tutorial fixture is copied as ordinary Workspace files only after explicit user authorization
and still requires the normal published-profile, mapping-confirmation and normalization flow.

The checked-in tutorial fixture has bounded city-grain sources and is copied as ordinary
Workspace files only when the user explicitly requests tutorial/mock data. There is no
separate Demo MCP and no implicit fallback to fixture files. Demand does not carry a duplicate
coordinate or synthetic order identifier; coordinates are owned by the city records.

The contract does not yet model multi-echelon inventory, safety stock, SKU-specific
capacity, facility construction schedules, closure decisions, carrier step tariffs,
time-dependent traffic distributions, tax, carbon, or uncertainty. Extend the typed
contract and deterministic engine rather than embedding those semantics in a Skill
prompt.
