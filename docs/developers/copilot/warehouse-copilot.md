# 开发仓网 Copilot

> **适合谁**：准备维护仓网参考实现，或要把单 Agent 方案演进为明确多 Agent 分工的开发者
> **预计时间**：45 分钟阅读与静态检查；真实三轮 Web 验收另计
> **前置条件**：先完成[第一个 Copilot](first-copilot.md)，并能启动本地 Web
> **完成结果**：能复用根级 Planner/Maps，分别维护单 Agent 与多 Agent 包，并完成 12h → Balikpapan → 90% 三轮地图验收

仓网参考实现不是新人模板的“大号复制品”。它已经把算法放在两个根级共享 Tool 包中：

```text
tools/warehouse-network-planner/   # 数据准备、路线、时效、成本、场景和选址
tools/warehouse-network-maps/      # 地图卡片创建与修订
```

两个 Copilot 只装配这些能力，不复制算法：

```text
copilots/warehouse-network-single-agent/  # 先看：一个 Root 独立完成
copilots/warehouse-network/               # 再看：Root + Data + Network
```

## 1. 先检查共享 Tool，再看 Copilot

根级 Tool 使用严格 `tool.toml` 和 `runtime.toml`。仓库包装器已经把共享注册表固定为 `tools/`，
不要再传 `--tool-registry-root`，也不要把 Planner 或 Maps 复制进 `copilots/`。

先验证两个现有包：

```bash
./scripts/copilot.sh validate ./copilots/warehouse-network-single-agent
./scripts/copilot.sh validate ./copilots/warehouse-network
```

期望摘要：

```text
Copilot package 'warehouse-network-single-agent' is valid.
  skills: 4
  agents: 1
  tools: 2

Copilot package 'warehouse-network-copilot' is valid.
  skills: 4
  agents: 3
  tools: 2
```

