# Capability Baseline

| 字段 | 内容 |
| --- | --- |
| 文档性质 | 当前事实与证据 |
| 观察日期 | 2026-08-23 |
| 代码快照 | ADR-025 source-unit/prepared v2 合同；真实 Web + DeepSeek Flash 三段业务验收已通过 |
| 当前阶段 | 阶段二已进入；阶段一仓网 Copilot 正常业务闭环作为已通过基线 |
| 接受基线 | [ADR-018](adr/018-built-in-network-copilot-runtime-closure.md) + [ADR-019](adr/019-task-selected-copilot-packages-and-shared-tools.md) + [ADR-024](adr/024-warehouse-copilot-contract-simplification.md) + [ADR-025](adr/025-source-unit-prepared-v2-reuse.md) |

本文回答“当前构建能证明什么”。源码存在、局部测试通过、真实 Runtime 运行和从阶段一
业务 Task/Web 开始的产品 E2E 是不同证据，不能互相替代。

## 1. 结论

停止控制路径新增 pending read 并发、计划重启不影响中断、跨 Workspace/缺失 Thread
拒绝和 stale Runtime request 回归测试。当前修复环境缺少 Cargo，测试尚未运行；真实
Provider 思考期间点击停止的产品 E2E 也尚未验证，不能据此宣称中断 hardening 已通过。

当前 checkout 已从 clean DB/Profile、真实 Web Task 入口、真实 Codex Runtime 和真实 Provider
完成多 Agent 仓网 Copilot 的阶段一正常业务闭环。这个结论不代表 Web Studio、Marketplace、
多用户产品或完整 hardening 矩阵已经完成。独立单 Agent 包已有静态组合、共享 Tool、package-keyed
Root config 和 Web 显式选择的 E1/E2 证据；本轮将真实 DeepSeek 入口拆为多 Agent 业务、单 Agent
业务和独立 Tool capability 三个门，并将成本证据收敛为 typed schema、选址收敛为
`minimum_feasible` 有界求解。此前 ADR-024 的自然语言多 Agent 与单 Agent 业务门曾用
`deepseek-v4-flash` 完成真实付费复跑。2026-08-22 又从 Web 新建 Workspace、上传 Downloads 的
6 个 mock data 文件，并在同一多 Agent Task 中完成 ADR-025 的 source-unit/prepared v2 三段验收；
该证据仍不扩大成 clean Profile E4。

阶段二 Copilot SDK 已提供 `copilot init` 源码脚手架和 `copilot validate` 静态组合
验证；`copilot dev` 提供隔离 discovery probe：通过官方 app-server 握手、
`skills/list`、带 `selectedCapabilityRoots` 的临时 `thread/start` 和线程范围
`mcpServerStatus/list` 验证声明能力可被 Runtime 发现。`copilot test` 另以本地确定性 Responses
fixture 驱动真实 app-server；每个用例使用独立临时 Profile 和 durable Thread，支持 Root 直调
或声明 child Role 两种 target，并从 `thread/read(includeTurns=true)` 的 canonical history 验证
精确 MCP Tool 参数/结构化结果、Role/parent 与 terminal 顺序。当前 repo Codex 已分别通过
meeting child 和仓网 single-Agent Root 两种真实正常链；PASS 不读取最终文本或模型请求体。
它没有生产模型质量验收、Web 创作链、Catalog 或 Marketplace。Platform 现在从显式可信
Copilot 根发现所有一级 package，并以 `(profile, package)` 持久 desired/configured/failure；
Browser 创建 Thread 时列出可用 package 并只提交所选 ID。Profile Host 在 Runtime 启动前收敛
全部 active 包的 Skill/child Role，每个包生成一个 package-keyed Root execution config；Copilot
installation 不产生或持久 `Ready`。当前 Tool 环境合同已收敛到根级共享 registry：`[[tools]]` 可按 package 引用严格
`tool.toml`，Tool 的 `runtime.toml`
source 只保留 Python/Node 项目 manifest、hash lock、server entry/env binding 声明与领域代码。
SDK 在 Profile、Tool source 和 Workspace 外执行 bounded build/install，生成内部
`prepared-tools.v1.json` 与 dev/test 使用的一次性 Plugin 投影；Tool 不再提供 setup、launcher
或 source `.mcp.json`/`.codex-plugin` transport。平台本地 real 启动只调用一次同一
`copilot prepare`，Server 泛化消费 descriptor 并按 Profile 解析 typed 绑定；Runtime launch
和用户对话期间不安装依赖。`dev`/`test` 默认复用稳定的 SDK Tool 环境缓存，组合提示变化不触发
未变化依赖的重装。SDK parser/provisioner、生成骨架、dev/test 和平台消费各自仍以本
工作树的 focused/full gate 为证据，不把它扩大为生产安装、Marketplace 或多用户能力。

当前证据支持：

> 用户从 Web 创建 Workspace/Task 后，仓网参考 Copilot 可以通过 Codex 原生 Root、Data Agent、
> Network Agent、typed MCP Resource、普通 Workspace 文件和最终 Artifact，完成数据准备、规划、
> 场景复用、地图卡片与中文 Markdown 正式简报。

2026-08-23 又完成开发者 SDK 真实验收：fresh single/multi package 的完整 `check` 均通过；临时
`record-review` 经真实 `sync`、Web 选择和 DeepSeek Flash Tool 调用完成后，再经 Web 停用、源码
移除和冷启动收敛为 `Unavailable/Inactive`，managed Skill/Role 为空。随后同一仓网多 Agent Task
完成 12h 基线（2:27，需求加权 81.5%、城市 66.0%）、Balikpapan（2:45，86.5%、+5.0pp）和
90% 最少新增仓（2:49，Balikpapan + Jambi、90.1%）三轮，地图均 Ready。Agent 拓扑始终为
Root/Data/Network；5 个 typed Tool 失败和 2 次模型参数构造错误均在原 Task/child 内恢复，未出现
unknown Tool、stream disconnected、权限扩大或浏览器控制台错误。

旧 Work State、SourceAsset、Case、ArtifactRef、冻结能力包和 Prompt 接线已经删除，不是当前
合同。当前单一分工是 Codex 原生协作、MCP Resource、Workspace 普通文件和显式最终 Artifact。

不能声称：

> 算法工程师已经可以只用正式 SDK 和 Web，从零创建、发布、安装、发现并运行
> Tool、Skill、Agent、Supervisor 和 Copilot。

2026-08-21 的 source-unit/prepared v2 gate 已通过：CSV、multi-sheet XLSX、nested JSON array、中文/随机表头、preview/total、selected raw/admin freshness、fresh prepared reuse、不同候选内容的选择分支和 geography replacement 均由 Planner/Maps/stdio 测试覆盖。复用只接受 inspect 输入中显式出现的 candidate；未选来源变化不阻塞当前目标，`needs_input` 和 `source_changed` 都是有界终态。

产品成熟度应分开判断：

| 维度 | 当前等级 |
| --- | --- |
| 真实多 Agent 运行 | 多 Agent 仓网包的阶段一 normal path 已通过真实 Web E2E |
| 单 Agent 仓网运行 | 独立 package、Root config、共享 Tool 与 Web 选择合同已通过静态/focused gate；自然语言真实门已完成 580 条报价 evidence 绑定与 `minimum_feasible` 90% 覆盖，clean Profile E4 未运行 |
| Root 用户输入与 execution projection | 阶段一 normal path 可用；pending approval 刷新恢复已通过 |
| Tool/Skill SDK 与 Studio | Copilot 源码 `init`/`validate`、通用环境 `prepare`、隔离 `dev` 与本地 fixture `test` 存在；旧 Web Studio 已删除 |
| Agent/Supervisor 创作 | 旧 Web authoring 已删除；阶段一仅直接编辑 Profile Skill/Role |
| Copilot 编译、Profile 安装和通用 Runtime discovery | 可信根一级目录发现、多 package 持久安装、停用与冷启动收敛已实现；当前 Runtime Skill discovery 即时验证，child Role spawn 仍由独立真实 gate 证明 |
| 算法工程师自助扩展 | 未实现 |
| 第二领域与多用户隔离 | `meeting-action-review` 已通过本地第二领域组合验证并加入应用 trusted source；通用单 Profile 安装的真实 PostgreSQL lifecycle 已通过，多用户隔离未验证 |

## 2. 证据等级

| 等级 | 含义 |
| --- | --- |
| E0 | 只有目标文档、类型或源码骨架 |
| E1 | 单元/静态合同通过，但没有跨 owner 边界 |
| E2 | 服务或集成测试通过，仍可使用预置对象/Fake |
| E3 | 真实 Provider + 真实 Codex Runtime 的受控纵向运行 |
| E4 | 从 clean DB/Profile 的真实业务 Task 入口完成阶段一 normal path，并通过一次关键 pending approval 刷新恢复 |

ADR-023/024/025 已将 Data→Network 输入、source-unit 选择、prepared provenance、导航执行和地图装配合同切换到新的 Tool 实现。ADR-025 当前已有
Planner/Maps、stdio、确定性 Copilot 和真实 Web + DeepSeek Flash 证据。下方旧 E4 记录适用于被替代的 Data Resource/手工导航路径，不能作为当前
合同的 E4 声明。单 Agent 与多 Agent 仍需按新的 Workspace 输入、导航确认、地图和报告路径持续复验。多用户、完整失败/竞态矩阵、a11y 和视觉细节仍属于后续 hardening。

### 2026-08-22 ADR-025 真实 Web 三段验收

Root 在 Web 新建 `DeepSeek Flash 仓网验收` Workspace，从 Downloads/mock_data 上传 6 个业务文件，
选择 `warehouse-network-copilot`、DeepSeek E2E Provider、`deepseek-v4-flash`，并确认 Tool search 开启。
三段业务在同一 Task 中完成，地图均在 Web 实际显示为 `Ready`，浏览器控制台无错误：

- 12h 基线：回答 2 分 35 秒，地图 Ready 约 2 分 51 秒；城市口径 66.0%（33/50），需求加权
  81.5%（41,399/50,815），580 条路线事实形成 556 对、0 缺失，地图 117 个要素。
- 新增 Balikpapan：回答 4 分 52 秒，地图 Ready 约 5 分 33 秒；`WH-CANDIDATE-BALIKPAPAN`
  已纳入 prepared input，城市与需求加权口径均为 0.0pp 变化。上传数据缺少候选仓路线，结果明确
  报告 51 对缺失而没有伪造参数；场景地图 130 个要素并高亮新增启用仓。
