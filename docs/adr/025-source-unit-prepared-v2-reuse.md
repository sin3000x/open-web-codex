# ADR-025：Source Unit 选择与 Prepared v2 新鲜复用

状态：Accepted
日期：2026-08-21

补充并局部替代 [ADR-023](023-workspace-prepared-inputs-and-domain-geospatial-execution.md) 与
[ADR-024](024-warehouse-copilot-contract-simplification.md) 中 Data 检查、准备输入和复用的对应段落。
Runtime、Platform、Workspace 授权、MCP Resource、Artifact 和 Role 权限所有权不变。

## 背景

Data 需要支持 CSV、XLSX sheet 和任意 JSON array，同时避免把 preview 当成完整数据，也不能让
未选来源的弱角色评估阻塞任务。旧检查握手和 prepared 输入还无法表达精确 unit、完整映射和来源
新鲜度，导致 Agent 重复检查或错误复用。

## 决定

1. Data 按用户目标选择 `required_roles`（仅 `demand`、`existing_warehouse`、`candidate_warehouse`、`current_assignment`、`route_quote`），inspect 为每个 exact source unit 返回有界字段、样本类型、代表值、preview/total 和角色评估。国家已知时可显式传入；未知时先 inspect，唯一 admin catalog 的 top-level `country_code` 才能提供派生值，缺失/冲突返回 null 并询问。行政区 catalog 只作为 inspected relative path 与 prepare 参数，不是 planning role。角色评估不是全局业务缺口；只有选中 unit 的语义、单位、值或冲突无法确定时才询问。
2. `workspace_source_profile.v2` 的 inspect 结果只在当前握手中使用。`prepared_network_input.v2` 是 Data→Network 的唯一持久化业务文件；它保存选中 unit、resolved mappings、raw/admin SHA、国家、roles、role_counts 和一个 `selected_source_identity`。
3. inspect 只有在调用方显式传入 exact prepared candidate path，且候选为 ready、国家/角色匹配、选中来源和映射仍 fresh 时才复用。相同 prepared 内容按规范路径稳定选择；不同内容返回一次 `prepared_selection_required`；未选来源变化不影响复用。
4. prepare 在完整读取前后校验 source snapshot；`ready` 写 create-new 文件，`needs_input` no-write 一次交接，`source_changed` 只允许一次重新 inspect。Network/Maps 只接受 v2 ready 输入，直接加载 prepared snapshot 不依赖 raw 文件；复用和 geography 准备另行校验 raw/admin 新鲜度。

## 修订（2026-11-03）：检查与复用拆分

第 1、3 点中 inspect 同时承担 prepared 复用的部分被替代。`inspect_workspace_sources` 只检查原始来源，
不再接收 `required_roles`，传入 prepared 路径时明确失败；unit profile 只保留 `role_assessments`，不再
重复输出 `mapping_suggestions`。prepared 复用由只读 `resolve_prepared_network_input` 负责：调用方仍须
显式给出 discover 列出的 prepared 路径和本次 `required_roles`，新鲜度校验不变；新鲜文件按所需 roles 的
规范化记录（及国家）分组，记录相同即视为对本次目标等价，复用 roles 最多、其次路径最小的文件并报告
`equivalent_count`；只有记录不同才返回 `prepared_selection_required`；没有新鲜文件返回 typed
`prepared_unavailable`。比较记录而非 source selection，是因为规范化跨 role 校验、地理补全会改写记录，
记录才是权威数据。

同日，source profile 升级为 `workspace_source_profile.v3`：不再返回 head preview 行（此前截到 3 行，类型和代表值
只来自前 20 行），改为对每个 unit 的全部记录做一次流式统计，每字段给出类型计数、空值、去重计数（超过 256 个
跟踪值即标为非精确）、全数值字段的 min/max，以及重复的低基数字段的取值计数或至多 3 个代表值。统计仅用于判断
字段含义和映射；业务事实仍由准备后的 `query_prepared_network_input` 回答。

## Owner、持久化与边界

Data Tool 拥有 unit 解析、全量读取、映射、规范化和 prepared 文件；Network/Maps 拥有其 typed Resource 和交付。Platform 不保存映射、复用 registry、revision、fingerprint 或 workflow state；Codex Runtime 不做领域解释。Role TOML 继续显式声明 MCP 权限，Skill 只描述目标角色、业务询问和 typed 终态交接。

## 被否决方案

- 用文件名、preview、国家或历史 child 状态猜角色和“最新”来源；
- 为 source profile 建立 Resource、Platform 状态机或跨任务缓存；
- 看到任意 prepared 文件就自动复用，或让未选来源变化阻塞当前目标；
- 在 Skill 中重复 Tool 已强制的路径、schema、hash、create-new 和安全规则。

## 验证与退出条件

Planner/Maps unit、schema、stdio、freshness、source-change、XLSX/JSON unit、非标准表头和真实/确定性
Copilot 门验证上述合同。若未来官方 Runtime/SDK 提供等价的 typed source-unit/provenance 能力，在
保留 Data/Network owner 和失败语义后删除领域重复实现；此前不增加兼容读取或第二套复用机制。
