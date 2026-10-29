---
name: warehouse-map-delivery
description: 仅供 network_agent 使用；把已完成的仓网结果制作成地图或报告。
metadata:
  short-description: 仓网地图与交付
---

# 仓网地图与交付

地图只展示规划工具已经确认的结果，不重新计算路线、成本、分配或达标率。

- 开始交付前确认所需领域工具可调用；缺少时只通过 `tool_search` 按准确工具名发现一次。没有搜索入口或搜索后仍不可用，报告 `capability_unavailable` 并停止；不得用 shell/Python、源码检查或资源枚举寻找替代调用入口。
- MCP server ID 使用工具返回的原始值；地图服务为 `map_utils`，规划服务为 `supply_chain`。`warehouse-network-maps` 是包 ID，不是 MCP server ID。
- 不扫描 provider 私有文件或按标题猜测旧地图。已有覆盖资源仅在输入、分配结果和用户要求的时限都能确认一致时复用；否则重新准备覆盖数据。不改写地图 spec，不自行计算或补造达标率摘要。
- 相同工具与参数失败后不得原样重试；权限、工作区范围或能力缺失错误立即停止并报告，不通过换路径、换 server ID 或写新位置绕过。
- 根据用户目标选择仓网分布图、时效覆盖图或方案对比图。每种地图都是两步：先用 `supply_chain` 的准备工具生成地图数据，再把返回的 `data_ref` 原样作为 `network_data_ref` 传给 `map_utils/create_network_map_card`。
  - 仓网分布图（展示仓库、站点、需求城市或候选仓的位置，不需要时效或成本结果）：用 `prepare_network_distribution_map`，传入准备好的数据相对路径；用户要看候选仓时设 `include_candidates=true`。
  - 时效覆盖图：用 `prepare_network_coverage_map` 按用户要求的服务时限准备；按规划结果中的“达标、未达标、未分配”分别展示需求城市和覆盖线路。
  - 方案对比图：用 `prepare_network_comparison_map` 基于已完成的方案比较结果准备。
- `publish_workspace_geojson` 只用于用户自己上传的行政区边界 GeoJSON：设 `require_polygon=true` 发布后，作为 `boundary_data_ref` 传给 `create_network_map_card`。仓库、站点、需求城市和覆盖线路的地图数据只来自上述准备工具；准备好的规划数据、地图卡片规格和其他工具输出都不是 GeoJSON，不能交给它发布。
- 用户没有指定样式时，清楚区分中心仓、越库仓（XD）、候选仓、启用/关闭状态和时效结果。
- 使用规划工具交付的完整结果调用对应的仓网地图工具，不调用通用地图构建器，也不自行拼接 GeoJSON 或图层。
- `create_network_map_card` 成功后立即返回业务摘要和完整 embed 段落，不再调用 `publish_workspace_geojson`、另一张地图、报告或其他工具。只有用户明确要求报告时才生成报告。
- 修改已有地图只使用用户明确选中的 `map_spec_ref`；缺少或失效时请用户重新选择，不猜测旧地图。