- 90% 最少仓数：首次 2 分 37 秒返回一次 `needs_input`；确认需求加权口径、haversine 1.3/40kph
  与完整报价 `observed_quote_mean` 后，最终回答 2 分 25 秒，端到端约 6 分 41 秒。最小方案为
  13 仓，即现有 11 仓加 Balikpapan、Jambi；最少新增仓数 2 已证明，12h 需求加权达标率
  90.07%，城市口径 76.0%，地图 131 个要素。

验收过程中记录 5 次失败或中断：国家询问过早、两次 Provider/Runtime 重连、Data 漏选路线角色且
Root 丢失地图 embed、以及 90% 求解前的一次必要参数输入。前三类机制问题已分别由 typed country
推导、current-request Tool search/路线依赖、Root exact map embed 转发修复；Provider 使用用户更新的
有效凭据；最后一项以显式估算口径继续，没有把默认值伪装成原始事实。

### 2026-08-20 ADR-024 真实 DeepSeek 生产门记录（不作为 ADR-025 v2 验收证据）

`apps/web/scripts/real-deepseek-e2e.mjs` 只运行多 Agent 业务门，默认使用 `observe` 模式，不改写任何
`tool_choice`；`real-deepseek-single-agent-e2e.mjs` 只运行单 Agent 业务门；
`real-deepseek-tool-capability-e2e.mjs` 是独立的原生 `tool_search → spawn_agent` 诊断门，不由业务门
默认调用。三者共享 `scripts/real-deepseek/harness.mjs` 的 Provider、fixture、Task/Run、bounded
timeline 和终态清理，并在 `E2E_REAL_DEEPSEEK_SELF_TEST=1` 时不访问网络。

当前 exact `deepseek-v4-flash` 已通过正式 typed model metadata 获得 `supportsSearchTool=true`。
多 Agent 业务门从真实 `warehouse-network-copilot` Task 上传 6 个 fixture，验收 typed Data/Network
child、准备输入交接、12h baseline、只新增 `WH-CANDIDATE-BALIKPAPAN` 的城市与需求加权变化率以及
完成地图。单 Agent 业务门验收无 child、完整报价数量与 `warehouse_quote_mean_calculation.v1`
证据绑定，以及 `minimum_feasible` 的 90% 需求加权覆盖结果。真实业务门不要求模型采用固定 Tool
次数或顺序；invalid wire Tool、Tool 后流断开、权限/身份/服务器失败和清理失败保持 typed terminal
failure。严格的 Tool 次数、顺序、Network Resource inventory 和 topology 只属于
`real-platform-e2e.mjs` 的 fake/deterministic 合同门。临时 Provider、Run、Workspace、Project 与原
Provider 选择在终态清理。

2026-08-21 ADR-024 最终自然语言复跑结果：多 Agent 门 40 个有效 Chat 轮次、133.552 秒，完成
Data/Network child、12h baseline、只新增 Balikpapan 的城市数量与需求加权变化率（本 fixture 均为
`0`）和地图；单 Agent 门 26 个有效 Chat 轮次、109.585 秒，以 580 条完整报价生成并绑定脚本
evidence，选择 Balikpapan 与 Jambi，`minimum_feasible` 两阶段均证明最优，12h 需求加权覆盖率为
`0.9005214995572174`。deterministic multi-agent Gate 1/2 均连续通过并保存 `route_matrix.v3`、
`network_baseline.v2`、`network_coverage_geojson.v1` 与 `map_card_spec.v1` provenance。接受运行没有
不可见 wire Tool、已接受 Tool 后断流、权限/身份失败或清理失败；真实门现在也会拒绝带
`codex.turn.completed.error` 的 Turn，不能再把 Tool 已完成但 Provider 后续协议失败的 Run 记为 PASS。

该复跑属于 ADR-024 旧合同，不证明 ADR-025 v2 source-unit/prepared 真实门，也没有证明模型层加速。旧自然语言基线为 Multi 26 轮/91.768 秒、Single 24 轮/84.851 秒；
第一批拆门后的基线为 Multi 26 轮/98.407 秒、Single 23 轮/97.383 秒。当前 Tool 合同减少了标准
Data 往返、把单设施路线缩到精确候选集、把最小仓数求解限制为最多两个 CP-SAT stage，但自然模型
仍可能做额外计划、搜索和一次 typed 参数纠正，因此最终 Provider 轮次与墙钟时间反而增加。性能
结论必须分开陈述为“Tool 内部工作与磁盘显著收敛，真实模型编排未加速”。

本地共享 Tool build store 当前占用约 405MB，其中两个被三份 prepared descriptor 引用的 build
分别约 55MB 和 298MB；仓网单/多 Agent descriptor 各约 40KB。它替代了第一批前约 10.2GB 的
package-keyed 重复环境。首次新 fingerprint 的 Copilot 环境准备观测为 31 秒，随后同 fingerprint
冷启动准备为 1 秒；GC 只移除未被 descriptor 引用的 build。

最终回归证据：Copilot SDK 114、Provider SDK 11、Planner 186、Maps 38、Web Vitest 184 files/
1348 tests、两个 4-Skill Copilot manifest、八个 Skill validator、Web Rust 148 passed/7 ignored 与两次
deterministic multi-agent gate 全部通过；Rust 中要求外部 PostgreSQL/真实 Runtime 的 ignored 门仍按
各自前置条件保留，不冒充已运行。

Chat 与 Responses 共用同一个 Thread/Turn/ToolRouter/Agent/Skill/MCP 主干。Chat 仅在 wire 边界把
canonical Responses request 转为 Chat Completions，并把 SSE 恢复为 `ResponseEvent`；canonical history
中的 completed/client ToolSearch call-output 对做 request-scoped Chat function 转译，已加载且当前
request 可见的 schema 在新 Turn/resume 后继续可用，不进入 Core registry、缓存或第二份 Tool 状态；
失败、取消和孤立结果不产生目标，已从当前 Runtime 移除的历史 Tool 仍由 Core ToolRouter typed 拒绝。
Data Tool 对无歧义 source profile 自动物化字段 mapping 并原子完成 geography；
baseline Tool 的 `coverage_mode=auto` 根据标准输入是否存在 current assignments 决定实际或优化现有
仓口径，模型不再猜测后重试。

### 历史验证过程（不代表当前能力结论）

#### 早期真实 DeepSeek 工具能力门

使用 `apps/web/scripts/real-deepseek-e2e.mjs` 的 opt-in 门，从真实
`warehouse-network-copilot` Task 入口创建临时 Project/Workspace，上传仓网 6 个 fixture
文件，并通过本地只记录元数据的转发探针调用真实 DeepSeek。探针不记录密钥、完整 prompt、工具
schema 或工具参数；临时 Provider、Run、Workspace 和 Project 在终态清理，原 Provider 选择和
模型目录恢复。

已观察到的第一轮安全摘要：

- Chat 请求包含 26 个 Runtime 可见工具，`tool_choice` 未设置；其中有
  `multi_agent_v1__spawn_agent`，没有 `tool_search`。
- DeepSeek 返回了结构化 `exec_command` Tool call，说明 function-tool wire 调用本身可达；但
  当前模型没有 Runtime 原生 `tool_search` 能力，不能进入仓网 deferred MCP discovery。
- 门以 typed `provider_tool_search_capability_unavailable` 终止，没有提示词重试、历史 schema
  回放、single-agent fallback 或伪造路线/地图结果。脚本另有普通文本响应的 typed
  `provider_tool_call_not_produced` 分支，当前这轮未触发。

因此这次是 E3 受控 Provider 能力证据，不是仓网 E4；要通过完整真实门，必须由可信 Runtime
模型目录为 `deepseek-v4-flash` 提供原生 `ModelInfo.supports_search_tool=true`，并重新运行同一
脚本验证 `tool_search → spawn_agent → Data/Network MCP → 12h baseline → Balikpapan 增仓时效变化率 → map` canonical 链。不能按模型名
或 DeepSeek 的普通 `/models` 响应推断该能力。

#### D3：per-model ToolSearch capability 与真实仓网门

Provider model 配置已收敛到 Runtime `ModelProviderInfo.models` 的 typed
`ProviderModelConfig`。配置按 exact `model_id` 合并到 `ModelsManager` 产生的原生
`ModelInfo.supports_search_tool`；Provider 级别、模型名、普通 `/models` ID 或错误文本都不推断
该能力，配置存在但未声明的模型默认为 `false`。Platform 的 Provider model DTO、PATCH、Profile
恢复和 refresh 保留同一 `supportsSearchTool` 字段；空/失败 refresh 不覆盖既有 capability。

真实 DeepSeek opt-in 门不再写 `model_catalog_json`。它通过正式 Provider refresh/typed model
metadata API 对 exact `deepseek-v4-flash` 持久化 `supportsSearchTool=true`。本轮最小门真实通过：
Round 1 暴露 12 个工具并结构化调用 `tool_search`，Round 2 暴露 17 个工具并结构化调用
`multi_agent_v1__spawn_agent`，canonical item 为 `collabAgentToolCall/spawnAgent`。随后完整仓网
门的首个 Chat 请求没有返回结构化 Tool call，终态为 typed
`provider_tool_call_not_produced`；受限 timeline 显示只有 Root thread 的 started Turn，尚未
创建 Data/Network child，也没有 wait/mailbox、MCP 或地图 producer Item。因此这次没有证据表明
Runtime 协同或地图投影失败，不能把模型未作出结构化调用解释成业务链成功。此前一次完整门曾执行
`discover_workspace_sources`、`inspect_workspace_sources`、`prepare_network_input` 和
`prepare_network_geography`，但以 `copilot_chain_incomplete/map_producer_item_not_projected`
停止；这些都是未完成的 typed 证据，完整 Provider E4 仍未通过。

真实门失败时脚本输出有界 timeline：Root/Data/Network thread 与 Turn 状态、协作调用/等待/消息、
MCP 工具名及终态、地图 producer Item 和最后 assistant 文本摘要。它不记录 key、完整 prompt、
schema 或 arguments；临时 Provider、Run、Workspace、Project 和原始选择均已清理，未伪造业务结果。

#### D5b：Role config path 与非法 wire Tool 诊断

D5b 修正了 Copilot package 将 server-owned Role metadata 写成 `agents.roles.<role>.*` 的路径错误。
原生 `AgentsToml` 使用 flatten Role 合同，正确路径是 `agents.<role>.*`；旧路径会产生
`agent role roles must define a description` warning。修正后该 warning 消失，Role 的 typed MCP/Skill
projection 测试与 child cold-resume 测试通过。