成功信号：两包都通过，并各自只声明 `supply_chain` 与 `map_utils` 两个共享 Tool。失败时先看
[共享 Tool 找不到](testing-and-troubleshooting.md#共享-tool-找不到)。

## 2. 先建立独立的单 Agent 包

一个 Copilot 只有一个 Root。单 Agent 不是多 Agent 包里的 mode，而是独立包
`warehouse-network-single-agent`。它的 `copilot.toml` 必须具备下面的精确关系：

```toml
schema_version = 1
id = "warehouse-network-single-agent"
display_name = "Warehouse Network · Single Agent"

[root]
skill = "warehouse-single-agent"
agent = "warehouse_single_agent"
task_skills = "all"

[[skills]]
id = "warehouse-single-agent"
path = "skills/warehouse-single-agent"

[[skills]]
id = "warehouse-data-preparation"
path = "skills/warehouse-data-preparation"

[[skills]]
id = "warehouse-single-network-planning"
path = "skills/warehouse-single-network-planning"

[[skills]]
id = "warehouse-single-map-delivery"
path = "skills/warehouse-single-map-delivery"

[[agents]]
id = "warehouse_single_agent"
role = "agents/warehouse_single_agent.toml"

[[tools]]
id = "supply_chain"
package = "warehouse-network-planner"

[[tools]]
id = "map_utils"
package = "warehouse-network-maps"
```

完整当前文件以
[`copilots/warehouse-network-single-agent/copilot.toml`](../../../copilots/warehouse-network-single-agent/copilot.toml)
为准。`package` 由根级共享注册表解析；不要改成 `root = "../..."`，也不要手写 Tool 启动命令。

单 Agent Role
[`warehouse_single_agent.toml`](../../../copilots/warehouse-network-single-agent/agents/warehouse_single_agent.toml)
同时启用四项 Skill、`supply_chain_data`、`supply_chain` 和 `map_utils`，并显式
`multi_agent = false`。它可以独立完成一条仓网任务，但不创建 child Agent。Root Skill
`warehouse-single-agent` 按问题类型写明固定调用顺序和关键参数，常见问题只靠它完成；其余三项
Skill 只在首次准备数据、少见的路线/成本补齐分支或报告与改图时读取。

成功信号：业务计算仍只在共享 Tool；单 Agent 包只拥有方法、权限和装配。若只是为了省事把
Tool 代码复制进包，当前步骤没有通过。

## 3. 再建立独立的多 Agent 包

多 Agent 包 `warehouse-network-copilot` 当前是 **4 Skills / 3 Agents**：

| Agent | 启用的 Skill | Tool 权限 | 明确不做 |
| --- | --- | --- | --- |
| `warehouse_supervisor_root` | `warehouse-supervisor` | 无仓网 MCP | 不读文件、不计算、不画图 |
| `data_agent` | `warehouse-data` | `supply_chain_data` | 不算时效、成本、选址或地图 |
| `network_agent` | `warehouse-network-planning`、`warehouse-map-delivery` | `supply_chain`、`map_utils` | 不重读原始文件 |

关键 manifest 关系：

```toml
[root]
skill = "warehouse-supervisor"
task_skills = "all"
agent = "warehouse_supervisor_root"

[[agents]]
id = "warehouse_supervisor_root"
role = "agents/warehouse_supervisor_root.toml"

[[agents]]
id = "data_agent"
role = "agents/data_agent.toml"

[[agents]]
id = "network_agent"
role = "agents/network_agent.toml"

[[tools]]
id = "supply_chain"
package = "warehouse-network-planner"

[[tools]]
id = "map_utils"
package = "warehouse-network-maps"
```

完整当前文件以
[`copilots/warehouse-network/copilot.toml`](../../../copilots/warehouse-network/copilot.toml)
为准。Root Role 无 `[plugins]` 能力；不要为了“兜底”把全部 Tool 给 Root。Data 和 Network 使用
Codex 原生协作，平台不建立第二套 Agent 消息或结果通道。

成功信号：Root 只协调；Data 只准备；Network 只分析和交付；三者都没有复制 Planner/Maps 算法。
Role 引用失败时见 [Role 找不到 Skill、Tool 或 server](testing-and-troubleshooting.md#role-找不到-skilltool-或-server)。

## 4. 精确声明浏览器交付

中间 Resource 仍由 MCP provider 拥有；只有明确的最终报告或地图进入 delivery。两个仓网 manifest
都声明四类当前交付：

| delivery ID | producer | kind | 用途 |
| --- | --- | --- | --- |
| `network-planning-report` | `supply_chain.publish_network_planning_report` | `workspace_artifact` | 最终 Markdown 报告 |
| `network-map-card` | `map_utils.create_network_map_card` | `inline_geojson_map_card` | 标准仓网地图 |
| `network-map-card-custom` | `map_utils.create_map_card` | `inline_geojson_map_card` | 用户明确要求的自定义地图 |
| `network-map-card-revision` | `map_utils.revise_map_card` | `inline_geojson_map_card` | 已有地图的样式修订 |

标准地图声明必须精确写 producer 和媒体类型：

```toml
[[deliveries]]
id = "network-map-card"
server = "map_utils"
tool = "create_network_map_card"
kind = "inline_geojson_map_card"
schema = "map.v3"
mime_type = "application/vnd.open-web-codex.map-card+json"
display_name = "Warehouse network map"
```

Platform 不会从最终文字、Tool 显示名或普通 Resource 猜测交付。修改 producer 时，要在同一改动中
更新 manifest、Role allowlist、schema/fixture 和 Web 验收。详情见[交付与 UI](advanced/deliveries-and-ui.md)。

## 5. 逐包完成本地门

先跑单 Agent，再跑多 Agent；这样失败时能区分“共享 Tool/业务合同”与“协作拓扑”：

```bash
./scripts/copilot.sh check ./copilots/warehouse-network-single-agent \
  --workspace "$PWD"

./scripts/copilot.sh check ./copilots/warehouse-network \
  --workspace "$PWD"
```

两个命令都应输出 `passed validate, prepare, dev, and test`。当前 manifest 的本地测试是小而确定的
正常链，不代替真实模型和三轮业务验收。

需要把修改装入本地服务时，分别同步你实际修改的包：

```bash
./scripts/copilot.sh sync ./copilots/warehouse-network-single-agent \
  --workspace "$PWD"

./scripts/copilot.sh sync ./copilots/warehouse-network \
  --workspace "$PWD"
```

`sync` 每次都会先跑完整 `check`，再冷重启服务。成功信号是包 ID 与健康重启都被确认；失败见
[同步失败](testing-and-troubleshooting.md#同步失败)。

## 6. 做三轮真实 Web 地图验收

在一个新的 Workspace 上传
[`apps/web/scripts/fixtures/warehouse-network/mock_data/`](../../../apps/web/scripts/fixtures/warehouse-network/mock_data/)
中的六个业务文件，不上传 `manifest.json`。新建 Task，明确选择要验收的单 Agent 或多 Agent 包。
在**同一个 Task**按顺序发送三轮：

### 第一轮：12 小时基准

```text
根据我上传的文件计算 12 小时时效达标率并绘制地图。
```

验收：结构化结果同时包含 12 小时的城市达标率与需求量加权达标率；地图来自已计算分配，出现
标准 `create_network_map_card` 交付。

### 第二轮：只新增 Balikpapan

```text
只新增候选仓 Balikpapan，评估 12 小时时效达标率相对上一轮的变化，并绘制对比地图。
```

验收：`assess_facility_change` 的 addition 精确是 `WH-CANDIDATE-BALIKPAPAN`，策略是
`min_time`，没有 removal 或 relocation；结果同时给出城市和需求量加权变化，随后产生新的地图。
不要只凭回答文字出现 “Balikpapan” 判定通过。

### 第三轮：90% 的最少新增仓

```text
在 12 小时需求量加权达标率达到 90% 的前提下，找出最少的新增仓库数量和选址结果，并用地图展示。
```

验收：结果明确说明是否可达；可达时给出经 Tool 验证的最少新增数、精确选址和达标率，并生成
第三张地图；不可达时明确失败，不能伪造一个 90% 方案。

三轮共同成功信号：每轮都有结构化业务结果和独立地图卡片；第二、三轮复用同一 Task 中已经确认
的数据，而不是重读预览或从模型文字重建。页面刷新后仍能从正式 Task 历史看到三轮结果。

失败时先按现象进入 [Web 与真实业务验收](testing-and-troubleshooting.md#web-与真实业务验收)，
再判断 owner，不能用 Prompt 强制固定调用顺序来掩盖问题。

## 完成检查

- [ ] Planner 与 Maps 只保留在根级 `tools/`，两个 Copilot 都通过 registry 的 `package` 引用；
- [ ] 单 Agent 与多 Agent 是两个独立包，各自只有一个 Root；
- [ ] 多 Agent 是 4 Skills / 3 Agents，Root 没有仓网 MCP；
- [ ] 报告和地图 producer 有精确 delivery 声明；
- [ ] 两包 `check` 通过；
- [ ] 12h → Balikpapan → 90% 三轮都有真实结构化结果和地图。
