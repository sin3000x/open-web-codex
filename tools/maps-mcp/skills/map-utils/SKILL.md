---
name: map-utils
description: Use the map_utils MCP server for geocoding, reverse geocoding, routes, travel paths, distance matrices, coordinates, or interactive map-card output backed by the configured Google Maps or Mapbox provider.
---

# Map Utils

Use `map_utils` for geocoding, routing, travel matrices, and interactive maps. Never request or
pass a provider key in chat; the MCP server owns configuration.

## Data workflow

1. Call the relevant data Tool:
   - `batch_geocode` for addresses.
   - `batch_reverse_geocode` for coordinates.
   - `get_route` for a route.
   - `distance_matrix` for travel distances or times.
2. Data Tools return `structuredContent.data_ref` and a matching MCP `resource_link`.
3. For a map, copy the complete `data_ref` unchanged to
   `create_map_card.sources.<source-id>.data_ref`. Do not read or reproduce large Resource
   GeoJSON merely to build the card.
4. The Resource server is exactly `map_utils`, never the model-visible Tool namespace
   `mcp__map_utils`.
5. A successful `create_map_card` call only creates the Artifact; it does not display the map.
   To display it, copy `structuredContent.embed.code` verbatim into the Assistant response as a
   standalone paragraph, with a blank line before and after it. The paragraph may appear anywhere
   in the response where the map should be shown.
6. Do not wrap the embed code in a code fence, blockquote, or list, and do not merely describe the
   map. The map is not displayed until the standalone embed paragraph is present.

## Map card protocol

`create_map_card` is Mapbox-style authoring over a platform-owned basemap:

- `sources` is platform-managed GeoJSON source JSON.
- `layers` is standard Mapbox Style Specification Layer JSON.
- `center`, `zoom`, `bearing`, and `pitch` are standard Mapbox camera fields.
- `extensions.hover` and `extensions.legend` are optional Open Web additions.

Do not wrap these fields in `style`. The renderer calls `map.addSource` and `map.addLayer`; it
does not call `map.setStyle`.

### Sources

Each source is keyed by its source ID and must have `type:"geojson"`. Use exactly one:

- `data`: direct standard GeoJSON for inline data. This source remains valid Mapbox JSON.
- `data_ref`: the complete object returned by a `map_utils` data Tool.

```json
{
  "sources": {
    "routes": {
      "type": "geojson",
      "data_ref": {
        "type": "mcp_resource",
        "server": "map_utils",
        "uri": "maps-data://geojson/map-data-...",
        "format": "geojson"
      },
      "lineMetrics": true
    }
  }
}
```

Standard GeoJSON source options such as clustering, `generateId`, `promoteId`, `lineMetrics`,
buffer, and tolerance may be used when the current Mapbox Style Specification permits them.

Do not:

- put a Resource object in `data`;
- wrap inline GeoJSON in `{type:"inline",geojson:...}`;
- use source URLs, tiles, credentials, or non-GeoJSON source types;
- invent or rewrite the returned `server` or `uri`.

Even a single point should keep the Tool-returned `data_ref`. Direct coordinates supplied by the
user may use small inline GeoJSON.

### Layers

Write `layers` exactly as standard Mapbox Layer JSON. All layer types, `paint`, `layout`,
filters, expressions, `minzoom`, `maxzoom`, `source-layer`, metadata, and other properties
supported by the installed official Style Specification validator are allowed.

```json
{
  "layers": [
    {
      "id": "routes",
      "type": "line",
      "source": "routes",
      "layout": {"line-cap": "round", "line-join": "round"},
      "paint": {
        "line-color": [
          "interpolate",
          ["linear"],
          ["zoom"],
          4,
          "#2563eb",
          12,
          "#ef4444"
        ],
        "line-width": 4
      }
    }
  ]
}
```

The Tool uses Mapbox's official Style Specification validator. Unknown Style properties produce
warnings; invalid known syntax fails. Do not convert Mapbox fields to Open Web shorthands such as
`geometry`, `style`, `label-property`, or layer-local `hover`.

### Camera

Provide `center` and `zoom` together for an explicit camera:

```json
{"center":[114.0579,22.5431],"zoom":8,"bearing":0,"pitch":0}
```

Omit both `center` and `zoom` to fit all loaded GeoJSON. Do not use the removed `view` object.

### Optional extensions

Hover is opt-in and text-only. It references existing Mapbox layer IDs:

```json
{
  "extensions": {
    "hover": {
      "layers": [
        {
          "layer": "routes",
          "title_property": "label",
          "fields": ["distance", {"property":"duration","label":"时长"}]
        }
      ]
    }
  }
}
```

Legend is also opt-in and is not part of Mapbox Style:

```json
{
  "extensions": {
    "legend": {
      "title": "图例",
      "items": [
        {"label":"城市","color":"#e11d48","type":"circle"},
        {"label":"路线","color":"#2563eb","type":"line"},
        {"label":"区域","color":"#0891b2","type":"fill"}
      ]
    }
  }
}
```

Legend item `type` is optional and may be `circle`, `line`, or `fill`. Omit hover or legend when
not needed. Never put either extension inside a Mapbox layer, and never include HTML or
JavaScript.

## Complete call shape

```json
{
  "title": "深圳到广东各地级市驾车路线",
  "intent": "展示驾车路线",
  "sources": {
    "routes": {
      "type": "geojson",
      "data_ref": {
        "type": "mcp_resource",
        "server": "map_utils",
        "uri": "maps-data://geojson/map-data-...",
        "format": "geojson"
      }
    }
  },
  "layers": [
    {
      "id": "routes",
      "type": "line",
      "source": "routes",
      "paint": {"line-color":"#2563eb","line-width":4}
    }
  ],
  "extensions": {
    "legend": {
      "items": [{"label":"驾车路线","color":"#2563eb","type":"line"}]
    }
  }
}
```

Never reproduce the renderer payload, Artifact envelope, Resource contents, local paths, or
credentials in the Assistant response.