同一临时 Provider 的真实门随后确认：Data Turn 已完成并生成 prepared input；Root 的 `wait` Item
以 `completed` 终态返回，Provider 每个 Chat 请求均为 HTTP 200 且 SSE `done`。Root 下一次采样时
request 只暴露 12 个基础工具，但 DeepSeek 返回不可见的 `multi_agent_v1__spawn_agent`；Runtime
公开的 `codex.unknown` error 为 typed `interrupted`，没有 Network child、MCP Network Tool 或地图
producer。该失败归类为 `provider_tool_call_not_visible`，不是 wait/mailbox、Provider stream 或
Platform map projection 故障；没有伪造重试或业务结果。确定性 multi-agent gate 在重启后连续两次
通过，包含完整 Resource schema 与 map provenance。

当前 Chat seam 已把这类 request 外 Tool 名改为原子、不可重试的 provider protocol violation：
在投影任何 FunctionCall Item 前先校验整组 Tool 名和参数，因此不再显示为
`stream disconnected`，也不会留下已接受一半的 `tool_search`/业务 Tool 组。

#### D5c：Chat history-loaded Tool 投影与真实完整门

Chat bridge 对 canonical history 中的 completed/client ToolSearch call-output 对建立 exact
namespace/name reverse target 和 function schema，并将其投影到当前 Chat request；不会按 `turn_id`
清除跨 Turn 或 resume 后的已加载 Tool。缺失、失败、取消和孤立结果不产生目标；同名不同
identity/schema 是 typed conflict。Runtime 在 active Turn 注入 model-visible Agent completion 时
仍可携带内部 metadata，但 Chat wire `messages` 不序列化该字段；当前 Runtime 已移除的历史 Tool
即使仍可见也由 Core ToolRouter typed 拒绝。

`codex-api` 的 222 项测试及 history-loaded/resume-shaped schema 复用、相同 target/schema 去重、
failed/cancelled/orphan 结果处理等 focused 机制门已通过。2026-08-22 的正式 Web 真实门也已通过：
新 Workspace 上传 `Downloads/mock_data` 的 6 个文件，在同一 multi-agent Task 使用
`provider=deepseek-e2e`、`model=deepseek-v4-flash`；拓扑恰好为 Root
`01a02752-3f97-7600-83d0-0c5a961fcec7`、Data
`01a02752-df97-73d2-9494-9c1195946d54`、Network
`01a02754-71a8-70c3-98a2-ac2ea27c0704`，后续任务复用同一 child。三轮分别耗时 159s（12h 需求加权
81.5%、城市 66.0%，map Ready）、298s（默认 Haversine；Balikpapan 后需求加权 86.5%，+5.0pp，
城市 74.0%，+8.0pp，map Ready）、150s（`minimum_feasible` proven，最少新增 2 仓/总 13 仓，
Balikpapan+Jambi，需求加权 90.1%，map Ready）。

ToolSearch 证据符合 history-loaded 设计：Data resume 直接调用 `discover_workspace_sources`；用户输入
后 Network 直接调用 `prepare_route_matrix`；第三轮直接执行
`prepare_route_matrix → solve_p_median`，仅为此前未加载的 facility/cost Tools 搜索。正式成功 Task 的
unknown tool、`provider_tool_call_not_visible`、`stream disconnected` 与 Runtime WARN/ERROR 均为 0。
Task 内记录 8 次可恢复 typed 失败（Skill item 缺 name、两次 candidate ID route 拒绝、identity/scope/
cost-ref/incomplete-matrix/prior-rule 不匹配）；均最终恢复，列为后续 Agent 效率/鲁棒性问题，不计为门失败或本轮修复内容。验收使用与仓库真实门一致的本地转发路径，完成后 base URL 已恢复 `https://api.deepseek.com`，
3 个模型与 Flash ToolSearch 均恢复；直连/TUN 及旧 Thread Provider 切换失败不纳入本机制证据。

#### D6：仓网 Agent 执行面与地图交付终态

仓网多 Agent package 现在通过 native Role 的 typed `[features] shell_tool = false` 关闭 Root、
`data_agent` 和 `network_agent` 的 shell/code 执行面；该设置只属于仓网 package 的三个 Role，
没有修改 Profile 全局 capability。Root 仍保留原生 `tool_search`、协作和用户输入面，Data/Network
仍保留各自 package-owned MCP。package/runtime focused gate 检查真实模型请求不含
`exec_command`、`shell_command` 或 `write_stdin`，并检查 Root 的协作面与 child 的领域 MCP 仍可见。

生产 Skill 进一步把 `prepare_network_input` 和 `create_network_map_card` 的成功结果定义为
当前工作的 terminal Tool 结果：Data 只交接 Workspace 相对路径与 input identity，地图只引用
本次返回的 embed code；成功后不继续 Resource 探索、低层 `create_map_card` 或样式修订。真实门的
受限 timeline 记录 MCP/地图 Item 的 typed status、error code/message 和 bounded result summary，
不记录 key、完整 prompt、schema 或 arguments。

D6 复跑中已观察到完整 canonical 链 `spawnAgent → wait → Data MCP → prepared Workspace input →
Network route/baseline/coverage → create_network_map_card(completed) → closeAgent`；域地图 Tool
返回的结构化结果为 `open-web-artifact/inline-visualization.v1`，并保留 producer Item、Thread、Turn
和 Run provenance。另有真实 Provider 轮次在模型继续选择低层 `create_map_card`、求解 Tool 失败或请求
用户审批时以 typed failure/timeout 终止；这些不能解释为地图 provider 实现失败，也不计为完整门通过。
若 Provider 发出当前 request 未公开的 `bash` 等 wire Tool，timeline 记录
`invalid_wire_tool_names`，Runtime 不执行该调用，也不把它当作 shell capability。
deterministic multi-agent gate 在 clean service restart 后连续两次通过，未发现 shell/tool loop 或
残留 Run/Workspace 冲突。

#### D7：真实 DeepSeek 单 Agent 全量报价与 90% 选址门（当前合同已通过）

`test:e2e:real-deepseek-single-agent` 固定选择 `warehouse-network-single-agent`，不运行或允许
`spawn_agent`。当前自然语言门在一次 Root Turn 中完成 `discover_workspace_sources → inspect_workspace_sources →
prepare_network_input → prepare_route_matrix → plan_cost_matrix → solve_p_median`，collaboration 与
mailbox timeline 均为空，并在 `outputs/warehouse-network/calculations/` create-new 执行 Python
脚本和 typed evidence。

成本 Tool 以 `warehouse_quote_mean_calculation.v1` typed evidence 校验并绑定 `observed_quote_mean`；
业务门验收 `opening_policy.kind=minimum_feasible` 的 typed 结果和 90% 目标，不把模型采用的
Tool 次数或顺序当作业务成功条件。当前结果选择 `WH-CANDIDATE-BALIKPAPAN` 与 `WH-CANDIDATE-JAMBI`，12h 需求加权覆盖率为
`0.9005214995572174`。门禁从 `solve_p_median` 的 bounded typed `coverage` 断言数值，不读取最终
Assistant 文本判定成功。成本均值外推是敏感性方案，缺少仓租、建设、容量和吞吐成本时不称为经济全局最优。首次启用或 source revision 变化仍需要正式冷启动，
测试在 `restartRequired=true` 时报告 typed `copilot_restart_required`，不会继续到 Task 503。单 Agent
没有可预先 spawn 的 child Role，因此安装投影在 Task 前保持 `Configured`；当前 revision 且
状态保持 `Configured` 时允许创建 Task，Root/child 的真实 MCP 执行能力由随后完整业务 Turn 证明，
不把缺少 child inventory 误报为安装失败。

## 3. 历史：2026-08-12 clean real Web E4（已被 ADR-023 合同替代）

在当前 Server/Codex、clean DB/Profile、真实 Provider 和一个授权 Workspace 中，使用用户给出的
三条自然语言请求，在同一 Task/Root Thread 依次完成：

- **S1 数据准备：** 浏览器观测耗时 164.782 秒。Root 原生委派 Wanwan；Data Tool 返回 ready 的
  `normalized_network_input.v1`，覆盖 50 个需求城市、11 个已有仓、12 个候选仓、50 条当前覆盖
  和 580 条路线报价；Network、地图、报告和 Artifact 调用均为 0。
- **S2 完整规划：** 浏览器观测耗时 302.141 秒。Harbor 保留 11 个已有仓并新增 Kendari、Manado，
  得到 13 个活跃仓；总成本由 55,815,960,000 降到 18,579,390,907.47 IDR。6/12/18 小时需求
  加权覆盖由 55.8%/67.3%/72.4% 提升到 73.9%/83.2%/96.6%，城市覆盖由
  34%/44%/54% 提升到 52%/70%/90%。对话出现 ready 的 `map.v3`，最终只有一个 ready、
  3,930-byte、`text/markdown` 中文正式简报 Artifact。首次报告调用因模型选择不存在的父目录被
  Tool 以 `workspace_file_invalid` 拒绝，随后改用 Workspace 根文件名正常完成，没有重复 Artifact。
- **S3 关闭场景：** 浏览器观测耗时 136.828 秒。Root 恢复同一个 Harbor，只执行场景评估与比较，
  不重做 geography、p-median、地图或报告；关闭 Bekasi 后 12 个仓的成本与 6/12/18 小时覆盖不变，
  affected/reassigned 为 1/1，Artifact 增量为 0。

三轮从首次发送到最终观测共 603.791 秒，包含浏览器轮询间隔。另一个同日恢复门在 final report
approval pending 时整页刷新，验证授权 Workspace、Root Thread、history、Agent activity 与唯一
pending approval 恢复，接受后原 Network child 继续并产生唯一 Artifact。地图卡片的 typed 状态与
DOM 已验证；用户明确跳过 Mapbox 视觉细节，因此不声称地图像素或交互视觉验收通过。

## 4. 最近一次已退役旧链的真实最小 E2E

证据来源是 Codex 任务“开发牛马”中 2026-08-08 完成的真实运行记录，任务 ID
`019fdfc8-d475-7821-8ff6-a3b785a9d536`，执行 Turn
`019fe094-c071-75d2-bc4a-80d46166eabf`。运行当时使用仓库内旧 E2E 脚本和旧服务路径；
这些入口已随旧数据面删除，仓库也没有保存一份可重放的脱敏 evidence JSON。因此下列
结果是已核对的历史运行记录，不是当前 checkout 可重放的发布证明。

### 已通过

