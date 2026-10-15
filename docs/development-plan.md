# open-web-codex 开发计划

| 字段 | 内容 |
| --- | --- |
| 文档性质 | 当前与下一里程碑的执行计划 |
| 更新日期 | 2026-08-21 |
| 当前阶段 | 阶段二：公开 SDK 与 Web 创作体验 |
| 当前状态 | 多 Agent 仓网正常业务主链完成；目录发现、Task 显式选包、独立单 Agent 仓网包与根级共享 Tool 已落地 focused gate |
| 当前阶段裁决 | Codex 原生协作与 MCP Resource + Workspace 文件 + 最终 Artifact 混合边界；ADR-018/019/024/025 为当前基线 |
| 当前事实 | [Architecture](architecture.md) 与 [Capability Baseline](capability-baseline.md) |
| 阶段二后续 | Web Studio、Marketplace、生产模型质量与真实产品 E2E；多用户产品流程属于阶段三 |

本文记录阶段一已完成的正常业务主链、仍有效的边界和后续工作。用户心智统一为：

> Workspace 管理用户文件，Task 做一个独立方案，Copilot 通过原生协作和领域工具处理、复用并交付数据。

阶段一不建设平台数据操作系统。Task 之间没有直接消息、上下文、结果或数据接口；同一
Workspace 下的 Task 可按用户意图复用普通文件，也可通过同一授权 MCP provider 的精确
Resource ref 复用中间数据。保存什么、读取什么、怎样复用、裁剪、合并或覆盖，由用户
要求、Skill 和 Tool 决定，Platform 不理解仓网数据语义。

## 0. 阶段二当前切片：Copilot SDK、Task 选包与单 Profile 多包组合

Atom 1 先建立一个不依赖 Web、Catalog 或安装状态的开发者源码入口：

1. `copilot init` 在空目录生成默认单 Agent或显式多 Agent骨架；`copilot tool init` 生成可由
   多包共享的根级 Tool package。仓库开发者统一通过 `scripts/copilot.sh` 使用隔离 SDK 环境。
2. `copilot validate` 以调用者给出的显式 source root 为边界，静态验证 manifest、Skill、
   Runtime Role identity/reference 与 Tool package 引用，并输出有界摘要或 typed failure；
   它不证明完整 Role 可被 Runtime 加载，该 Runtime 门属于 Atom 2。
3. Copilot-owned Skill/Role 路径都相对于显式 package root；一个 package 只有一个 `[root]`。
   `copilots/warehouse-network` 与 `copilots/warehouse-network-single-agent` 是两个独立 package，
   分别拥有多 Agent Supervisor 和关闭 multi-agent 的单 Root Agent。
4. Atom 2a 的 `copilot dev` 在隔离 Profile 中复制完整 Skill 树和 child Role TOML；Tool 保持独立
   source owner。Copilot 可用 `{id, package}` 引用显式根级 Tool registry 的严格 `tool.toml`；
   仓网 Planner/Maps 当前位于根 `tools/`，不通过另一个 Copilot 引用。Tool 只声明直接项目 manifest、hash lock、
   Python module server、参数与 typed env binding；SDK generic provisioner 在 Profile、Tool source
   和 Workspace 外执行 bounded Python/Node build/install，缺声明、锁、宿主 adapter 或准备失败
   返回 typed `EnvironmentUnavailable`。SDK 不扫描文件猜语言或 server，不调用 Tool setup，也不
   在 Runtime launch 内安装。准备完成后，SDK 生成一次性 Plugin 投影，Tool 通过官方
   `thread/start.selectedCapabilityRoots` 从 exact projection root 选择。它用 app-server 官方握手、
   `skills/list` 一次性验证声明 Skill；Role/MCP 执行能力留给真实 Task/child MCP Item 验证。
5. `dev` 的 app-server HOME/cwd 使用 Profile 外的独立临时目录并始终清理；默认临时 Profile
   清理，`--keep-profile` 或显式 `--profile` 才保留。成功只报告 `discovery_ready`，Role spawn
   和 model acceptance 固定为 `not_run`。
6. `copilot test` 最多读取 16 个隔离用例，显式区分 Root 直调与 child Role；每例在临时
   Profile 的 durable Thread 中运行，以 canonical `thread/read(includeTurns=true)` 验证精确 MCP
   Tool、参数、结构化结果、Role/parent 和终态，不以实时 child 事件、最终文本或模型请求体
   作为 PASS。`copilot check` 串行执行 validate→prepare→dev→test，并在首个 typed 失败停止。
7. Atom 2b 增加单 Profile local/private package 安装正常链。应用启动配置只给可信 Copilot 根
   与 prepared 根，Server 从一级子目录的严格 manifest 发现所有包；Browser 只能请求已发现 ID。
   Platform 按 `(Profile, package)` 持久 desired active、source/configured revision、managed
   Skill/child Role IDs 与 safe failure；冷启动在
   app-server 前预检/stage/publish或精确清理 managed destinations。运行中变更返回
   `restartRequired`，不伪造热切换。仓库 `sync` 只接受可信根直接子包，完整 check 通过后才
   委托 `run-local` prepare/GC/冷重启并复核 descriptor identity；它不写 DB 或 Profile。
   最小 Web 管理面只展示状态并调用 activate/deactivate。即使源码先移除，持久 record 仍可停用，
   Host 成功冷启动后精确清理 managed Skill/Role，并保留 `Unavailable` 记录供恢复。
8. `ready` 不进入数据库；GET 状态只在当前 Runtime instance 对 trusted authorized Workspace
   调用官方 `skills/list(forceReload)` 后给出。单 Agent 只有 Root Role 时保持 `Configured`，
   明确表示配置已收敛但尚未观察 child/MCP 执行；不伪造 Runtime `Ready`，真实执行仍由 native
   spawn/MCP acceptance 证明。所有 package 共用一次 Skill discovery 投影；Role/MCP 不参与
   installation status，也不产生 Copilot `Ready`。
9. 当前仍不建立真实生产模型质量验收、Web Builder、Catalog、Release 或 Marketplace，也不把
   Settings Agents 误写成 Copilot 创作入口。
10. 存在可用包时，新建 Thread 列出全部 configured package 并要求用户显式选择；Task 固定
    `copilot_package_id`，Run/Thread/Turn 复用它。平台不提供单/多 Agent 模式开关，不按 Prompt
    或默认包自动加载仓网能力。历史读取、Turn 控制、名称、压缩、Review 和 fork 的 cold-resume
    同样从已授权 Task 传递该 ID；不得先恢复无包 Thread 再把它缓存为已绑定。

当前验证 reference 是：

```bash
./scripts/copilot.sh check copilots/warehouse-network --workspace "$PWD"
./scripts/copilot.sh check copilots/warehouse-network-single-agent --workspace "$PWD"
```

退出：新源码目录可由单/多 Agent `init` 生成并完成完整 `check`；两个独立仓网包从各自 source root
并通过同一根级 Tool registry 静态通过；runtime parser、hash lock、Python/Node provisioner、prepared descriptor 与
临时 Plugin projection 有 focused tests；fake transcript 覆盖 official RPC 顺序、参数、inventory
与 cleanup，真实 app-server fresh init→dev gate 证明 SDK-managed preparation 与同一 discovery 链；fresh init→test gate
证明本地确定性 Provider 下的 Root/child Role/MCP 正常链。仓网参考工程与 generated package 使用
同一个 generic prepare owner；`dev`/`test` 默认复用稳定 Tool 环境缓存，Skill/Role/提示词变化
不会重装未变化依赖。平台 real 启动遍历可信 Copilot 根，并为每个包调用同一个
`copilot prepare`，再把按 package ID 分区的内部 descriptor 交给 Server；不再按 supply/maps
或具体 Copilot 名称分叉安装。能力基线和教程明确本地 E2 gate 与未实现边界；生产
模型质量、Web 创作和 Marketplace 只有在各自 owner 的后续 Atom 具备真实证据后才能更新；
Runtime ready 刻意保持 instance-scoped observation，不建设持久 readiness truth。当前 SDK 124 项
单测、fresh single/multi real app-server `check`、Provider Cookbook 与开发者文档 smoke 已通过；
Web Studio、外部 push/import、Release 和 Marketplace 仍不在本切片。

