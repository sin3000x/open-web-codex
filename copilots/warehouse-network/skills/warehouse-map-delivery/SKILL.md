---
name: warehouse-map-delivery
description: 仅供 network_agent 使用；把已完成的仓网结果制作成地图或报告。
metadata:
  short-description: 仓网地图与交付
---

# 仓网地图与交付

地图只展示规划工具已经确认的结果，不重新计算路线、成本、分配或达标率。

- 开始制图前，先检查已发布的 `.codex/map-card-specs/` JSON 与当前对话中的地图交付结果。若已有与用户目标、规划结果和服务时限一致的地图卡片（例如标题含“12小时”“时效”“覆盖”或“SLA”，且图层按 `service_status` 区分 `attained`、`missed`、`unassigned`），直接展示已有的 embed code 和业务摘要；若只有可复用的 spec 而没有当前可用的 embed，使用该 spec 的来源重建地图卡片，不重跑规划或地图数据准备。

- 根据用户目标选择仓网分布图、时效覆盖图或方案对比图。
- 时效覆盖图先用 `prepare_network_coverage_map` 按用户要求的服务时限准备，再用 `create_network_map_card` 交付；按规划结果中的“达标、未达标、未分配”分别展示需求城市和覆盖线路。
- 方案对比图先用 `prepare_network_comparison_map`，且 `service_target_hours` 必须从该比较结果的 `requested_service_targets` 中选择一个确切目标，再用 `create_network_map_card` 交付。
- 用户没有指定样式时，清楚区分中心仓、越库仓（XD）、候选仓、启用/关闭状态和时效结果。
- 使用规划工具交付的完整结果调用对应的仓网地图工具，不调用通用地图构建器，也不自行拼接 GeoJSON 或图层。
- `create_network_map_card` 成功后立即返回业务摘要和完整 embed 段落，不再调用 `publish_workspace_geojson`、另一张地图、报告或其他工具。只有用户明确要求报告时才生成报告。
- 修改已有地图只使用用户明确选中的 `map_spec_ref`；缺少或失效时请用户重新选择，不猜测旧地图。
- 同一工具连续失败 2 次后停止重试，先读工具源码或契约文档确认错误码语义，再决定下一步；不要换路径或参数盲试。
