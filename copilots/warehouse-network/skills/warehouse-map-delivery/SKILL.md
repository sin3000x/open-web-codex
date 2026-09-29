---
name: warehouse-map-delivery
description: 仅供 network_agent 使用；把已完成的仓网结果制作成地图或报告。
metadata:
  short-description: 仓网地图与交付
---

# 仓网地图与交付

地图只展示规划工具已经确认的结果，不重新计算路线、成本、分配或达标率。

- 制图前先核对当前任务交接和对话中已有的地图交付结果。如果已有精确的 `map_spec_ref`，并能从对应规划结果确认它使用同一输入、同一分配结果及用户要求的服务时限，调用 `present_map_card` 重新交付该 spec，再返回新生成的 embed code 和业务摘要。不要凭标题关键词猜测，也不要扫描 provider 私有目录或复制先前 Turn 的 embed code。无法确认精确匹配时才按下列流程准备新地图。

- 根据用户目标选择仓网分布图、时效覆盖图或方案对比图。
- 时效覆盖图先用 `prepare_network_coverage_map` 按用户要求的服务时限准备，再用 `create_network_map_card` 交付；按规划结果中的“达标、未达标、未分配”分别展示需求城市和覆盖线路。
- 用户没有指定样式时，清楚区分中心仓、越库仓（XD）、候选仓、启用/关闭状态和时效结果。
- 使用规划工具交付的完整结果调用对应的仓网地图工具，不调用通用地图构建器，也不自行拼接 GeoJSON 或图层。
- `create_network_map_card` 或 `present_map_card` 成功后立即返回业务摘要和本次结果的完整 embed 段落，不再调用 `publish_workspace_geojson`、另一张地图、报告或其他工具。只有用户明确要求报告时才生成报告。
- 修改已有地图只使用用户明确选中的 `map_spec_ref`；缺少或失效时请用户重新选择，不猜测旧地图。
- 同一工具连续失败 2 次后停止重试，先读工具源码或契约文档确认错误码语义，再决定下一步；不要换路径或参数盲试。