地图配置复用现有 Settings、typed API 与加密 Secret Store：无论首次配置或替换均可从
“地图服务”打开专用弹窗。受管 Maps Tool 显式使用 platform credential source，每次凭据操作
从 Server 取得当前配置，不能沿用 provider-local 旧 Key；独立 CLI local source 不作为 fallback。

### 已验证的运行体验与低延迟领域操作基线

阶段二已经开始，但不从旧 Catalog/Work State 方案恢复实现。当前先处理真实用户运行中已经
测量到的两个普通问题：Agent activity/等待状态需要可理解，简单仓网 follow-up 的领域计算只有
毫秒级，却因多个模型回合和重复 child history 变成分钟级。当前修复保持以下边界：

1. 设施增减搬迁的单次影响评估由供应链 MCP domain owner 提供一个 typed 粗粒度 Tool；它以
   exact before result 为基座，一次求解后直接比较，并返回 scenario ref、绑定标准化输入与
   前后方案的单一 `plan_comparison_ref`，以及有界成本、覆盖和城市变更结果。
2. Supervisor 拥有 child 使用策略：同一 Task 内只有上一 Turn 已 `completed` 且仍适配后续目标的
   child 可以 follow-up 唤醒；当前请求中 child 或其 Tool 的 failed/errored/cancelled/interrupted/
   rejected/timeout 是 typed 终态，Root 只报告并停止，不能根据错误文本重派、续跑或新建 child。只有
   终态之后用户明确发起的新请求才按普通路由创建独立 child；正确性仍依赖 typed handoff，不依赖记忆。
3. 不新增 Platform workflow、handoff ledger、业务缓存、自动重试、固定案例流程或 Codex seam。
4. 先以 Python/stdio/Role/Skill 合同证明结果不变，再用用户原始 follow-up 请求做真实 Web 计时；
   记录 Tool 时间、模型调用数、输入 token 和总墙钟时间，不在测量前伪造性能承诺。

这一切片不会替代阶段二公开 SDK/Web 创作的退出条件；它先把现有 built-in 正常使用体验变成
可观察、可解释且不会因完整旧 child 历史持续膨胀的基线。

## 1. 阶段目标与边界

阶段一交付一个内置仓网 Copilot：Root、Data Agent 和 Network Agent 复用真实 Codex
Runtime，在浏览器中完成数据准备、距离与成本、覆盖、SLA、模拟、选址、地图和报告。
用户与一个 Copilot 对话；Root 在 child 运行期间仍可响应用户、接收补充要求和更新进度。

阶段一必须同时完成：

1. Task 在创建时固定一个授权 Workspace，Root 与所有 child Thread 使用该 Workspace。
2. 直接复用通用 Workspace 文件列举、上传、读取、下载、删除和原子写入能力，并复用
   Codex 官方 MCP Resource/ResourceLink/read_resource 语义。
3. Data/Network Tool 以 Codex 原生 sandbox metadata 解析 Turn Workspace，对用户文件使用
   经校验的 Workspace 相对路径，对 provider-owned intermediate 使用 typed Resource ref。
4. Supervisor、Roles、Skills 和 MCP tools 由 Profile 托管，并通过 Codex 原生发现与热刷新
   生效，不走正式发布流程。
5. child 发起的结构化问题直接通过原生 MCP elicitation 到达浏览器，答案返回原 child
   request；Platform 不中继、不续跑、不全局 interrupt。
6. 默认印尼 Mock 完整链和真实 Excel/CSV/JSON 交互链均通过 Web E2E。
7. 删除 Work State、Data Intake、SourceAsset、DatasetRelease、DomainResource、Resource
   Broker、Case store、generic Resource→Artifact、Artifact 输入传输和 Platform continuation 等旧路径。
8. Web 可从 Root Task 查看 child Agent 的官方 Thread/Turn/Item 执行过程；实时与刷新恢复必须
   一致，不得把不同的 command、MCP Tool、进度、等待、审批和终态统一压成通用占位文案。

阶段一不交付公开 SDK、Studio、五对象 Catalog/Release、Marketplace、第二领域或多用户
产品流，也不建设 Workspace 数据对象、数据 revision/head、Schema registry、Task 数据绑定、
Task 私有文件库、COW/worktree、Platform 计算缓存或整数据集语义指纹、Blackboard、第二调度器、durable
mailbox、Workflow DSL、Run Completion Controller、签名链或 exactly-once。

## 2. 唯一所有权合同

| 事实 | Owner | 阶段一规则 |
| --- | --- | --- |
| Thread、Turn、context、spawn/followup、wait/mailbox、steer、child 终态 | Codex Runtime | Platform 只桥接和投影，不重新实现 |
| Skill、MCP、Role config 发现与刷新 | Codex Runtime；Profile Host 负责托管固定内置 Skill/Role 与 MCP activation | built-in 不使用 Plugin/selected roots；观察结果必须来自当前 Runtime |
| 仓网 Tool 代码与只读 Mock 源 | 显式配置的共享应用资产 | 不复制到 Profile，不从 cwd、Workspace 或源码树扫描 |
| Task 与唯一执行 Workspace | Platform | Task 创建后固定；Run、resume、fork 不得改选其他 Workspace |
| Thread cwd 与 MCP sandbox metadata | Codex Runtime | Root/child 都继承授权 Workspace；不新增 Codex seam |
| 普通数据文件 | Workspace 文件系统 | 同 Workspace Task 天然可见；没有 Task→文件 binding 或数据生命周期 |
| typed intermediate 内容与生命周期 | MCP provider | 通过官方 Resource URI/template/read 使用；Platform 不存内容或建通用 Broker |
| Workspace authority | Platform/Runner | authenticated Workspace capability、canonical containment/no-follow、final file atomic create-new 与 Artifact 授权物化 |
| 通用 Copilot provider primitives | `packages/copilot-provider-sdk` | 独立提供 `ResourceRef` envelope、expected-schema validation、canonical codec、payload bounds、typed errors、Workspace scope/writer 与 provider-scoped load/publish；Tool 通过 `runtime.toml.platform_packages` 声明，generic provisioner 从单一 SDK registry 注入，不依赖仓网路径 |
| Resource ref 可发现与授权 | Codex official Item + 有界 Platform projection | 只投影 completed Tool Item 的 exact ResourceLink provenance；同 Profile+Workspace 的用户显式选择带 producer event/ordinal 与完整 `{server, uri, resource_schema}`，Server 逐字段复验；不读或保存内容、不猜模型文本 |
| 文件选择、字段、映射、标准化、复用和合并 | 用户 + Skill + Tool | Platform 不分类、不猜测、不限制业务复用；Resource 是可消费的不可变计算快照，不代表来源真实、业务可信或自动复用许可 |
| Agent 协同方法 | Supervisor Skill | 不落 Platform workflow 状态机 |
| 仓网规划能力 | Data/Network MCP domain Tool bindings、Role/Skill 与 pure owners | 阶段一统一拥有 Data mapping/normalize/geography、route/cost facts与matrix、baseline/scenario/p-median/comparison 以及 final map/report；输入输出为 Workspace 相对路径、typed Resource ref 或普通业务参数，不拥有通用 scope/store/codec/writer。未来公共供应链领域层只记为触发式 TODO，不增加当前实现层、提交线或验收节点 |
| Agent 活动与问题卡片 | Codex Runtime 提供 child Thread/Turn/Item 事实；Platform projection 只做授权、安全裁剪和浏览器 DTO | 保留 item type、Tool 身份、bounded action/result、Runtime-provided reasoning 文本、生命周期、错误与时间顺序并容忍乱序；不另建 child 日志、执行历史或状态机，不用通用占位文案替代可安全展示的官方事实 |
| 地图、报告和明确交付件 | Artifact | 只展示和下载，永远不作为 Task/Agent 输入或交换介质 |

### Platform 只执行的硬约束

- 用户、Organization、Profile、Workspace 的授权与 scope；当前单用户也不得退化为无 scope
  全局状态。