- E2E `7/7` 阶段通过，约 721 秒；
- 真实 DeepSeek Provider 与真实 Codex Runtime；
- Root、Network Agent、Data Agent 均参与；
- 4 个 Agent execution 全部为 `completed`；
- 2 个官方 Root `request_user_input` 均为 `answered`，无 pending input；
- Work State 中 `network_input`、`route_matrix`、`network_deliverables` 均为 `ready`；
- 处理 50 个需求城市、11 个仓、550 条球面距离路线；
- `optimized_existing_footprint` 为 50,815 个需求单位完成分配，且未冒充
  `actual_current`；
- 生成 `network_planning_report.v1`；
- 后置供应链 Python 测试 `100 passed`、Codex 合同检查和 `git diff --check` 通过；
- 本轮没有新增 `codex-rs` 修改。

### 仍有异常

- 一次重复登记中间交付件触发非关键 Work State mutation 失败；最终组件和最终 operation
  仍完成。该异常说明模型可见的低层事务接口仍容易重复提交，不能按“最终成功”忽略。
- 约 12 分钟的最小链路仍依赖大量模型协调，尚无产品级耗时、取消和阶段进度门禁。

### 这次运行没有覆盖

- `current-coverage-extension` 和 `actual_current`；
- 成本矩阵、网络场景、p-median、服务约束选址和地图；
- 浏览器从新建 Copilot 开始的完整创作旅程；
- SDK package push/import、Catalog Compiler、Profile Installation、Runtime discovery；
- 浏览器刷新、断线恢复、Profile/Runtime 重启；
- 未授权 Role/Tool 与跨 Workspace 路径拒绝；
- 第二个非供应链领域；
- 两用户并发隔离。

### 为什么它不是产品 E2E

已删除的 `apps/web/scripts/enterprise-supervisor-e2e.mjs` 当时：

- 直接通过内部 API 上传数据并创建带仓网 component 的 Work State；
- 从仓库内置 6.0 Agent/Supervisor 复制定义并发布 Draft；
- 在任务 Prompt 中注入 Work State ID、Run ID、字段形状、MCP/Tool 使用规则、执行顺序
  和禁止事项；
- 没有通过 Tool SDK、Skill Studio、Agent Studio、Copilot Builder、Profile Installation
  和 Runtime discovery 的完整路径。

因此证据等级是 E3，而不是 E4。

### 2026-08-09 阶段一 Workspace 与 Task 绑定证据

以下证据与上面的旧仓网 7/7 E2E 分开判断，不使用 Work State、Dataset、SourceAsset 或
新增 Codex seam：

- `native_workspace_probe` 使用当前 checkout 构建的 Codex app-server、真实 Profile Host、
  native `collaboration.spawn_agent` 和 Python FastMCP；Root 与 child 在 Indonesia Workspace
  只能读取 Indonesia 文件，在 Thailand Workspace 只能读取 Thailand 文件，Plugin process
  cwd 没有被当作业务 Workspace；Root 在 child 运行期间通过官方 `turn/steer` 继续交互。
- `StartRunRequest` 的合同测试证明退役的 Run-level `workspace_id` 会被明确拒绝；Browser
  client 和 Web client 的 52 个定向测试证明 Task 创建提交 Workspace UUID，而 Run 启动和
  Task analysis readiness 不再提交 Workspace。
- 一次性干净 PostgreSQL 集成测试证明 Task 创建会校验 Project、Profile、Workspace
  grant 和 Workspace 状态，响应不暴露 `root_path`；不存在的 Task 统一返回 typed
  `task_workspace_unavailable`。
- Run orchestrator 一次性数据库测试证明同一 Task 多 Run、合法同 Task fork、恢复和续跑
  保持同一 Workspace；Run-level Workspace 切换、跨 Task fork 和跨 Workspace fork 在
  预解析、入队及执行层被拒绝。
- 当前 `codex/` 没有新增源码差异；上述能力复用官方 cwd、spawn、steer 和 fork 路径。

这些证据把 Task→Workspace 固定合同和 native Root/child Workspace 传播提升到 E2；它们
仍不是仓网完整 E4。

### 2026-08-09 阶段一普通 Workspace 文件证据

Slice 2 直接复用 `/workspaces/{id}/files` 与 `GitRuntime`，没有增加文件 registry、revision、
Task binding、Task files API 或 Artifact 输入链：

- Web 只有 Workspace 文件面板这一处普通文件入口，支持列举/搜索、逐文件上传、文本查看、
  二进制下载和删除；每个 HTTP multipart 请求只接受一个 file。
- 已有目标默认返回 typed `workspace_file_exists` conflict；Web 对当前文件明确选择覆盖、
  改名或取消。多选由 Web 逐文件执行，后项失败或跳过不会把前项伪装成失败或静默重试。
- `GitRuntime` 以 Workspace-relative path 写入，同目录 staging file 完成 write/sync 后才
  publish；publish 成功即成功，staging 失败会清理临时文件。19 项 crate 测试覆盖普通读写、
  create-new/overwrite、absolute/`..`、symlink、regular-file 和大小边界。
- fresh PostgreSQL ignored integration 证明同 Workspace 两个 Task 可读取同一路径文件，
  `/tasks/{id}/files` 不存在，多-part request 在任何文件落盘前被拒绝；另一个租户对普通
  Workspace files 的 list/content/delete 都得到 `NOT_FOUND`。
- 当前 schema migration 删除 Data Intake session/draft/mapping/input request、SourceAsset、
  Dataset Release/file、Task dataset/analysis snapshot/intake projection、Agent dataset
  dependency，以及 `workspaces.source_revision` 和 `artifacts.intake_envelope`；干净数据库
  断言这些对象不存在。
- 冻结的仓网 Agent/Supervisor/Instruction Policy/Tutorial 包、旧 E2E 和 lifecycle smoke
  入口已删除；确定性供应链 Python MCP/算法与普通 Workspace fixture 保留供后续切片重建。

这些证据把普通 Workspace 文件数据面提升到 E2，但不证明 Profile 热加载、原生协同或完整
仓网 E4。

### 2026-08-09 Profile 原生热加载审计证据

Slice 3A 只审计 Codex 原生路径并运行定向测试，没有修改 `codex/` 或产品实现：

- `skills_changed_notification_is_emitted_after_skill_change` 通过，证明 Skill watcher 会清
  Runtime Skill cache 并发送 `skills/changed`；客户端仍需重新 `skills/list`，新内容从下一
  Turn 生效。
- `refresh_mcp_servers_keeps_the_previous_runtime_alive` 通过，证明 MCP refresh 原子发布新
  runtime，而活动 step 持有的旧 runtime 可继续完成。
- `apply_role_skills_config_disables_skill_for_spawned_agent` 通过，证明原生 Role layer 可以
  控制 child Skill；`cli_override_can_update_project_local_mcp_server_when_project_is_trusted`
  通过，证明高优先级 Role MCP enable/tool policy 可以递归保留底层 server transport 定义。
- 源码确认 Role 文件在每次 spawn 时重读，因此内容修改对下一次 spawn 生效；既有 Root
  Thread 的 Role 集合、allowlist 和 spawn limits 不会随 user config reload 完整重算，角色
  增删改名只对新 Root Thread 保证。
- 源码确认 selected capability discovery cache 是 Thread-scoped、没有 invalidation，并同时
  缓存成功与失败。因此它不能作为 built-in 热加载基础；阶段一不使用 Plugin、Marketplace、
  Installation 或 `selectedCapabilityRoots`。

这些证据把 Skill/MCP 热刷新与 Role 应用语义提升到 E1。Slice 3B.1 已进一步实现启动前
Profile seed 与显式应用资产 composition：

- checked-in `copilots/warehouse-network` 提供 Supervisor、Data、Network planning、Map delivery
  四项 Skill 默认内容和 `data_agent`、`network_agent` 两项原生 Role 默认配置；
- checked-in `copilots/warehouse-network-single-agent` 是不同 package ID 的独立 Copilot；它用
  一个 Root Agent 直接启用相同仓网 Tool policy 并关闭 multi-agent。该 Root Agent 配置不会作为
  child Role 安装；当前证据为 package validate、Root config projection 和 focused contract；
- Profile Host 只接受 native Skill/Role 两种 typed startup destination，并显式区分普通
  `Seed` 与 package-managed `Managed`。普通 seed 仍只在缺失时 create-new 并保留已存在内容；
  多 Agent 仓网包使用其四项 Skill 和两项 Role 声明 ID，部署升级在 Runtime 启动前只在
  内容漂移时原子更新。duplicate spec 在任何写入前拒绝；symlink、目录、逃逸和非法输入失败；
  `config.toml`、用户其他 Skill/Role 与 Workspace 不受影响。6 项单测通过；
- 仓网验收 Profile 通过 Codex 官方进程级 feature override，在首次请求前关闭
  `plugins`、`remote_plugin`、`apps` 与 `tool_suggest`；CLI feature discovery 精确报告四项
  均为 disabled，real clean-Profile Runtime gate 仍能发现三项 package Skill、两项 Role 与
  role-local MCP。平台没有复制 Plugin/App/Tool Suggest discovery，也没有改写 `config.toml`；
- real mode 在 Server 启动前枚举显式可信 Copilot 根，并对每个包执行 generic
  `copilot prepare`。两个仓网包只按 package ID 引用根级
  `tools/warehouse-network-planner`/`tools/warehouse-network-maps`；SDK 从 Tool package manifest、
  runtime、direct manifest/hash lock 生成外置依赖环境和
  `prepared-tools.v1.json`；Server typed 校验 capability root、server、stdio transport 和 env
  binding，并要求 command/dependency 全部落在同一 trusted shared build fingerprint 及其严格 build marker 内，
  不从 cwd、Workspace、`CARGO_MANIFEST_DIR` 或源码树扫描 fallback；
- Profile 不复制 Tool、venv、Node dependency、Mock、cache 或 test。Copilot Root 只显式加载小型常驻 Skill；
  多 Agent Root 不发现任务 Skill，单 Agent Root 与 child Role 按其显式 scope 通过 Runtime Catalog 渐进发现并按原生选择读取。Role source 持有精确 MCP server scope
  与 Tool 级 approval policy；三个仓网 Role 都显式关闭上游 multi_agent_v2，只使用当前已验证的原生协作合同。server 内 Tool 通过 Runtime deferred discovery 暴露，Server 从 prepared descriptor 投影
  role-local MCP transport，Root 没有全局仓网
  MCP；Data4 全部是有界本地预批准，Network 预批准本地计算/验证与 `publish_network_planning_report`，maps 预批准
  `create_network_map_card`、`create_map_card` 与 `revise_map_card`，外部地图调用和 final Workspace 地图导出保持 `prompt`。prepared transport
  不携带 cwd，Runtime 使用 Thread 已授权的 Workspace 作为 maps stdio MCP cwd；
  状态写入 Profile 私有 `mcp-state/maps-mcp`。Network 所有分析入口在 Tool schema 固定为
  Data provider 的 `supply_chain_data` inline inspection identity 与 Workspace prepared file；Role
  不通过 Data Resource 发现或读取来恢复交接。

