# Map Utils MCP

Python MCP server that exposes paid Google Maps and Mapbox operations without modifying
`codex-rs`:

- `batch_geocode`
- `batch_reverse_geocode`
- `get_route`
- `distance_matrix`
- `execute_navigation_matrix`
- `publish_workspace_geojson`
- `create_map_card`
- `create_network_map_card`
- `revise_map_card`

The geocoding and routing tools use one active provider/key pair selected in
configuration; provider and credentials are never model-visible tool arguments.
A later configuration replaces the previous provider and key. Provider request
limits are handled inside the server. Batch geocoding is capped at 500 inputs
and distance matrices at 2,500 billable origin/destination elements per call.

## Credential configuration

In the Web app, open **Settings → 地图服务 → 配置 / 更换地图 Token**. The same secure dialog is
available on a pending maps credential request. Enter the key in that password field, never as a
chat message. The platform validates permission and stores one active provider/key pair in its
encrypted global configuration. Saving replaces that pair; retry the conversation after saving.
Google credentials stay server-only. A Mapbox public browser token is returned because Mapbox GL
needs it for map cards. Saving configuration does not prove the external provider accepts the key.

The checked-in `runtime.toml` explicitly launches this server with `--credential-source platform`.
In this mode every credentialed operation asks for a fresh MCP URL delivery from the platform;
it never reads or writes provider-local credential memory, even when a submitted form requests
remembering. This makes both key rotation and provider changes effective on the next operation,
without a process restart or a second credential source. Delivery failure, rejection or cancellation
fails the operation explicitly; it never falls back to a local key.

The delivery uses a single-use `127.0.0.1` URL with a random 256-bit path token. The browser does
not navigate there: the platform's secure dialog saves the key, and the Server submits it directly
to the local MCP process before accepting the Runtime elicitation. When already configured, the
platform delivers its saved key without exposing it to the model.

For independently launched `maps-mcp` outside the Web platform, the CLI's `--credential-source local`
mode (the CLI default) owns its explicit local configuration. It reads a remembered key from
`<tool-state-root>/.codex/maps-tool-memory.json` and lets the URL form remember a replacement with
file mode `0600` and directory mode `0700`. This standalone mode is not consulted by Web-managed
operations. Neither mode reads provider keys from environment variables. Keys must never enter
`MEMORY.md`, instructions, prompts, model-visible arguments, results or logs.

## Install

This Tool source declares its Python project, Node project, hash lock, `map_utils` module entry and
typed environment bindings in `runtime.toml`. The SDK generic provisioner owns virtualenv creation,
pip/npm caches, staged non-editable installation, `npm ci --ignore-scripts`, timeouts and reuse.
Prepared environments live under the caller-provided output root, never inside this Tool, a Profile
or a Workspace. Python 3, Node.js and npm are explicit host adapters.

For manual repository development, invoke the same platform preparation contract from the
repository root:

```bash
python3 -m pip install -e packages/copilot-provider-sdk -e packages/copilot-sdk
python3 -m copilot_sdk prepare copilots/warehouse-network \
  --output-root "$PWD/.local/open-web-codex/copilot-environment"
```

`scripts/run-local.sh` performs this generic preparation once before a real Platform Server starts.
It does not contain maps-specific installation logic. Runtime launch and user conversations never
download or install Tool dependencies.

Google projects must enable Geocoding API v4 and Routes API. Mapbox requires an access token with
Geocoding, Directions, and Matrix access.

## Codex discovery

This directory is a Tool source, not an authored Plugin transport root. The configured Copilot
manifest identifies it as capability root `map_utils` and explicitly references `runtime.toml`.
The provisioner compiles an internal prepared descriptor; SDK `dev`/`test` create disposable
`.codex-plugin`/`.mcp.json` projections from it, while the Platform Server consumes the same
descriptor and projects the exact transport into the Profile Role. Neither path scans source or
writes hidden Profile configuration.

The Network Role keeps `default_tools_approval_mode` at `prompt` because geocoding, routing and
distance matrix calls reach a credentialed external provider and may be billable. Only
`create_map_card` and `revise_map_card` are explicitly preapproved. This Role policy remains separate from environment
preparation and from provider credential elicitation. The MCP client must advertise URL elicitation
support; if the browser cannot render the request, the Tool fails safely instead of exposing a key
to the model.

## Tests

Tests use fake HTTP responses and never call a paid provider:

```bash
PYTHONPATH=. python3 -m unittest discover -s tests -v
```

Map-card output:

Geocoding and routing tools publish GeoJSON through a standard MCP `resource_link`. Their
schema-validated output contains the raw MCP server ID and Resource URI in `data_ref`. Copy the
complete object unchanged into `create_map_card.sources.<source-id>.data_ref` in the same Run and
Thread. When downstream work needs the GeoJSON contents, pass `data_ref.server` and `data_ref.uri`
unchanged to MCP `resources/read`; `mcp__map_utils` is a model-visible Tool namespace, not the
Resource server ID.

`create_map_card` accepts one Mapbox-style `map.v3` contract. `sources` contains
platform-managed GeoJSON and must use the Open Web `source.data_ref`; GeoJSON contents are never
passed through the model context. Standard GeoJSON source options are preserved.
`layers` is official Mapbox Style Specification Layer JSON and is validated by
`@mapbox/mapbox-gl-style-spec`; there is no separate layer, paint, layout, filter, or expression
whitelist. Official unknown-property diagnostics are warnings, while invalid known syntax fails.

Camera fields are the standard top-level `center`, `zoom`, `bearing`, and `pitch`; omitting
`center` and `zoom` fits all GeoJSON. Optional Open Web behavior lives under
`extensions.hover` and `extensions.legend`. There is no `style` wrapper, old `view` object,
layer-local hover, or top-level legend.

The Tool returns an `open-web-artifact` / `inline-visualization.v1` envelope with a typed `map.v3`
renderer and a provider-owned immutable, content-addressed `map_card_spec.v1` ResourceLink. The host independently
validates the browser DTO, resolves authorized `data_ref` values to opaque URLs, and strips MCP
Resource identity from public events. The browser passes every layer to `map.addLayer` unchanged
except for browser-local layer/source IDs. Tool completion only creates an inline card; it does not
display the map. `revise_map_card` consumes one exact map spec plus a bounded patch and returns a
new child spec; pure style revisions reuse source GeoJSON, while new coverage geometry must come
from the domain Network Tool. To display the card, Assistant messages copy only
`structuredContent.embed.code` verbatim as a standalone paragraph with a blank line before and
after it. That paragraph may appear anywhere in the response where the map should be shown and
must not be wrapped in a code fence, blockquote, or list.

Provider endpoints implemented:

- Google Geocoding API v4 address/location endpoints
- Google Routes API `computeRoutes` and `computeRouteMatrix`
- Mapbox Geocoding API v6 batch endpoint
- Mapbox Directions API v5 and Matrix API v1