- Task 所属 Workspace 与 Runtime `cwd` 的一致性；浏览器不能提交服务器绝对路径。
- Workspace 文件相对路径、大小、regular-file、符号链接逃逸和跨 Workspace访问检查。
- Task/Run/Approval/Artifact 的持久化、浏览器 DTO 与安全事件投影。
- SHA 若存在，只用于字节完整性、ETag 或 provider 物理去重，不作为授权或整数据集复用许可。

### Tool 执行的硬约束

- 允许用户、Skill、模型与 Tool 使用 Workspace 相对路径与 typed MCP Resource ref；拒绝
  绝对路径、`..` 逃逸、含义不明的 `source_ref` alias 和历史 asset/Dataset ID。
- Data Tool 对 Excel/CSV/JSON 的格式、结构和业务字段负责；Platform 不保存映射状态。
- 写文件使用临时文件和同目录原子 rename；默认 create-new，不静默覆盖已有文件。
- 文件重名时通过原生 elicitation 询问覆盖、改名或取消。
- 外部导航前展示路线数量、预计调用量和费用风险，并取得用户许可。
- 路线和成本按 exact pair fact 复用已有真实计算，只计算缺失 pair，并分开报告
  reused/computed 数量。
- 中途失败不得留下看似完成的结果文件。

### Skill 负责的行为

- 读取哪些文件、何时询问用户选择文件、如何解释缺失和歧义。
- 哪些中间数据或结果写入 Workspace，哪些保存为 MCP Resource，以及对用户如何呈现。
- 如何复用已有文件或 Resource、复用多少、如何裁剪、转换、合并或补算。
- Data/Network Agent 的职责、顺序、业务问题、默认值和交付结构。

## 3. 已验证且优先复用的原生基础

- 通用 `/workspaces/{id}/files` 已提供列举、上传、文本读取、二进制下载和删除；
  `GitRuntime` 已有 Workspace 授权、相对路径、symlink escape、单文件原子写和审计。
- Codex MCP runtime 已携带 Turn Workspace；`native_workspace_probe` 已证明 Root 与 native
  child 的 Python FastMCP 调用从 `codex/sandbox-state-meta.sandboxCwd` 解析业务 Workspace，
  Plugin 进程 cwd 没有被当作业务 Workspace。
- native child completion/mailbox、Root early final、forked context、MCP elicitation、Skill
  reload/watcher、MCP runtime refresh、fork/LRU/restore、Stop 和 PreToolUse 定向测试已通过。
- `selectedCapabilityRoots` 的 executor discovery 是 Thread-scoped 且没有 invalidation，成功与
  失败都会缓存；它不能作为 built-in 热加载基础。阶段一改用 `$CODEX_HOME/skills` 和
  `$CODEX_HOME/agents/*.toml`，MCP transport 只放在 Data/Network Role TOML 中，不走
  Plugin/Marketplace/Installation 或全局 Profile MCP 配置。
- Role 文件内容由 Codex 在每次 spawn 时重读；既有 Root Thread 的 Role 集合、allowlist 和
  spawn limits 不会随 user config reload 完整重算。内置 Data/Network Role 必须在 Root Thread
  创建前注册；内容修改对下一次 spawn 生效，角色增删改名只保证对新 Root Thread 生效。
- `SubAgentActivity Started` 不保证早于 child 首个 tool event；Platform projection 必须按
  thread identity 幂等并容忍乱序，不能据此申请新 Runtime seam。

该 Probe 已使用当前 checkout 构建的 Codex app-server、真实 Profile Host、native
`collaboration.spawn_agent`、官方 `turn/steer` 和 Python FastMCP 通过；阶段一没有因此新增
Codex seam。

## 4. 实施顺序

### Slice 0：同步轻量基线并完成 native Workspace Probe

状态（2026-08-09）：已完成。权威文档已统一到 ADR-018；native Workspace Probe 通过；
没有新增 Codex seam 或未消费数据框架。

1. 以本计划重写 ADR-018、Architecture、安全模型、能力基线和相关当前文档；撤销此前
   Workspace revision、cache、binding、typed publish/adopt 与 TaskSourceAsset 结论。
2. 在 root 与 child FastMCP tool call 中断言 `sandboxCwd` 等于授权 Workspace，同时断言
   Plugin process cwd 不能成为业务数据根。
3. 固化现有 wait/mailbox/steer/elicitation/reload 证据；Patch Map 明确阶段一不新增 Codex
   seam。
4. 从目标设计删除 Dataset/DomainResource/Resource Broker、Schema registry、logical head、
   read binding、semantic fingerprint/cache、Task file store 和 ExecutionBinding 业务扩展。

退出：权威文档不再要求 Platform 数据 registry/binding/fingerprint/cache；native Workspace Probe
通过；不存在为以后保留的未消费数据框架。

### Slice 1：Task 固定执行 Workspace

状态（2026-08-09）：核心合同与执行不变量已完成并通过定向验证。Task 创建固定唯一授权
Workspace；Browser 不再为 Run 提交 Workspace；数据库、HTTP、orchestrator、fork 执行与
恢复路径均强制 Run Workspace 等于 Task Workspace。跨 Task/Workspace fork 已拒绝。
同 Workspace 多 Task 的普通文件读写、路径拒绝和文件冲突矩阵将在 Slice 2 的真实文件产品
流中一起验收，因此阶段一整体仍未完成。

1. 在当前 Task 合同中直接保存唯一 `workspace_id`，不新增 binding 表。
2. 创建 Task 时验证 Project、Profile、Workspace grant；StartRun 从 Task 解析 Workspace，
   Browser 不再为每次 Run 自由选择另一个 Workspace。
3. Run 继续记录实际 Workspace 作为执行审计，但必须等于 Task Workspace。
4. start、resume、fork、follow-up 均通过官方 app-server 合同使用同一 Workspace；child 使用
   Codex 原生继承，不依赖异步 projection 决定权限。
5. 不创建 Task-owned checkout、worktree、clone、overlay、snapshot 或 COW。

退出：同 Workspace 的两个 Task 能读取同一普通文件；Indonesia 与 Thailand Workspace 无法
经相对路径、绝对路径、`..` 或 symlink 交叉读取；Task 恢复后仍使用原 Workspace。

### Slice 2：收敛通用 Workspace 文件产品面

状态（2026-08-09）：已完成并通过定向验证。普通 Workspace 文件已经是当前用户上传、浏览与编辑的产品数据
面；旧 Platform Data Intake/SourceAsset/Dataset/Task attachment 生产路径、冻结能力包与旧
E2E 入口已删除。阶段一整体仍未完成，下一切片是 Profile 托管原生热加载。

1. 直接复用现有 `/workspaces/{id}/files` 与 `GitRuntime`，只补真实缺口，不建设新文件服务、
   文件 registry、数据页或第二套选择器。
2. Web 统一通过 Workspace 文件面板上传、搜索、查看、下载和删除；Task 对话只允许引用
   已有 Workspace 相对路径，不再上传或附加 SourceAsset。
3. 上传成功后文件立即作为普通 Workspace 文件出现；同内容不同路径仍是两个普通文件。
4. HTTP 每次只接收一个 multipart file；Web 多选逐文件处理。已有目标返回 typed conflict，
   用户对当前文件选择覆盖、改名或取消；不承诺批次原子、内容寻址、版本或输入快照。
5. 删除 Data Intake Draft/session/mapping/gate/hold、SourceAsset 表与专用文件 API、
   DatasetRelease 路由/表、Task dataset/attachment/snapshot/projection、`analysis_start`、
   `source_asset_ids` Prompt 注入和相应 UI/fixture/tests。

退出证据：fresh DB migration 断言 Platform 数据库不存在 Task→文件关系和旧 Data Intake/
Dataset 对象；页面不再出现 Dataset、Release、binding、revision、source asset 入口；同
Workspace 两个 Task 可读取同一路径文件，另一个租户的 list/content/delete 均拒绝；absolute、
`..`、symlink 与同名写冲突由 Runner/HTTP 测试覆盖；多-part request 在落盘前拒绝。

### Slice 3：Profile 托管的原生热加载