这些证据把启动 seed、prepared environment/descriptor 和低层 MCP inventory 提升到 E2。Slice 3B.2 又完成
两个使用生产 composition、真实 Codex app-server 与本地 mock Responses provider 的 ignored
exact integration：

- clean Profile 的官方 `skills/list(forceReload=true)` 在 user scope 精确发现三项仓网 Skill，
  repo scope 为空；Standard Root 的官方 MCP status 没有四个仓网 server；
- Root 通过原生 collaboration namespace spawn Data/Network。Data 只启用 warehouse-data，
  只看到 supply-chain Data 的 4 项工具；Network 只启用 warehouse-network，只看到
  Network/Map 的 `12+9` 项工具。两者都由真实 child model request 与官方
  `mcpServerStatus/list` 观察，不由 Host 文件自报；
- Root 两次 native wait/mailbox 与对同一 Data child 的 follow-up 正常，Root/Data/Network
  全部到达原生 terminal。对既有 Data child 请求官方 MCP reload 后，在 follow-up safe
  boundary 继续使用同一精确 inventory；Role layer 没有 delta，因此没有伪造 startup event；
- 修改 Profile seed 副本中的 Skill 后收到 `skills/changed`，force reload 读取新 marker；修改
  Profile Role 后只有下一次同 Role spawn 的真实请求包含新 marker，既有 child 请求不变；
- 隔离用例在 Profile 中写入 malformed Role 并 restart，Runtime 发出
  `Ignoring malformed agent role definition` config warning；Root 的 native spawn 明确返回
  `unknown agent_type`、不创建 child，Root 仍正常终结；
- Profile Host 为每个 Host 持有系统临时根下的 canonical 0700 neutral process cwd，初启与
  restart 共用；Profile 私有 `HOME`/`USERPROFILE` 与 neutral cwd 阻止宿主
  `$HOME/.agents`、Runner/源码祖先 `.codex` 进入 clean Profile discovery。Thread 业务 cwd
  仍由官方授权 Workspace 参数拥有。

这把当前开发者包的原生发现、Role activation、精确 inventory 和已声明热边界提升到 E2，
但没有仓网业务入口或完整链，因此仍不能声称阶段一完成。3B.3-B2 已删除
`RunStartPreflight`、Governed runtime mode、request-scoped Role SHA/inventory 校验和 Profile
`platform-agents` 第二启动系统；真实 Indonesia/Thailand 探针继续证明 Standard Root 与原生
child 的 `sandboxCwd` 都等于各自授权 Workspace；prepared transport 不携带 cwd，
Runtime 同样使用该 Thread 已授权的 Workspace 作为 stdio MCP process cwd。
Catalog/Studio/Python publish 生产表面已由 3B.3-B3 删除；3B.3-B4 又删除 DB-only readiness、
旧 Draft/Release/Installation、Agent definition/release/run binding、Workspace package release 与
Supervisor instruction policy release 等 14 张无 owner 表。Slice 4B.2-A 的 fresh
PostgreSQL 全迁移链又断言 Work State 十表、Supervisor 六表及 provider speculative hash
四列全部不存在，同时保留 provider_call_metrics 与 Runtime agent projection。Slice 4B.2-B 已由
migration 56 删除 `profile_capabilities`，并从 Codex initialize、ProfileHost、Server 和 Web
删除本地 capability manifest；ProfileHost 只 typed 校验官方四字段和 owned `codexHome`。
Slice 3 至此完成。
fresh PostgreSQL integration 明确断言旧 Agent/Supervisor/Catalog/Python authoring HTTP route
均为 `NOT_FOUND`，同时继续覆盖普通 Workspace 文件；全 workspace Rust wrapper、Web
lint/typecheck、181 个测试文件共 1280 项测试、build 与 no-desktop 通过。3B.2 clean/hot、
malformed Role 和 Indonesia/Thailand native Workspace exact gate 也在删除后继续通过。

## 5. 当前能力矩阵

