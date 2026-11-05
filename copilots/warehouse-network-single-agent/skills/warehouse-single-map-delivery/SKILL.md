---
name: warehouse-single-map-delivery
description: 单 Agent 把已完成的仓网结果制作成地图或报告。
metadata:
  short-description: 单 Agent 地图与交付
---

# 单 Agent 地图与交付

地图只展示规划工具已经确认的结果，不重新计算路线、成本、分配或达标率。常见的“画出来”步骤在 `$warehouse-single-agent` 中；本文件补充报告、修改已有地图和行政区边界。

- MCP server ID 使用工具返回的原始值；地图服务为 `map_utils`，规划服务为 `supply_chain`。`warehouse-network-maps` 是包 ID，不是 MCP server ID。
- 已有覆盖资源仅在输入、分配结果和用户要求的时限都一致时复用；否则重新准备。不改写地图 spec，不自行补造达标率摘要。
- 根据用户目标选择仓网分布图、时效覆盖图或方案对比图。每种地图都是两步：先用 `supply_chain` 的准备工具生成地图数据，再把返回的 `data_ref` 原样作为 `network_data_ref` 传给 `map_utils/create_network_map_card`。
  - 仓网分布图（展示仓库、站点、需求城市或候选仓的位置，不需要时效或成本结果）：用 `prepare_network_distribution_map`，传入准备好的数据相对路径；用户要看候选仓时设 `include_candidates=true`。
  - 时效覆盖图：用 `prepare_network_coverage_map` 按用户要求的服务时限准备；按规划结果中的“达标、未达标、未分配”分别展示需求城市和覆盖线路。
  - 方案对比图：用 `prepare_network_comparison_map` 基于已完成的方案比较结果准备。
- `publish_workspace_geojson` 只用于用户自己上传的行政区边界 GeoJSON：设 `require_polygon=true` 发布后，作为 `boundary_data_ref` 传给 `create_network_map_card`。仓库、站点、需求城市和覆盖线路的地图数据只来自上述准备工具；准备好的规划数据、地图卡片规格和其他工具输出都不是 GeoJSON，不能交给它发布。
- 用户没有指定样式时，清楚区分中心仓、越库仓（XD）、候选仓、启用/关闭状态和时效结果。
- 不调用通用地图构建器，不自行拼接 GeoJSON 或图层。
- `create_network_map_card` 成功后立即返回业务摘要和完整 embed 段落，不再调用 `publish_workspace_geojson`、另一张地图、报告或其他工具。只有用户明确要求报告时才生成报告。
- 修改已有地图只使用用户明确选中的 `map_spec_ref`；缺少或失效时请用户重新选择，不猜测旧地图。