状态（2026-08-09）：已完成。3A 已审计原生能力；3B.1 已交付 clean Profile native
Skill/Role seed、显式共享应用资产 composition 与四个 MCP 的真实 stdio inventory probe；
3B.2 已删除 built-in/product Thread 的 selected-root 注入，并通过 clean Profile Runtime
discovery/native spawn/mailbox/reload/hot-boundary gate；3B.3 已把 Run admission 收敛为
Standard，删除 Governed runtime、RunStartPreflight、`platform-agents` 第二启动系统、
Catalog/Studio/Python publish 生产系统及其失去 owner 的旧 DB schema。阶段一整体仍未完成，
下一步只进入 Slice 4 的原生协同与第二调度器清理。

1. 把内置 Supervisor、Data/Network Role 与 Skills 作为固定的 Profile-owned 仓网能力托管；
   Tool source 只持有 runtime/dependency 声明、领域代码与只读 fixture。不建立
   Release/Installation、registry、版本状态机或哈希信任链。Supervisor 是 Root Skill，不新增
   独立发布对象。
2. 使用 Codex 原生 `$CODEX_HOME/skills` 与 `$CODEX_HOME/agents/*.toml`。每个 Copilot 只有一个
   小型常驻 Root Skill；任务 Skill 通过 Runtime Catalog 的 name、description、short-description 和 locator
   渐进发现，只有原生选中后才加载完整正文。Role TOML 源码只持有 Plugin/server policy；选中的 server 内 Tool
   一律通过 deferred `tool_search` 发现，不维护 `enabled_tools` 名称白名单；完成的 client ToolSearchOutput 由 canonical Thread history 保留，并由 Chat bridge 投影到后续 request，已加载且当前 request 可见的 schema 可直接复用；只有所需 Tool 未在当前 request 中出现时才重新搜索。已关闭 child 恢复后必须用 `send_input.items` 重新注入精确任务 Skill，Skill item 重新注入与 Tool schema 历史复用分开；同一 Task、同一业务 Role 默认复用稳定 child target，禁止用纯文本 follow-up 丢失任务 Skill。SDK prepared descriptor 提供 transport，Server
   在当前 Profile 下解析 typed binding 后合成 Role-local MCP。不改 Profile `config.toml`、全局 role allowlist
   或 Codex built-in roles。Profile Host 的普通 startup file 只为 clean Profile seed 默认文件
   并保留已存在内容；仓网三项内置 Skill 与两项内置 Role 是显式 managed 保留 ID，部署升级
   在 Runtime 启动前更新到当前 checked-in 内容，不覆盖其他用户 Skill/Role。Server 绑定经过
校验的 prepared descriptor；Server 还要求 command/dependency 全部落在明确配置的 shared build store 同一 fingerprint build 内；Runtime 不扫描 process cwd、Workspace 或源码树。缺少 runtime
   声明、lock、prepared transport 或 host adapter 时明确 unavailable。
3. Root、Data、Network Role 都显式关闭上游 multi_agent_v2，仓网只使用当前已验证的原生协作合同。Root 只暴露原生多 Agent 面，不配置全局仓网 MCP；Data/Network Role 通过原生 Role
   config 启用各自 Skill、MCP server、deferred Tool discovery 和精确 Tool approval policy。Data4 全部
   有界本地预批准；Network 预批准本地路线/成本/验证/分析/选址/比较与 `publish_network_planning_report`，`map_utils` 预批准
   `create_network_map_card`、`create_map_card` 与 `revise_map_card`；外部地图请求和 final Workspace 地图导出保持 `prompt`，不把全局
   `approvalPolicy` 降为 `never`。prepared transport 不固定 MCP cwd；Runtime 使用 Thread
   已授权的 Workspace 作为 stdio MCP cwd。maps
   credential/resource state 位于 Profile 私有 runtime，业务 Workspace 只能来自 native
   `sandboxCwd`。Data→Network 只交接完整 `prepared_network_input.v2` 的 Workspace 相对路径和
   `input_identity`；Network Tool 在授权 Workspace 内验证和读取该准备输入，不扫描或重解析 raw 数据。
   矩阵、成本和方案继续由同一 Profile+Workspace `supply_chain` Resource owner 保存，并绑定输入身份。工具代码、
   共享 venv、Node 依赖、Mock、测试和缓存均不复制进 Profile。
4. Skill watcher/`forceReload` 对下一 Turn 生效；Role 文件修改对下一次 spawn 生效；MCP
   reload 在安全 step 边界切换。Role 集合、allowlist、增删改名只对新 Root Thread 保证，
   不修改 Codex 弥补这一边界。
5. Runtime 观察只使用一次 `skills/list(forceReload=true)`、`config/read`/config warning。Copilot
   installation 状态只称 `installed/configured/unavailable/failed`，按 package 声明 Skill 交集投影；
   不保存或伪造 Copilot ready。真实 Role/MCP 执行能力由 Root 对 Data/Network 的 native spawn 与
   child MCP completion gate 证明。
6. built-in/product Standard/fork 路径已经删除 capability path scan 与 selected-root 主动
   注入。3B.3-B2 进一步删除 `RunStartPreflight`、Governed Agent/Supervisor mode、旧
   `platform-agents/<definition>/<version>` writer/verifier 与 request-scoped SHA Role/inventory
   校验。3B.3-B3 已删除 Catalog/Studio/Python publish crate、route、DTO、client、UI 与
   Workspace package publisher；3B.3-B4 又删除失去生产 owner 的旧数据库对象。保留 Codex 内
   已登记的 selected Plugin policy seam，但 built-in 不调用它。

3B.1 已有证据：Profile startup seed/managed 单测 6 项、Server composition 单测 5 项均通过；真实
stdio probe 启动 Data/Demo/Network/maps 并断言 Role 所需 inventory，两个 Tool source
启动前后无新增文件或 mtime 变化。该 probe 不是 Runtime discovery/native spawn gate。

3B.2 已有证据：使用当前 checkout 构建的 Codex、真实 Profile Host、生产 built-in
composition 与本地 mock Responses provider，clean Profile 的官方 `skills/list` 精确发现三项
仓网 Skill，Root 不发现仓网 MCP，Data/Network native child 只看到各自 Skill 和精确
`4`/`12+9` MCP tool inventory。Root 每个 Turn 由 adapter 使用官方 typed Skill input 直接注入
Supervisor 正文并隐藏通用 Skill 目录；child 继续由 Role 只展示唯一启用 Skill。原生 wait/mailbox、同 Data child follow-up 与所有 terminal
通过；Profile Skill 修改触发 `skills/changed` 并由 force reload 读取，Profile Role 修改只影响
下一次 spawn。既有 child 的无 delta MCP reload 在 safe boundary 保持同一 inventory，不伪造
startup transition。malformed Role 发出 config warning，spawn 明确失败、不创建 child，Root
仍完成。Profile Host 的私有 HOME 与 canonical 0700 neutral process cwd 阻止宿主
`$HOME/.agents` 和 Runner/源码祖先 `.codex` 污染 clean Profile；Thread cwd 仍是授权
Workspace。该 gate 证明 built-in Runtime composition，不替代仓网业务 E2E。

3B.3-B2 已有证据：Adapter、Profile Host、Run Orchestrator 和 Server
定向测试通过；唯一启动调用是 Standard start/fork。真实 Indonesia/Thailand 探针通过 Profile
原生 Skill/Role/MCP seed 启动 Root 与 child，二者只读取各自授权 Workspace，MCP process
cwd 与 Root/child 各自的 Thread authorized Workspace 一致，Workspace 中没有 capability
文件。clean Profile activation/hot
boundary、malformed Role 和 B1 fork/idempotency PostgreSQL 门均继续通过。

3B.3-B3 已删除 `capability-catalog`/`supervisor-catalog` crate、所有 Agent/Supervisor/
Instruction Policy definition 与 Python capability publish route、Browser authoring DTO/client、
Agent Studio/三项 Settings Studio，以及 GitRuntime 的 `.open-web-release.json` package publish/
verify 特殊语义。随后 Slice 4B.2-A 删除了 `/runs/{id}/supervisor-policy` 只读投影；
原生 Profile Agents/Skills/MCP、普通 Workspace 文件和通用 Artifact 保留。