| 能力 | 当前事实 | 等级 |
| --- | --- | --- |
| Codex Runtime bridge | 官方 Thread/Turn/Item、Agent 生命周期、Tool 和 Root 输入继续由 Runtime 拥有；pure-official probe 已由 `SubAgentActivity` Item 的 child Thread ID 通过 `thread/read` 读到 parent/source/Role/nickname/Provider/status | E3，child identity 无需本地 seam；physical-cwd join 属后续 backlog，不是当前仓网前置 |
| Runtime history | 当前 subtree 有 `thread/turns/list`/`thread/items/list` 与本地 history seam；隔离 sync 的 latest official 已原生提供 paginated history、Item timestamp 和 Turn error 语义 | official 能力已证实但尚未回填；legacy/materialization 清理进入后续 backlog |
| Durable event projection | `run_events` 有全局 sequence、run/thread/turn/item provenance、Task/Organization 授权查询和 reconnect replay | 身份/顺序基础可保留；payload、unknown event 与 Browser heuristic 收敛进入后续 backlog，除非真实仓网门证明会泄密或写错数据 |
| Profile Host | 单 Profile 目录、进程已隔离；只 typed 接收官方 `initialize` 四字段，并只以 `codexHome` 对 owning Profile home 做安全校验；Runtime HOME 与 neutral process cwd 已隔离；Server 仍有显式宿主 auth 导入 | E2，Runtime discovery 隔离已通过，身份隔离未完成 |
| Approval / User Input | Root 官方输入可持久化、回答并恢复执行 | E3，最小链 2 次通过 |
| Agent execution projection | 4 个 execution 收敛到 terminal；等待和输入可投影 | E3，刷新/重启未测 |
| Work State / Platform coordination | 无 production crate、route、MCP、gate 或 current schema 对象；migration 55 删除十张 Work State 表 | 已删除；不建设替代状态机 |
| Root coordination | Platform 第二控制面、Supervisor continuation 与主动 Root Turn 已删除；Runtime 原生 wait/mailbox/steer 是唯一协作路径 | E2 原生 runtime/projection gate；完整业务 E2E 未完成 |
| Capability Catalog | 无生产 crate、API、Browser DTO/client、UI 或当前 schema 对象 | 不再是阶段一能力 |
| Package Compiler | 无当前生产 owner；阶段二目标文档保留设计输入 | 不再是阶段一能力 |
| Profile Installation | Platform 从显式可信应用根发现一级 Copilot 目录，按 `(Profile, package)` 持久 desired state、configured revision、managed Skill/child Role IDs 与安全 failure；最小 Web 管理面只展示状态并激活/停用；冷启动前合并收敛或清理精确 native destinations，源码缺失 record 仍可停用并在 Host 成功后清空 managed IDs | E3：全量 Rust、disposable PostgreSQL lifecycle、真实 sync→Web可选→停用→移除源码→冷启动清理已通过。没有 Catalog/Release/Marketplace、多用户产品流或动态热切换 |
| Runtime discovery/readiness | 当前 instance 每次 status 最多调用一次官方 `skills/list(forceReload)`，按 package 声明 Skill 交集投影 `Installed/Configured/Unavailable/Failed`；不再有 Copilot `Ready` 或 MCP inventory 字段，Role/MCP 可执行性由真实 Task/child MCP completion gate 证明 | built-in native gate E2；无 DB ready |
| Copilot / Tool SDK | `init` 生成单/多 Agent源码，`tool init` 生成共享 Tool；`check` 聚合 validate→prepare→dev→test，`test` 支持 Root/child、多例隔离和 canonical history 取证；仓库 wrapper 提供隔离 bootstrap、可信 registry/build store 与安全 `sync` | E3：124 项 SDK 单测、bootstrap/wrapper/run-local 门、fresh single/multi real check、Provider SDK 11 + Cookbook 4 + stdio、124-link docs smoke 和真实 DeepSeek Web调用通过。没有外部 SDK push/import、生产模型质量矩阵、Web Studio 或 Marketplace |
| Skill 创作 | 阶段一仅支持直接编辑 Profile Skill；公开 SDK/Web 创作后移 | 无阶段一产品流 |
| Agent/Supervisor Studio | 旧 Settings/Sidebar Studio 已删除；原生 Profile Agent 配置保留 | 无阶段一 authoring 产品流 |
| Copilot Builder | 没有 Web 创作入口；现有 Profile 安装状态 API 不是 Builder、Catalog 或 Marketplace | E0 |
| Browser 产品入口 | 生产 `index.html` 只经 `browser-entry.ts` 渲染 `WebApp`；展开 Vite alias 后 production graph 为 75 个本地 non-test 文件，另 466 个不在图内却仍由 `tsconfig` 编译；完整 `PlatformClient` 又把 dead endpoint methods 带入 bundle | E2 单一生产入口；legacy import graph/client 收缩进入后续 backlog，不阻断当前仓网链 |
| Web UI parity | 历史 `check-main-ui-parity`、Git/UI overlay 和手工 SHA 清单已删除 | 已删除第二 UI truth；当前以类型检查、组件/合同测试、生产构建和真实浏览器验收为准 |
| Browser Terminal | 生产 `/web` WebApp 不调用 Terminal API；旧 App 路径仍保留 Workspace Terminal UI。Server 打开 Terminal 时按 `run.updated_at DESC` 猜一个最新 Run，并把输出投影到该 Run | E1 legacy 残余；多 Task/fork 时会错配会话，待确认旧 App 退出后原子删除，当前不建设新 selector |
| Browser local usage | 旧 App 的 `/profile/usage` 同时从 `run_events` 近似重算 token/turn，并在完全空时切换到 official `account/usage`；生产 `/web` 未调用 | E1 legacy 双 owner；随旧 App 删除，不建设第二 usage aggregator；`provider_call_metrics` 独立保留真实观测 |
| Browser hidden generation | 已认证的 `/runs/{id}/generate` 会在 adapter 中创建一个被 `suppressed_threads` 隐藏的持久 official Thread，手工等待 Turn delta 并 best-effort archive；production 无 caller 的 generic `rpc(method, Value)` 又保留字符串 `start_thread`/`send_user_message`；生产 `/web` 不使用两者，但 direct HTTP generation 仍可触发 | 已证实 legacy 偏离；原子删除进入后续 backlog，不阻断当前仓网链 |
| Browser remembered approval | 生产 `/web` 不使用 `/profile/approval-rules`；该 route 只凭 Run 授权接收 Browser 自报命令前缀并永久写 Profile `default.rules`，不绑定 pending approval/item/version/Profile | 已证实永久执行授权旁路；后续原子删除，当前仓网链只使用 exact pending approval accept/decline |
| Browser raw Profile files | 生产 `/web` 不使用 `/profile/files/{agents|config}`；config PUT 直接替换整个 `config.toml`，只在事后调用 `config/read`，绕过 official expected-version CAS 和 reload owner | 已证实配置写旁路；后续整链删除，Workspace `AGENTS.md` 文件面不受影响 |
| Browser Workspace preferences | 生产 `/web` 不使用 `browser_workspace_preferences`；旧 App 路径仍可存任意 JSON、不会实际应用却回显为 applied 的 Runtime 参数，以及取出后经 Terminal 自动执行的 worktree setup script | E1 legacy 假能力/危险执行残余；与 Terminal/Usage 同片删除，不保留为未来 Workspace 合同 |
| Browser custom prompts | 生产 `/web` 不使用 `/profile/prompts`；Platform 自行扫描和编辑 `$CODEX_HOME/web-prompts/<project>` 与 `$CODEX_HOME/prompts`，但 latest official 没有 prompts discovery/API，只有 cwd-scoped Skills/Plugins | E1 legacy Profile 污染；与旧 App 同片删除，不迁移为另一套 Prompt 系统 |
| Task→Workspace 固定合同 | Task 持久化唯一 Workspace；Run/fork/recovery 只能继承；Browser 无 Run-level 选择权 | E2；HTTP、数据库、orchestrator 与 Browser 定向测试通过 |
| 通用 Workspace 文件 | `/workspaces/{id}/files` + `GitRuntime` 是当前唯一用户上传/浏览/编辑文件产品面；Web 逐文件处理 conflict，同 Workspace 两 Task 共享和跨租户拒绝已通过 | E4 normal path；真实 Workspace 文件进入 S1/S2/S3 |
| MCP Resource | Codex 官方 Tool Item/ResourceLink/read_resource 与供应链 Workspace-scoped ResourceStore 形成 Network typed provider 数据链；Data inspection 只返回 inline `workspace_source_profile.v2`、source units 和 `workspace_source_inspection.v2` identity，不注册 Data Resource template。Network 通过 package-owned facade 使用 Profile+Workspace `supply_chain` Resource 域；ResourceRef/codec/bounds/store/runtime/Workspace primitives 已由领域无关的 `open-web-codex-provider-sdk` 提供，仓网只保留领域模型与算法。Platform 不存内容、不建 Broker、handoff ledger、latest 或相似性匹配 | focused 回归覆盖 inspection identity、Data Resource surface empty、Network Resource scope/ref 拒绝、prepared fresh reuse 与 pair-level partial reuse；真实 Web 耗时待本轮复测 |
| Final Artifact 与地图卡片 | durable `artifacts` 只接受 active Copilot `[[deliveries]]` 声明的 exact producer、固定 typed kind/schema/MIME/verifier 与 Workspace-relative descriptor，并有 Task grant、exact Run/Thread/Turn/Item provenance、物化字节和授权读取；Platform 不理解包或业务字段，也不把任意 Resource 升为 Artifact。仓网 `map.v3` 另有 provider-owned immutable `map_card_spec.v1`：Platform 只保存 renderer、exact spec ref 和卡片 provenance，Maps 保留 parent revision 链；纯样式 revision 复用 GeoJSON，新增覆盖线须先由 Network 产生新 GeoJSON。关闭 producing child 后，inline source 读取以授权 Task 的固定 Copilot package config 第一次 cold-resume child Role/MCP，再走 official `mcpServer/resource/read`；浏览器不能选择 config，Platform 不复制 GeoJSON。紧凑 GeoJSON profile 声明无值的字段类型，Maps 拒绝全空/类型不匹配表达式与未声明 image asset | focused Adapter/Server 回归覆盖首次 resume config；原失败地图 URL 已从 502 修复为 200/129 features，真实浏览器卡片状态 Ready 且控制台无错误 |
| Data Intake | SourceAsset、session/draft/mapping/gate、Dataset Release、Task binding/snapshot/projection 生产路径与当前 schema 已删除 | 不再是当前能力；阶段一无替代状态机 |
| 旧数据引用 | Platform 文件面只接受 Workspace 相对路径；Data/Network active surface 只使用 strict `ResourceRef`，Case/NetworkSnapshot、ArtifactRef、taskEvidence hashes 和双重 ref 旧 tail 已删除 | 已完成 Stage E tail deletion；继续保持单一 ResourceStore owner |
| 仓网算法 | Data4 标准化保留完整的用户路线 pair facts；Network12 通过一个 `prepare_route_matrix` 入口物化 provided/haversine 路线矩阵或给出 navigation 请求估算，计算成本、场景、求解、交互地图卡片数据和 Markdown 简报，并返回城市数量与需求量加权两种时效达标指标；`prepare_network_coverage_map` 从 exact assignment 生成通用覆盖 LineString。单次设施变更评估从 exact before result 的活动仓集合起算，一次求解并直接比较，返回 scenario ref、绑定标准化输入与前后方案的单一 `plan_comparison_ref` 及有界指标 | 路线规划/构建/显式重复验证和独立场景入口已从模型可见面删除；真实 Web 计时待复测 |
| Provider 定义与选择 | Profile `config.toml` 和 Runtime 已能持久化 Provider/模型及未来 Thread 默认选择；已物化 Thread 的实际 pair 由 official Thread settings 拥有。Runtime Provider catalog 必填返回 `supportsFunctionTools`；Platform 对可编辑 Provider 按 Wire API 归一化：Chat 强制 true、Responses 为 false，并在 `profile_provider_definitions.supports_function_tools` 精确持久化。Web 不展示 Function tools 复选框；模型级 `supportsSearchTool` 保持独立选择。已有 Chat Provider 由数据库迁移自动改为 true，数据库约束阻止再次写回 false。Runtime typed error 仍保护 Platform 外配置 | 第二/第三 owner 与 Task pair 全量收敛进入后续 backlog；当前交互只保留 Wire API 与模型级 Tool search 两个独立选择面 |
| Provider 模型刷新 | Codex 的 provider-scoped `modelProvider/models/list` 对 exact 目标 Provider 执行 fresh `/models`，不切换 current Provider；Platform 只在非空 typed success 后持久化该 Profile 的完整模型目录。由于 `modelProvider/list` 不回显自定义模型目录，单模型 capability/context 修改与启动恢复必须先从数据库完整目录合并目标模型，再一次性原子写入 Runtime `models`；禁止逐模型更新覆盖兄弟模型。启动恢复完成后安排一次安全边界 Runtime 重启，防止 live ModelsManager 继续使用恢复前快照 | focused 回归覆盖 Runtime catalog 为空时一次写入 flash+pro、修改 flash 保留 pro；真实重启同时核验 config 双模型和 Data child ToolSearch |
| Provider Secret | Profile/provider scoped Secret 以平台密文保存，只把稳定 env ref 写入 Codex config，并在 owned Profile process 注入 | E2；重启与零明文负向门保留 |
| Provider metrics | 持久化 schema/route 保留真实 token、延迟、压缩和终态观测；四个没有 Codex producer 的 speculative SHA 字段已删除 | 真实 usage 观测未验收 |
| 多用户隔离 | 身份 scope 部分存在，产品入口仍单用户 | E0，隔离矩阵未运行 |

## 6. 已证实的架构偏离

以下是本轮过度设计、重复 owner 与未充分复用 Codex 原生能力审计的统一索引。
Architecture 只陈述当前 owner 与运行事实，Development Plan 只维护唯一执行顺序，
ADR-018 是阶段一规范裁决，Codex 子树 seam 只由 Patch Map 分类。本表不新建第二份
架构 truth，也不把后续 backlog 提升为当前阶段前置。