3B.3-B4 新增 current-schema migration，按显式 FK 顺序删除 Capability Catalog
Draft/Release/Installation、Agent definition/release/run binding、Workspace capability package
release 与 Supervisor instruction policy release 共 14 张无 owner 表。fresh disposable PostgreSQL
从完整 migration 链初始化后断言这些表全部不存在。Slice 4B.2-A 的 migration 55 继续删除
Supervisor policy/definition/release/continuation 六表和 Work State 十表；fresh gate 保留
Runtime agent projection 与 provider_call_metrics，并断言 provider speculative hash 四列
不存在。Slice 4B.2-B 已由 migration 56 删除 `profile_capabilities`，并从 Codex initialize、
ProfileHost、Server 和 Web 删除本地 capability manifest；ProfileHost 只 typed 校验官方四字段
和 owned `codexHome`。测试库随后被精确删除。

退出：干净 Profile 在 Root Thread 创建前完成固定 Role 注册并发现四项 Skill；Data/Network
native spawn 成功且只看到各自允许的 MCP tools；Skill、Role 内容和 MCP 的上述热更新边界有
真实门；缺失任一 Role/Skill/tool 或发生 config warning/reload/status 失败时明确 unavailable，
不产生或沿用 false readiness；Workspace 中没有 capability 文件。

### Slice 4：原生协同与 child elicitation

状态（2026-08-09）：4B.1 已完成 standard MCP typed form 的 child→Approval→原 child 恢复
闭环；4B.2-A 已删除 Work State/coordination 第二控制面、Supervisor snapshot/binding/
continuation、Event Projection 主动 continuation/Root Turn、provider speculative hashes 和
历史 SHA password compatibility；4B.2-B 已删除非 official capability manifest、
`profile_capabilities` 与 generated Web capability bundle 链。fresh PostgreSQL security gate、native
child projection 与 Workspace reuse gate 均通过。阶段一不再把 Browser legacy 清理、Platform
Provider 全量去重、首消息 compound selection 或完整 4B.3 Thread truth 当作仓网前置。它们保留在
后续 backlog，只有出现会直接使当前仓网链不可达、写错数据或泄露 Secret 的可复现失败时，才与
触发该失败的最小原子片一起修复。

当前唯一关键路径固定为：

1. **A — Codex Runtime 最小封口。** R2 的 Chat transport 已完成
   `tool_search → spawn_agent` 兼容门；D3 又把能力声明收敛为既有 Provider `models` 配置中的
   typed exact `ProviderModelConfig.supports_search_tool`，由 `ModelsManager` 合并到原生
   `ModelInfo`，未配置默认 false，不按模型名或普通 `/models` ID 推断。Platform DTO/API/UI、Profile
   恢复和 refresh 保留同一 capability。真实 DeepSeek D3 最小 `tool_search → spawn_agent` 门已通过；
   完整仓网门在 Data MCP 后以 typed `copilot_chain_incomplete/map_producer_item_not_projected` 停止，
   不能伪造 deferred MCP 或最终业务结果。
2. **B — Server 实际适配。** 只完成通用 Copilot 运行对当前 Web Server 必需的 typed bridge 与
   owning-layer 适配，包括把官方 child Thread/Turn/Item 生命周期投影为可追踪的 Agent activity；
   不建立第二 Runtime、第二上下文、第二执行日志或固定业务流程。
3. **C — 通用可组合 Tools。** 统一 `supply_chain` provider 和单一 `ResourceRef`，Data4 已完成
   strict inspect→confirmed-normalize→optional-geography active surface；Network12 active surface
   已完成 provided/haversine/navigation 路线、route/cost pair-level 复用、双覆盖口径以及 final
   地图卡片、map 文件与单份中文 Markdown 简报的最小交付合同；正文只显示关键结论和 durable Artifact 授权下载链接，不复制整份简报。Stage E non-active compatibility tail 已完成尾删，后续只维护
   Platform 业务状态。
4. **D — 完整案例验收。** 已用 clean real Web 的 S1/S2/S3 验证 Indonesia 数据准备、完整规划、
   同一 Network child 场景复用、地图卡片、唯一 Markdown Artifact 和一次 pending approval 刷新恢复；
   案例顺序没有进入 Platform workflow。
5. **E — 旧 caller 尾删。** 已原子删除 Network non-active compatibility tail 中的
   `CaseRepository`、`NetworkSnapshot`、`ArtifactRef`、旧 services/models/tests 与 Demo launcher
   surface，不保留兼容双路径；Data4 与 Network active `ResourceRef` surface 不回退为 aliases。

真实 DeepSeek 生产门当前已拆为两个业务门和一个独立诊断门：fresh local session 通过正式 Provider API
为 exact `deepseek-v4-flash` 持久化 `supportsSearchTool=true`，配置只保存 env ref。多 Agent 业务门只
验收 typed child、Data→Network 准备输入交接、12h baseline、Balikpapan 两种变化率和地图；单 Agent
业务门验收完整报价 typed evidence、无 child 和 `minimum_feasible` 的 90% coverage；独立诊断门才运行
`tool_search → spawn_agent`。业务门不把固定 Tool 次数或顺序当作成功条件，协议/权限/身份/服务器/清理
失败均为 typed terminal failure。`observe` 模式不改写 `tool_choice`；所有门共享 bounded timeline 和
终态清理，deterministic multi-agent gate 的严格 topology、Tool 顺序与 Resource inventory 仍只属于
`real-platform-e2e.mjs`。
2026-08-21 ADR-024 最终复跑中，多 Agent 业务门为 40 个有效 Chat 轮次、133.552 秒；单 Agent
业务门为 26 个有效 Chat 轮次、109.585 秒，并完成 580 条报价 evidence 绑定与 90.0521% 需求加权
覆盖。旧门对应基线为 Multi 26 轮/91.768 秒、Single 24 轮/84.851 秒，第一批拆门基线为 Multi
26 轮/98.407 秒、Single 23 轮/97.383 秒。Data 标准链已缩为 discover→inspect→prepare，单设施
路线已精确到候选 ID，最小仓数已收敛为最多两个 CP-SAT stage，共享 Tool build store 也从约
10.2GB 降到约 405MB；但 DeepSeek 的额外计划、搜索和参数纠正使模型轮次与墙钟时间没有下降。
后续模型层加速必须用新的真实测量驱动，不能重新向 prompt 填回固定 Tool 顺序，也不能把内部计算
加速误报为端到端加速。
浏览器响应、日志、Workspace 与普通 Profile 文件均不得出现 Secret 明文；不写 `model_catalog_json`、
不按模型名推断、不重试提示词、不回退 single-agent。后续真实空目录故障已用窄 follow-up 收口：Codex 只增加 exact
Provider 的 fresh typed catalog，Platform 不切换 current Provider，只在非空成功后持久化目标目录并在
刷新/重启后恢复；Turn Provider override、Provider owner 全量重构和 Browser dead graph 仍不属于阶段一
关键门。

D6 已收敛仓网 package 的执行面：Root、Data、Network 的 native Role config 显式设置
`features.shell_tool=false`，只对该 package 生效；Root 仍保留协作、`tool_search` 和原生用户输入，
child 仍保留各自的 typed MCP surface。Data 与地图 Skill 在成功终态后立即交接或交付，禁止继续
探索性 Tool 调用。真实 DeepSeek 诊断 timeline 现能安全记录地图 Tool 的 typed terminal、bounded
error/result summary 与 producer provenance；成功地图调用按 `create_network_map_card` 的 completed
Item 验收，不把后续模型误选低层 Tool 或求解失败伪装成地图成功。deterministic multi-agent gate
连续两次通过；真实 Provider 仍可能因模型未作出下一结构化调用或触发审批而 typed 终止，完整真实门
只在完整 Data→Network→12h 基线→Balikpapan 增仓时效变化率→map canonical 链实际完成时计为通过。

后续 backlog 继续遵守已确认边界：official Thread/Turn/Item/history 是唯一会话事实；Browser legacy、
Terminal/Usage/prompts、Provider 重复 owner、Task creation selection、Run lease/history overlay 与
physical-cwd join 必须按 owner 原子收敛，但不得再次插到上述仓网关键路径之前。

阶段二已把领域无关的 ResourceRef/schema/codec/bounds/error/store/runtime/Workspace file
primitives 从仓网包迁到 `packages/copilot-provider-sdk`。Tool 的 Python 项目 metadata 与
`runtime.toml.platform_packages` 共同形成声明合同；Copilot SDK 的单一 registry 解析已安装的
平台 distribution，并由 generic provisioner 注入外置 Tool 环境。仓网不再拥有安装器、通用
Resource/Workspace 模块或 repo 路径 fallback；MCP provider 继续拥有 Resource 字节和生命周期，
Platform/Runner 继续拥有 Workspace 授权与 Artifact 物化。

Indonesia 完整验收可以组合为：Root 确定国家和目标；Network child 定义数据要求；Data child
检查与准备文件；Root 使用 native wait/mailbox 并保持可与用户互动；再由 Network child 完成
所需分析并由 Root 综合交付。这只是覆盖完整能力的一种验收组合，不是固定 Supervisor workflow；
实际 Agent、工具和先后顺序由用户目标、当前上下文与 Skill 动态决定。

2026-08-11 已补一条真实上传、自然语言“当前覆盖”回归证据：Web 中原样发送
`根据我上传的数据，评估当前仓网 24 小时时效覆盖率`，总耗时 293.112 秒。Root 直接创建一个
Data child 和一个 Network child，没有单独的需求定义 child、`request_user_input` 或失败 MCP
调用；Data 首次即以两位国家代码发布 ready Resource，Network 只调用一次 final report Tool、
一次地图卡片 Tool，唯一审批是 create-new 写正式简报。结果为实际当前归属 29/50 城（58.0%）、
需求量加权 74.5%；对话地图卡片 Ready，最终只有一个 2,913 字节中文 Markdown Artifact，正文
区域从授权 Artifact DTO 显示可点击下载链接。该证据只提高当前覆盖纵向切片，不替代三个产品
样例、恢复矩阵和跨 Workspace 拒绝。

2026-08-12 从 clean DB/Profile、真实 Web Task、真实 Codex Runtime 与真实 Provider 完成阶段一
三轮正常链。S1 观测 164.782 秒：Wanwan 产出 ready 的 `normalized_network_input.v1`，包含
50 demand、11 existing、12 candidate、50 current assignments 和 580 quotes，Network/map/report/
Artifact 均为 0。S2 观测 302.141 秒：Harbor 选择 Kendari 与 Manado，13 active，成本由
55,815,960,000 降至 18,579,390,907.47 IDR；6/12/18 小时需求加权覆盖由
55.8%/67.3%/72.4% 升至 73.9%/83.2%/96.6%，城市覆盖由 34%/44%/54% 升至
52%/70%/90%；最终只有一个 3,930-byte 中文 Markdown Artifact 和一个 ready 的 `map.v3`。
首次报告调用因不存在的父目录被 typed 拒绝，随后在 Workspace 根正常完成且未产生重复 Artifact。
S3 观测 136.828 秒：Root 恢复同一个 Harbor，只执行场景评估与比较；关闭 Bekasi 后成本及
6/12/18 小时覆盖不变，affected/reassigned=1/1，geography/p-median/map/report 增量为 0，
Artifact delta=0。三轮浏览器观测共 603.791 秒，含轮询间隔。

另一个同日门在 final report approval pending 时执行整页刷新，恢复同一授权 Workspace、Root
Thread、history、Agent activity 和唯一 pending approval；接受后原 Network child 正常完成。
用户跳过 Mapbox 视觉细节，因此这里只认 typed `map.v3` Ready，不声称地图像素或交互视觉通过。

当前用三个彼此独立的产品样例证明这一点：

1. **数据理解与准备。** 只使用授权 Workspace 中的用户数据，完成检查、映射、标准化与必要的
   地理信息准备；不启动 Network 分析。
2. **Indonesia 完整组合。** 在用户明确选择示例后完成 baseline，引入两个候选仓，计算分配、
   6/12/18 小时覆盖、成本，并生成地图和报告；验收业务结果与交付，不规定 Tool 顺序或固定数值。
3. **最短场景子链。** 复用同一份已准备数据，只关闭 `WH-CROSS_DOCKING-BEKASI`，比较 12 小时
   覆盖、成本与重分配；不重新运行选址、地图或报告。

样例 1 与样例 3 是轻量回归，样例 2 是 Stage D 收口门。任何实现都不得为了满足样例而把国家、
仓库 ID、调用顺序或场景步骤写进 Platform workflow、Tool 状态机或 Supervisor 固定流程。

1. child 发出的 requested schema 投影为统一业务卡片，答案直接回到原 child request。
2. 覆盖提交、拒绝、取消、超时、中断、校验错误和刷新恢复；UI 不暴露 child Thread ID、raw
   MCP request/response、app-server request ID 或未裁剪 schema。
3. Agent activity 以官方 child Thread/Turn/Item 为唯一事实源，展示 Agent 身份、任务、Item
   类型、MCP server/tool、bounded 参数与结果摘要、经安全裁剪的 command/action/output 摘要、
   started/completed/failed/waiting/approval 状态和时间顺序。不得继续把所有 commandExecution
   压成 `Using/Completed a workspace command`，也不得只保留“数据检查”“路线与成本计算”之类
   无法区分真实步骤的业务占位文案。Browser 可以按 Agent/Turn 展开官方 Item 明细，但不得显示
   server absolute path、Secret、未裁剪输出、raw JSON-RPC/request ID 或 MCP Resource 内容。
4. live WebSocket、刷新后的 durable projection 与 official paginated child history 必须收敛到同一
   Item 身份和终态；乱序 started/completed、child 先于 SubAgentActivity Started、审批等待、失败、
   取消、中断和 replay 均不得产生重复、丢失或伪造步骤。Platform 不复制 Thread history，只保存
   可重建的 bounded UI projection。
5. 已删除 Work State/coordination MCP 与 gates、`needs_input` 中继、event projection 的仓网
   continuation Prompt、主动发 Turn、Data gate global interrupt 和第二调度器路径。
6. 已删除 Supervisor policy snapshot/binding 与 continuation 表/调度；Runtime history 和 exact
   Item 仍是协作事实 owner。

退出：Root 在 child 运行时仍可响应用户；child success/error/cancel 均由原生终态和 mailbox
返回；Root final 后 mail 不自动启动新 Turn；Profile 重启时 pending form 明确 interrupted；
Web Agent activity 可实时展开并在刷新后恢复同一 child Item 序列，实际 MCP Tool、command/action、
progress、approval 与 terminal outcome 可区分，且没有第二 Thread/history/log owner。

### Slice 5：原生 Resource 与 Workspace 文件驱动的完整仓网能力

仓网规则全部进入 Skills 与确定性 Tools，不进入 Platform DTO、数据库或状态机。

#### Active surface 切换硬门

Data/Network 新 Resource 工具进入已授权 MCP server、真实调用或 E2E 之前，必须删除 generic
`ResourceLink→Artifact` 注册、任意 renderer 注册、跨 child Resource 搜索和 Artifact 输入回流，
否则每个中间 profile、mapping、matrix、scenario Resource 都会被 Platform 二次物化为 Artifact，
形成第二数据面。当前对话内额外展示只接受 active Copilot `[[deliveries]]` 声明的 exact producer
与固定 typed delivery kind；Platform 不按 Tool 名或业务字段猜测。仓网包当前用
`inline_geojson`/`map_card` 声明 `map.v3` renderer，Platform 只保存 renderer 与 producing Thread
的精确 Resource ref，并在浏览器授权读取时调用 official `mcpServer/resource/read`，不保存
Resource 字节、不登记 durable Artifact。文件交付同样由 exact producer、固定
`workspace_artifact` envelope 与 content verifier 声明；不得借展示卡片冒充文件交付。

Web 同时保留 Codex 原生 inline visualization 语义：仅从当前授权 Profile/Thread 读取原生 HTML
和签名验证后的 PNG/JPEG/GIF/WebP；Platform 可在完成的 Agent Message 对严格、Workspace-relative
`workspace_file` HTML 指令执行一次 no-follow 快照并改写为官方 `file` 引用，不引入 SVG/Markdown
renderer，也不把临时可视化登记为 Artifact。仓网 `map.v3` 是当前声明的额外 renderer 实例；旧
`report.v1` inline 卡片已删除。
仓网 Markdown 报告/地图文件和 meeting Markdown 报告是当前文件 delivery 实例。Artifact 保存
producer-time verifier snapshot，恢复不依赖届时 active package registry，切包不会使既有交付失效。