| 稳定 ID | 分类 | 已验证问题与源码证据 | 风险 | 当前裁决与阶段/触发条件 | 目标 owner/删除条件 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| `P1-R2-CHAT` | 当前关键链 | pinned official Codex 的 Chat transport 与 Platform `PUT/list` 依赖保留在 `codex/codex-rs/codex-api/src/{chat_translate.rs,chat_translate_history.rs,endpoint/chat.rs,sse/chat.rs}`、`core/src/client/chat.rs` 和 app-server catalog processor；Chat bridge 将 Runtime 原生 client ToolSearch 映射为普通 function call；canonical Thread history 中的 completed/client ToolSearch call-output 对贡献 exact namespace/name 的 request-scoped reverse target 与对应 function schema，稳定去重相同 target/schema，并对 identity/schema 冲突返回 typed failure；projection 不改写 canonical `Prompt.tools`、Platform 状态或 Core ToolRouter registry，Responses transport 不变。业务 child 恢复后，Root 以 `send_input.items` 重新注入当前任务 Skill；Skill item 重新注入与 Tool schema 历史复用是独立事实；同一 Task、同一业务 Role 默认复用稳定 child target，只有所需 Tool 不在当前 request 时才调用原生 `tool_search`。Runtime 仍拥有搜索、deferred registry、Tool identity、审批与分派，Role MCP server scope 仍是搜索边界；历史可见但已被当前 Runtime 移除的 Tool 仍由 Core typed 拒绝。Chat Provider 的 function-tool 能力来自 `ModelProviderInfo.supports_function_tools` 的显式 typed 配置/目录事实，缺失时在 Core tool planning 返回 `ProviderFunctionToolsUnsupported`；ToolSearch 仍只由模型级 `ModelInfo.supports_search_tool` 决定；Runtime 不按 `wire_api`、模型名或错误文本推断，Platform 只在可编辑 Provider 保存边界按 Chat 类型写入 function-tools 总闸门。 | 自定义历史 schema 缓存、Platform 回放或 Core authorization snapshot 会制造第二个工具生命周期 owner；纯文本 `send_input.message` 又会使恢复 child 丢失任务 Skill | 仅保留 Chat wire 适配、Provider capability/catalog 和真实仓网 gate 所需的最小 typed seam；历史 Tool schema 仅通过 canonical history 与当前 request 投影复用，ToolRouter 仍是最终权限 owner；Role/Skill 只规定 missing-tool search 与 stable child continuation，不建缓存、调度器或重试。2026-08-22 Web 真实 DeepSeek Flash 门已通过：同一 multi-agent Task 恰好 Root/Data/Network 三个 Agent，同一 child 跨后续任务复用；三轮结果分别为 12h 需求加权 81.5%（城市 66.0%）、Balikpapan 后 86.5%（+5.0pp，城市 +8.0pp）、`minimum_feasible` proven 的最少新增 2 仓/总 13 仓、需求加权 90.1%，均 map Ready。ToolSearch 只为此前未加载的 facility/cost Tools 搜索；正式成功 Task 的 unknown tool、`provider_tool_call_not_visible`、`stream disconnected` 与 Runtime WARN/ERROR 均为 0。8 次 typed 可恢复失败保留为后续效率/鲁棒性问题 | Codex 拥有 wire/app-server typed 执行、deferred registry、搜索与 Provider capability gate；Supervisor Skill 只拥有 structured continuation 方法；Platform 只拥有 CRUD、授权与密文注入 | 已验证：Chat request/stream/history 基础 round-trip、history-loaded schema/reverse-target replay、resume-shaped input、相同 schema 去重、failed/cancelled/orphan handling、collision/schema rejection、ToolSearch schema 不进入 `Prompt.tools`、Chat unsupported/function-only Core 回归；2A Role/Skill/SDK/Web 文本合同、focused copilot_package/SDK CLI tests 和 2026-08-22 Web DeepSeek Flash 三轮真实验收通过 |
| `P1-SC-REF` | 当前关键链 | Network `server.py` 的 decorated active tools 使用一个 logical `supply_chain` provider、Workspace-scoped ResourceStore 和 strict `ResourceRef`；Data inspection 不发布 Resource，只由 Workspace authority 返回 inline typed identity。通用 ref/schema/codec/bounds/store/runtime/Workspace primitives 已迁到独立 provider SDK，仓网旧模块和 import 已删除 | 跨 provider 越权、Resource 不可达、同一内容多个身份 | Network 保持单一 provider ResourceStore 与 strict typed refs；Data 保持 inline inspection identity，不建 Broker/新表或路径 fallback | Network MCP provider 拥有中间内容；Data Tool 拥有文件 inspection identity；provider SDK 拥有通用 primitives | Provider SDK 11/11、Planner full 123/123 与 Data/Network stdio normal/domain gates 通过 |
| `P1-SC-DATA4` | 当前关键链 | `data_server.py` active path 的 inspect 返回 bounded inline `workspace_source_profile.v2`、exact source units、`preview_sample_count`、`total_count`、`total_count_exact`、`inspection_identity` 与 `inspected_relative_paths`；角色评估不构成全局业务缺口，只有选中 unit 的语义、单位、值或冲突无法确定时才询问。`prepare_network_input` 使用 `source_selections` 和显式 mapping，重检变化返回 `source_changed` 且不写 output；ready prepared 绑定 selected raw/admin provenance、roles、role_counts 与 `selected_source_identity`，fresh candidate 只有显式选中才自动复用，内容不同则一次选择。| 预览样本曾被模型误认为完整正文，且生成输入曾落在 Workspace 根目录或源数据目录；缺字段曾使 Data Agent 在 mapping 与 schema 错误之间重复调用而不询问；旧 source profile Resource 使 Data/Network 产生不必要的 read 链 | strict surface：discover→inspect→prepare，fresh prepared 为 discover→inspect→handoff；`needs_input` 一次询问并终止，`source_changed` 最多一次重检；Data `list_resources`/templates 为空；目录截断时不得断言候选仓不存在；Indonesia built-in asset 只属于验收 fixture，其他国家无 provider 时 typed unavailable | Data Tool 拥有格式/映射/inspection identity/标准化、typed 数据缺口和生成目录；Skill 只把业务终态转为一次询问，不建重试状态机；复用 `workspace_intake.py`、`mapping.py`、`normalization.py` 中纯函数与 `geography.py`，不建 source/revision/CAS 包装 | focused Data tests 覆盖 sample/total、`warehouse_type` 缺失 no-write、非标准表头、multi-sheet/nested JSON、order-independent identity、fresh reuse、selected raw/admin change、Balikpapan 位于 preview 外完整准备、Data Resource surface empty 与两套 stdio domain gates |
| `P1-SC-COST-MEAN` | 当前关键链 | `plan_cost_matrix` 的 discriminated `cost_policy` 区分显式数值与 `observed_quote_mean`；后者在 Planner owner 内读取 exact ready prepared input 的完整标准化报价，按层计算 `arithmetic_mean(price_per_vehicle / vehicle_capacity)`，把 `warehouse_quote_mean_calculation.v1` 的 prepared identity、完整总数、分层计数、币种、公式、均值和 Tool 版本写入 bounded result 与 `cost_matrix.v3`。单 Agent Role 显式保留 shell，但 Skill 只允许用户明确要求的同口径脚本读取 exact prepared input，并把 agent 创建的 typed script/result create-new 写入 calculations 目录；Planner 重新计算并校验脚本证据 | 旧 Skill 把 preview 上下文限制扩大成禁止执行端完整计算；Planner 又只收显式数值，导致用户已授权“按全量报价均值外推”仍被错误终止 | preview 继续只用于映射；普通均值外推由 Planner Tool 完成，脚本只作为用户明确要求的可审计证据，不修改 prepared/raw 数据或替代求解 | Planner 拥有业务聚合和 cost matrix；单 Agent Skill 拥有何时创建并执行脚本，Workspace 拥有计算证据文件 | focused cost/Tool tests 已覆盖 580 条完整报价、550/30 分层计数、均值、缺层失败、evidence mismatch、bounded structured output 与 exact input identity；Role/Copilot 合同测试覆盖 shell 开启、multi-agent 关闭和 calculations 目录约束；2026-08-20 真实 DeepSeek 单 Agent typed evidence/one-call gate 已通过 |
| `P1-SC-NET9` | 当前关键链 | `server.py` 的 active Resource/final tools 使用 strict `ResourceRef`；`prepare_route_matrix` 统一 provided/haversine 入口并返回 discriminated `ready/needs_input`，只有零缺失矩阵才发布 `route_matrix.v3`；导航继续走显式 request/import 合同。baseline、设施变化和选址 Tool 独立拒绝不完整路线或成本。`assess_facility_change` 组合一次 scenario solve 与直接 before/after compare，并返回 scenario ref、单一 provenance-bound comparison ref 和最多 10 个重点城市。`prepare_network_coverage_map` 要求结果中存在的确切 `service_target_hours`，发布 `network_coverage_geojson.v2`，在需求点和末程线上携带 `attained/missed/unassigned` 状态；Maps 只按该状态生成红绿灰图层，不重算 SLA。 | Data/Network 必须继续保持单一 provider ResourceStore 与 strict typed refs，不能恢复旧 Case 状态、Demo 入口或 Platform workflow；数据缺口不得冒充零影响，普通文字不得冒充输入卡片 | 路线/cost 由 pair/lane fact 自主部分复用；child 把 typed `needs_input` 交 Root，只有 Root 调用 Runtime 原生 `request_user_input`；模型不再选择重复的 plan/build/validate、standalone scenario 或四引用交付拼装路径 | Network Tool 拥有算法、pair facts、缺口终态与业务地图状态；Root Skill 拥有输入卡片调用；Maps 拥有展示样式；Provider ResourceStore 拥有中间内容和统一 GeoJSON ref/profile | Planner 188/188、Maps 38/38、stdio 双门、Copilot/Skill validate、Web 1348/1348、deterministic multi-agent 2/2 通过；真实 DeepSeek 复验需恢复 Provider 凭据后完成 |
| `P1-ART-FINAL` | 当前关键链 | generic ResourceLink→Artifact、Run-scoped通用 inline 表、Artifact 输入回流和旧 `report.v1` JSON inline 卡片已删除；durable delivery 只认 active Copilot `[[deliveries]]` 的 exact producer、固定 typed kind/schema/MIME/verifier 与 Workspace-relative descriptor。Platform 只理解通用 delivery kind，不理解仓网、地图或会议字段。Artifact 持久化 producer-time verifier snapshot，恢复和读取不依赖当前 active registry。仓网报告/地图文件/地图卡片与 meeting Markdown 报告是当前声明实例；Resource 字节仍由 provider 持有 | 把展示卡片误写成文件会触发不必要审批并制造交付物；把任意 Resource 升为 Artifact 会形成第二数据面；依赖模型文本或 active package 猜既有 Artifact 合同会在刷新、改写或切包时丢失交付 | producer 只返回其声明 kind 的固定 structuredContent envelope；Platform 按 producing Item provenance 与 verifier snapshot 物化/恢复。原生 inline-vis 通过授权 Profile/Thread 文件端点读取 HTML 或签名验证后的 PNG/JPEG/GIF/WebP；完成的 Agent Message 可用严格 `workspace_file` 指令请求 Platform 以权威 Workspace no-follow reader 快照 HTML，再投影为官方 native `file` 引用 | Platform 拥有通用交付授权投影、verifier snapshot、原生文件授权读取、Workspace HTML→Thread snapshot 与 Artifact 物化；Copilot 包拥有 producer 声明；Codex 拥有原生 visualize；MCP provider 拥有 Resource 内容 | 原生 Workspace HTML 快照定向 Rust tests 已通过；仓网自然语言案例已通过；meeting package/Tool delivery gate 已通过，真实 Web 交付未运行 |
| `B-PROVIDER-OWNER` | 后续 backlog | `apps/web/crates/provider-service/src/secured.rs` 的 `profile_provider_definitions`、`apps/web/crates/platform-store/src/configuration.rs` 的 global default 与 Profile config/Runtime 重复 | 默认选择、已物化 Thread 实际 pair 和 refresh 被不同 owner 覆盖 | 当前 R2 只补 exact 真实 gate 依赖；全量 owner 收敛仅在仓网 gate 证明会选错 Provider/model 时提前 | Profile config/Runtime 拥有定义、未来 Thread default 和 Thread actual pair；Platform 只留密文 Secret 与必要 audit | 已证实，不阻断当前关键链 |
| `B-THREAD-HISTORY` | 后续 backlog | `codex-adapter/src/real.rs` 的 history mode/隐藏 Thread，`routes/threads.rs` 的 overlay，`WebApp.tsx`/`webThreadHistory.ts` 的 live/history 启发式合并 | 连续同文本消息误去重，Task/Run status 可以冒充 official Thread/Turn 事实 | 全量 Thread/Run/history/lease 轻纠偏是后续 backlog；只有当前 E2E 证明消息投递错乱才同原子修复 | Codex Runtime 拥有 Thread/Turn/Item/history，Platform 只做授权和 exact Item 投影 | 已证实，不阻断当前关键链 |
| `B-EVENT-BOUND` | 后续 backlog | `apps/web/server/src/event_projection.rs` 和 Browser item renderer 仍按动态 Tool 名/文本猜 command/diff，unknown event 可进对话，多类 payload 无统一限长/retention owner | 事件污染、过大持久化与未授权内容暴露 | 保留 `run_events` sequence/provenance；普通 RunEvent 不广播 private Resource ref；若仓网真实 gate 证明泄密才成为当前安全门 | Platform 只投影 bounded typed Runtime 事实；unknown 不猜内容 | 已证实，当前仅保留泄密负向门 |
| `B-BROWSER-LEGACY` | 后续 backlog | 生产只有 `browser-entry.ts`→`WebApp`，但死 `main.tsx`/`App`、全量 client、Terminal/Usage/generation/prompts/preferences/raw Profile writer/记忆 approval routes 仍在 | 继续维护第二 UI/API 表面，其中 generation/规则/整文件写面可创建隐藏 Thread 或绕过 official CAS/approval | 统一收缩已记入 backlog，不再作为仓网前置；只在真实关键链调用到旧入口时前移 | Browser 只保留 WebApp typed surface；Server 路由按 production reachability 原子删除，不建替代子系统 | 已证实，不阻断当前关键链 |
| `B-PROFILE-AUTH` | 后续 backlog | `apps/web/server/src/main.rs::import_file_backed_codex_auth_if_missing` 在单 Profile 过渡模式可从宿主 home 导入 `auth.json` | 未来多用户身份边界不成立 | 当前单 Profile 不阻断；启用多用户前必须删除宿主默认导入并完成隔离矩阵 | Profile 认证由显式用户/Profile owner 提供，不读服务器操作者 home | 已证实，多用户触发 |
| `D-CONTROL-PLANE` | 已删除 | migration 55 和 fresh-schema gate 证明 Work State/coordination 十表、Supervisor snapshot/binding/continuation 六表及对应 crate/route/MCP/gate 不存在 | 曾与 Codex context/mailbox/scheduler 形成第二控制面 | 不建替代状态机 | Codex 拥有协作；Platform 仅保留 Agent/Approval/Audit 投影 | 已完成，需继续通过 fresh-schema denial |
| `D-CATALOG-MANIFEST` | 已删除 | Catalog/Studio/Python publish 生产链与无 owner 表已删；migration 56 删 `profile_capabilities`，ProfileHost 只验证 official initialize 四字段 | 曾在 Codex 原生 Skill/Role/MCP discovery 上叠加发布/安装/本地 manifest truth | 阶段一 built-in 只走 Profile seed + Runtime discovery；公开 Studio/SDK 后移 | Codex 拥有 discovery，Platform 不伪造 readiness | 已完成，公开平台另起后续阶段 |
| `D-PLATFORM-DATA` | 已删除 | SourceAsset/Data Intake/Dataset Release/Task binding/snapshot/projection 生产路由和当前 schema 已删，Workspace 文件面与 denial 集成通过 | 曾与 Workspace 文件和 MCP Resource 形成第二数据操作系统 | 不建设 replacement registry/revision/binding/cache | Workspace 拥有普通文件，MCP provider 拥有 typed intermediate | 已完成，Stage E 已删除仓网 Tool 内旧 Case/ref tail |
| `D-TRUST-STACK` | 已删除 | provider speculative SHA 四列、历史 SHA password compatibility、Capability Manifest 与 Work State snapshot/hash 链已从 production caller/schema 删除 | 曾在 official Runtime snapshot/config CAS 上重复叠加平台“可信”链 | official config CAS/MCP/Turn/Skill snapshots 原样保留，Web 不镜像；ResourceStore SHA 仅作不可见物理 ID/字节完整性 | 各 owning layer；无 production caller 即删 | 已完成，不得复活 |