#### 数据准备

1. Network Agent 定义需求列表与已有仓列表的必要字段。
2. Data Agent 读取用户指定的 Workspace Excel/CSV/JSON；多个候选文件时用业务语言询问，
   不按时间或文件名猜“最新”。
3. 推断字段并映射数据要求；缺字段或映射歧义时用通俗业务语言解释并询问。
4. 获取国家行政区 city/province ID/name 与经纬度，为需求城市、已有仓和候选仓补坐标。
5. 有候选仓文件则使用；没有则询问省级、市级或用户指定范围。首次归一化保留所有已确认候选仓；
   baseline 通过计算范围排除，不删除已处理的候选事实。
6. Data Tool 以 create-new 语义把完整 `prepared_network_input.v2`（含选中 source units、候选仓和质量问题）
   写入用户可见 `outputs/warehouse-network/prepared/`；该路径、内容身份、roles/role_counts 和有界 warnings 是唯一 Data→Network 交接。路线/成本/方案等高成本
   typed intermediate 继续保存为 MCP Resource。
7. Data inspect 返回 `workspace_source_profile.v2` 的 exact units；角色评估不是全局缺口。fresh prepared 只有显式候选路径且 provenance fresh 才复用；`needs_input` 一次交回用户，`source_changed` 最多一次重检。
8. 候选仓、城市、现网仓、实际分配、需求或原始路线事实任一变化都产生新的完整准备输入；不使用
   candidate delta 或原地修改。校验结果不构成授权、可信等级或自动复用许可。

#### 距离、成本与分析

1. 构建已有仓/候选仓到需求城市的距离与时长；询问曲面距离×绕路系数或导航。缺绕路系数
   时询问；导航前展示路线数量、接口消耗和费用风险并取得许可。
   用户已经提供完整起终点距离、时长与来源方法时，由 Data Tool 写入准备输入，
   Network Tool 按当前分析范围直接物化并验证，不重复询问估算参数或让模型重读 raw 文件。导航则先
   生成精确缺失 lane request，在费用确认后由地理 Tool 自动执行并导入验证结果。
2. 优先使用用户路线报价；用户明确要求用现有报价均值外推时，由 Cost Tool 对 exact prepared input
   的完整报价按层计算 `mean(price_per_vehicle / vehicle_capacity)`，返回 `warehouse_quote_mean_calculation.v1`
   所需的 prepared identity、完整报价总数、分层报价数、币种、公式和均值 provenance，不让模型从 preview 推导。
   用户明确要求脚本证据时，单 Agent 可对同一 prepared input 执行受限脚本并把同一 typed evidence 写入
   calculations 目录；Planner 重新读取完整报价并逐字段校验后绑定证据。缺失路线且没有任何已确认计价口径时才询问，禁止以零成本填补。
3. 按成本优先或时效优先计算覆盖；计算一个或多个 SLA 目标的满足率，同时明确返回按城市
   数量与按需求量加权的两种口径及确定性的未覆盖城市，模型不得自行汇总。
4. 计算全网和分仓运输成本，完成增仓、减仓、搬迁三类模拟。
5. 执行 p-median：已有仓默认固定；用户给出 `p` 时传 `opening_policy.kind=exact`。用户只要求达到指定需求加权 SLA 的
   最优分布时，传一次 `opening_policy.kind=minimum_feasible`，Planner 在一个总时间预算内先最小化新增仓数、再固定仓数最小化成本，
   返回最多两个 `solver_stages`、selected number 和最终 coverage；只有用户明确许可时才允许指定已有仓关闭。
6. 执行给定 SLA 下的成本最优规划，输出覆盖、时效、距离、成本和仓库变动。
7. 当空间关系有助理解时生成对话内交互地图卡片；Network 从 exact 分配结果产生城市、设施、实际
   分配和城市→仓库 LineString 的通用覆盖 GeoJSON，Maps 只消费该精确几何 ref。结构化计算明细与
   Markdown 结果简报分开交付。
8. 路线按起终点坐标/身份、计算方法、provider 参数和 Tool 版本的 exact pair fact 复用；
   成本按路线 fact、报价/规则、币种和 Tool 版本的 exact lane fact 复用，只补算缺失项。

#### 文件与复用规则

- Tool 从 native `sandboxCwd` 解析 Workspace，相对路径是唯一文件定位合同。
- Task B 可读取 Task A 已写入同一 Workspace 的普通文件，或使用同一授权 provider 中
  由官方 Item/用户显式选择的精确 Resource ref；没有 Task A→Task B context/result API，Artifact 不作输入。
- Web Resource selector 从已授权 producer event 的 exact Item provenance 返回 bounded exact
  ResourceRef。Browser 选择时提交 producer event ID、ordinal 和 exact ref；Server 再次校验
  user/organization/Profile/Workspace、event/ordinal 与 `{server, uri, resource_schema}` 全字段相等。
  普通 RunEvent WS/HTTP 不广播 raw URI。唯一允许的 `resource_ref_projections` 只保存 official Item
  provenance、精确 ref 与安全展示摘要，不保存 Resource content，也不建立 opaque handle、latest/head
  alias、版本目录、dedupe registry、相似性匹配或 Platform 数据状态机。
- Tool 根据真实输入和用户要求最大化复用，可以复用部分 pair/行并只补算缺失部分；
  Platform 不以 SHA 或整个数据版本限制复用。
- 自动交接只发生在当前 Codex 原生 Root→child 协作链；后续复用必须由用户显式选择。同一
  Profile+Workspace 之外不得共享 provider Resource，跨 Workspace 只能导出普通文件后重新处理。
- Maps provider 保存 immutable、内容寻址的 `map_card_spec`（精确几何来源、图层、视角、标题与 parent spec ref）；
  Platform 只投影其 exact ref 供当前对话卡片修订鉴权。纯样式修订复用 GeoJSON；新增覆盖线先产生新的
  Network geometry Resource，再产生有 parent 的新 map spec。开发数据库按此 schema 重建，不从历史
  renderer payload 反推 spec。
- 仓网 Tool 与单 Agent 授权计算只允许在 `outputs/warehouse-network/{prepared,requests,calculations,deliverables}/` 下 create-new；
  输入源保持原位。文件名可由用户与 Skill 决定，同名时询问改名或取消，不覆盖旧结果。
- Mock 只在用户明确选择“使用印尼仓网完整示例”时把 fixture 作为普通可见文件放入干净
  Workspace，绝不能在真实输入失败后静默回退。

Data4 的 Stage C active surface 已不再依赖 `source_ref` 及 wire aliases、模型可见
revision/operation/CAS、ArtifactRef 双重 ref 或隐藏 Task 数据目录；inspect 使用 inline
`workspace_source_inspection.v2` identity，prepare 重新校验完整 Workspace 文件并写入 prepared
普通文件，不注册 Data Resource。Network decorated active surface 继续使用 strict `ResourceRef`；
未装饰的 `CaseRepository`、`NetworkSnapshot`、ArtifactRef compatibility tail 已由 Stage E 原子尾删。

退出：所有仓网 Tool 只使用 Workspace 相对路径、inline Data inspection identity、Network typed
MCP Resource ref 或普通业务参数；Data4 identity、完整仓网清单和 pair-level 部分复用都有确定性测试；两个 Task 可显式
复用文件或 Resource 但没有直连 context/result API；Platform 不出现仓网状态机。

### Slice 6：最小 Artifact 与安全投影

1. Artifact 仅承担完整分析的 Markdown 结果简报，以及用户明确要求导出、下载或保留的最终地图和其他交付件；结构化中间数据由
   Workspace 普通文件或 MCP Resource 拥有。只要求“展示/看看/可视化”时使用对话内地图卡片，
   不生成网页、PNG、Workspace JSON 或地图 Artifact。
2. 结构化计算结果保存完整覆盖关系及城市对应仓/距离/时长/成本，可由独立表格导出能力交付；
   正式结果简报只生成一份中文 Markdown 文件，在可下载 Artifact 中说明 SLA 覆盖率、总成本、
   模拟差异、p-median 结果与仓变动。Assistant 正文只概括关键结论，Browser 从授权 Artifact DTO
   将下载链接锚定到当前 Thread 的 exact producing Item 所在 Turn；跨 Thread 交付留在 Agent History，
   不追加到主 Thread 的最新 Turn，也不把整份简报插入正文。结构化结果与简报不得用同一 JSON 报告
   混充；地图在空间关系有助理解时使用对话卡片。
3. Artifact 状态只投影真实的生成中、已完成、部分完成或失败；模型文本不能冒充完成。
4. built-in final map/report Tool 使用 exact `(server, tool)` allowlist 和 typed output schema，原子
   create-new 写一个位于 `outputs/warehouse-network/deliverables/` 的自包含 map JSON 或 report Markdown。Platform 从 producing Run 解析授权
   Workspace，复用 `GitRuntime::download_file` 的 relative-path、regular-file、no-follow、physical
   containment 与 100 MiB 边界，即时复制到 Artifact；中间 MCP Resource 永不注册 Artifact。
5. Artifact delivery 直接附着在 producing official completed Tool Item，并以 Item provenance 做
   replay/idempotency；Browser 可在该 Tool 后或交付面板展示，不等待 Assistant 复述文本。
6. Runtime tool/child 事件转为安全业务 DTO；按 thread identity 幂等、容忍乱序、可重放，
   不暴露本地绝对路径、raw MCP payload 或 app-server request ID。

当前旧 Artifact rows 都由已否决的 generic 注册产生，不做兼容迁移。新增 current-schema migration
按外键顺序删除 `inline_visualization_artifacts` → `artifact_task_grants` →
`artifact_provenance` → `artifacts`，再只重建后三张表：Artifact source 改为产生时解析出的
Workspace ID 与相对路径，ready 后的内容不依赖 Run/Workspace 存活；provenance 对 exact
producer Item 唯一，Task grant 仍只负责授权。删除 `retention_state`、`source_server/source_uri`
及其 URI dedupe；没有用户 delete/expiry owner 前 Artifact 默认持久。物化 worker 接收现有
`GitRuntime`，不再通过 Codex `read_mcp_resource` 或 producer Run→Thread 恢复内容。

退出：刷新后对话地图卡片可由 Runtime history、bounded renderer/ref 投影和 provider Resource
恢复；明确导出的地图和报告仍可查看下载；中间矩阵不进入 Artifact 交换链；durable 交付可从
Runtime history 和 Artifact 权威记录重建浏览器视图。

### Slice 7：E2E、恢复和全量清理（完成）

#### 已通过的阶段一 normal-path 门

- 样例 1 只完成 Workspace 数据理解与准备，明确断言没有调用 Network Tool。
- 样例 2 由用户明确选择 Indonesia fixture，完成 baseline、两个候选仓、分配、6/12/18 小时
  覆盖、成本、地图和报告；fixture 作为普通 Workspace 文件使用，Tool 顺序和业务数值不被
  写成产品合同。
- 样例 3 复用样例 2 的已准备数据，只关闭 `WH-CROSS_DOCKING-BEKASI` 并比较 12 小时覆盖、
  成本与重分配；明确断言没有重新运行选址、地图或报告。
- 三个样例都不使用隐藏对象或特殊数据 API，不注入 Task/Run/WorkState/ref/wire ID；实际 Agent、
  Tool、Resource、Artifact 与业务结果均由 UI activity 和 owning store 交叉核对。
- clean DB/Profile/Workspace 从当前 schema/config 初始化；真实 Provider catalog、模型选择、Root/Data/
  Network 原生协作、Approval、reload restore、Workspace 文件和最终交付全部通过。
- 旧 Intake、Dataset、SourceAsset、DomainResource/Broker、Case、Work State、continuation、scan、Demo
  和长 Prompt E2E 已从 active 代码、测试和当前合同删除。

#### 后续 hardening（不再阻断阶段一）

- pending approval replay 仍从 sequence 0 扫描长 Run；应由 Platform approval owner 改为权威 pending
  projection/query，而不是在 Browser 或 Skill 增加缓存、重试或猜测。
- reasoning/agentMessage delta 对 `run_events` 有写放大；先测量用户可见延迟，再由 event projection
  owner 做 bounded persistence/retention，不为阶段一补极端竞态矩阵。
- 一次 E2E 中 Root 在 final Tool success 与 Artifact ready 后仍额外执行 `listFiles`；当前结果正确，
  记为 Skill 遵循噪声，后续用通用 Skill/Runtime 行为证据处理，不写死文件名或案例。
- 多用户、完整失败/取消/并发/跨 Workspace 矩阵、a11y、地图视觉细节和第二服务器 cold smoke
  随对应产品阶段验证，不反向扩张本阶段。

## 5. 删除与切换纪律

每个切片必须在同一 change 更新全部 caller、schema、fixture、测试、UI 和当前文档，然后删除
被替代路径；禁止双读、双写、字段猜测、deprecated DTO、alias、隐藏 fallback 或仅供旧 E2E
的成功分支。

阶段一最终不得保留；其中 4B.2-A 已完成的项标为“已删除”：

- 已删除：Work State service/routes/MCP/schema/migrations 与 Platform coordination MCP；
- Data Intake draft/session/mapping/gate/hold、SourceAsset 专用存储/API、DatasetRelease、
  DomainResource、Resource Broker 和 Task dataset/attachment/snapshot/projection；
- WorkspaceDataObject/revision/head/read binding、TaskSourceAsset、Task file store、semantic
  fingerprint/cache、typed publish/adopt；
- CaseRepository、NetworkSnapshot、DatasetRelease/profile SQLite 黑板、ArtifactRef、taskEvidence
  hashes、`source_ref` 与兼容 aliases；ResourceStore 仅保留 provider 内容存储必需部分；
- 已删除：event projection continuation、主动发 Turn、global interrupt；Slice 5 active surface
  切换前删除 generic ResourceLink→Artifact、inline/model-text 展示链、跨 child Resource lookup
  和 Artifact 输入回流；
- 已删除：Supervisor policy snapshot/binding/continuation、provider speculative hashes 和历史 SHA password 兼容；
- capability path scanning、假 installation/DB-only readiness，以及长 Prompt/手建 Work State E2E。

历史开发数据只通过 ADR-012 的显式重建流程清理，不能在 startup 猜测、迁移或兼容。

## 6. 验证与完成定义

阶段一完成以正常业务链、关键 typed failure、一次 pending approval 刷新恢复和 clean 初始化为门；
其余失败、并发和隔离矩阵随受影响 owner 进入后续 hardening。验证命令包括：

- `scripts/codex-upstream-status.sh`、`scripts/codex-customization-status.sh` 和
  `scripts/test-codex.sh`；
- `scripts/test-web-rust.sh`、Web lint/typecheck/Vitest/build；
- official Codex `schema_fixtures` drift gate 与真实
  `npm run smoke:codex-app-server` 的 `initialize`/`thread/list` 验证（协议变化时）；
- 仓网 Python pytest/Ruff、真实 stdio inventory/tool call、Mock E2E、真实上传 E2E 和配置真实
  导航时的 smoke gate。

阶段一完成证据如下：

1. Web 的真实 Workspace 文件 S1/S2/S3 正常链通过，同一 Task/Thread 完成 Data→Network→同 child
   follow-up；最终只有一个中文 Markdown Artifact 与一个 typed 地图卡片。
2. Task 之间不存在 Platform-owned message、context、result、data 或 adopt API；中间数据只走 exact
   MCP Resource，普通文件只走授权 Workspace。
3. Runtime 原生 discovery/reload、多 Agent、approval、history 与 Workspace metadata 被真实使用；
   pending approval 的整页刷新恢复通过。
4. Data4/Network12、双覆盖口径、完整规划与场景复用通过 typed 测试、stdio 和 Web 证据。
5. 旧重型路径已从 active 代码、数据库、测试和当前文档删除。
6. clean DB/Profile/Workspace 从当前 schema/config 初始化成功。

没有成功 evidence 时，只更新能力基线中的缺口；不得用模型最终文本、数据库 success 字段、
Mock fallback 或旧 7/7 脚本声称完成。