索引中的当前关键链精确顺序仍以 [开发计划](development-plan.md) 为准；
`codex/` 中的 retain/drop/upstreamed 状态只以
[Custom Codex Patch Map](custom-codex-patch-map.md) 为准。下列详细证据保留对源码现象的完整说明：

1. Stage E 已删除供应链 Case/NetworkSnapshot/ArtifactRef 及旧 source/mapping/operation tail；
   Network active surface 只保留 strict `ResourceRef` 和 provider-owned ResourceStore；Data inspection
   使用 inline `workspace_source_inspection.v2` identity，不发布 source Resource。
2. Event Projection 把 generic ResourceLink 提升成 Artifact、跨 child 搜索 Resource，并解析模型
   文本指令；`inline_visualization_artifacts` 随 Run 级联删除，`retention_state` 又没有任何
   production writer，还没有收敛为显式 final-deliverable 合同。
3. Data active surface 只接受 inline inspection identity、Workspace-relative paths 和 confirmed
   decisions；历史 Data aliases/wire-shape cluster 已删除，正式 schema 由当前 typed contracts 唯一确定。
5. Profile Runtime HOME/process cwd 已隔离，但 Server 默认认证导入仍可读取宿主
    `$HOME/.codex/auth.json`。
6. 历史 `check-main-ui-parity` 第二 UI truth 已删除；legacy
   `main.tsx`/`App`/hooks/facade 仍在非生产维护面，需按生产调用图另行收缩。
7. Platform 以 `profile_provider_definitions`、global `models.default_selection` 和启动回放重复
   拥有 Codex Profile 的 Provider 定义、模型目录及未来 Thread 默认选择；Task pair 又被错误用于
   后续 Turn。非当前 Provider 刷新会改变 Profile 默认 Provider，普通 catalog 更新还叠加了一条
   只有 Provider 服务使用的 scheduled-restart 生命周期。
8. Server approval history overlay 会按 Tool 字段或 message 文本猜关联位置；Browser
   `mergeWebThreadHistory` 又按相同用户文本消除 optimistic echo。latest official 已提供
   `clientUserMessageId`→`userMessage.clientId` 精确关联，但当前 Platform DTO/adapter/WebApp 尚未使用。
9. Browser 会按动态 Tool 名称和输出文本把 generic Tool 猜成 command/diff，并把 unknown event
   JSON 作为对话内容；`run_events` 对多类内容没有统一 payload 上限或 retention owner。
10. legacy Terminal 路径把 Workspace 级终端会话强制挂到 `updated_at` 最新 Run；同一 Workspace
    存在多个 Task 或 fork 时会把终端输出投影到错误会话。生产 `/web` 尚未使用该能力，不应为其
    新建 Run selector；应在 legacy App 退出后删除未使用的 route、session schema 和事件投影。
    同一路径的 `/profile/usage` 又混用本地 `run_events` 近似值与 official account usage，并把
    Turn 数标成 Agent Run；它应随旧 App 删除，不能演化为另一套 usage owner。
11. legacy `/runs/{id}/generate` 由 Platform 写死生成提示词，并在 adapter 内创建不会进入正常
    事件投影的持久 official Thread；`suppressed_threads` 没有终止清理，archive 又只是 best-effort，
    因而 direct HTTP 可制造隐藏 Thread、吞掉官方事件并累积进程状态。generic adapter
    `rpc(method, Value)` 同时保留字符串 `start_thread`/`send_user_message`，虽无 production caller，
    仍会迫使 typed Provider/首消息合同保留旁路。两者都不应继续兼容新的 `thread/start` 合同。
12. `browser_workspace_preferences` 只服务旧 App，却继续持久化 untyped settings、未实际应用的
    Runtime 参数和由 Terminal 自动执行的 worktree setup script；这些字段没有当前 Workspace/Runner
    lifecycle owner，不能作为未来初始化或 Runtime 配置合同保留。
13. `/profile/prompts` 只服务旧 App，在 Platform 内自建 project/global 目录、扫描、frontmatter
    parser 和 move lifecycle；latest official Runtime 不发现这些目录，当前原生扩展面是 cwd-scoped
    Skills/Plugins，因此该 API 写入的文件不是可用 Runtime capability。
14. `/profile/approval-rules` 没有绑定 exact pending approval、Item、版本或 requested Profile，
    却接受 Browser 自报命令前缀并永久写 Profile allow rule；latest official 已让 approval 请求携带
    `proposedExecpolicyAmendment` 并以 typed decision 原路响应，因此该文件写入是额外授权旁路。
15. `/profile/files/{agents|config}` 把 Profile 指令和 Runtime 配置折叠为同一个 whole-file writer；
    config 分支直接 rename 文件后才做 `config/read` 探测，没有使用 official config write 的
    expected-version CAS/reload，因此可与 Provider/Agent 等合法配置写并发互相覆盖。

## 7. 当前阶段裁决

阶段一内置仓网 Copilot 的正常业务链已经完成：旧跨 owner 数据面与 compatibility island 已删，
Runtime/bridge、Profile Skill/Role、Workspace 文件、provider Resource、Approval、Agent activity、
`map.v3` 与最终 Markdown Artifact 在真实 Web 链中闭合。阶段二已用
`copilots/meeting-action-review` 验证同一 Copilot/Tool SDK 合同可承载一个非仓网领域，但该证据
只覆盖本地 validate、prepare、Runtime discovery、Tool focused test 与 native normal case。公开
SDK/Web 自助创作、该参考包的 Web 运行、多用户产品、完整失败/竞态矩阵、
a11y 与地图视觉细节仍是后续阶段或 hardening，不是阶段一完成定义。应用已把该包注册为
trusted source，可经 activate API 持久 desired；一次性真实 PostgreSQL 门已证明 revision
refresh、失败保留精确删除权、deactivate 清理与 Artifact 幂等物化/恢复。

## 8. 证据更新规则

以后只有满足以下要求，能力才能提高证据等级：

1. E2E 输出脱敏、机器可读 evidence，记录 commit、实际 Profile/Runtime build、Runtime
   discovery inventory、Workspace、Run、terminal、交付 Artifact 和测试结果；
2. E4 必须从干净数据库和干净 Profile 开始，不复制宿主 auth/plugin/MCP 状态；
3. E4 必须从阶段一业务 Task 入口开始，不脚本手建 Work State、Data Intake、Dataset 或在
   Prompt 注入内部 ID；
4. 阶段一 E4 通过 S1 数据准备、S2 完整规划、S3 exact Resource 场景复用，并通过一次关键 pending
   approval 整页刷新恢复；是否使用同 child follow-up 由 handoff 是否仍依赖未结构化上下文决定；
5. 失败、取消、超时、并发、多用户、完整跨 Workspace 矩阵、a11y 和视觉细节按后续 owner gate
   验证，不再反向改写已经通过的阶段一 normal-path 等级；
6. 文档只在证据完成后更新，不以模型最终文本或数据库 success 字段代替验证。
