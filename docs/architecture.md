# Architecture

| 字段 | 内容 |
| --- | --- |
| 文档性质 | 当前事实 |
| 快照日期 | 2026-08-21 |
| 代码快照 | 阶段二 Copilot 平台工作树；仓网 source-unit/prepared v2 合同以 ADR-025 为当前补充 |
| 当前阶段边界 | [ADR-018](adr/018-built-in-network-copilot-runtime-closure.md)、[ADR-019](adr/019-task-selected-copilot-packages-and-shared-tools.md)、[ADR-024](adr/024-warehouse-copilot-contract-simplification.md)、[ADR-025](adr/025-source-unit-prepared-v2-reuse.md) 与 [开发计划](development-plan.md) |
| 接受决策 | ADR-018；ADR-019 局部替代其单包/default 假设；ADR-024/025 约束当前仓网 Tool 合同 |

本文只描述当前代码怎样组成、事实由谁保存以及已经存在的边界偏离。目标方案不在这里
冒充现状；验证强度以 [能力基线](capability-baseline.md) 为准。

## 1. 当前运行拓扑

```text
Browser WebApp
  -> authenticated Platform API + WebSocket
  -> Platform Server + PostgreSQL
  -> Profile Host / Codex Adapter
  -> one Codex app-server process for the current Profile
  -> Codex Runtime
  -> authorized Workspace / managed Runner
  -> Task-selected Copilot Root config + Profile-native Skill/Role + declared MCP Tools
```

当前交付仍是单用户、单 Profile，但 User、Organization、Profile、Workspace、Task、Run
等身份已经存在于主要数据库记录和运行合同中。Browser 不直接访问 app-server；平台把
Runtime 事件转换成浏览器 DTO 和持久化投影。

## 2. 当前权威所有者

Turn 中断由 Adapter 校验 official `thread/read` 的 Thread ID 与实际 cwd 是否位于已授权
Workspace，再发送 official `turn/interrupt`。该控制路径不获取普通 Thread 准备互斥锁，
不 resume Thread、不应用计划重启；Profile Host 在生命周期读锁内校验两次请求的同一
Runtime instance，进程已替换时显式返回 `StaleRuntimeRequest`。中断结果仍由 Runtime
确认；没有新增浏览器协议、能力声明或持久化状态。

| 事实 | 当前 owner | 当前实现状态 |
| --- | --- | --- |
| Thread、Turn、Item、上下文、Agent 调度 | Codex Runtime | Runtime 是权威 owner；已物化 Thread 的实际 Provider/model 属于 official Thread settings，不由 Profile 默认值或 Task 字段覆盖。当前 Adapter/Server/Browser 仍叠加本地 history mode、approval overlay 和 live/history merge，其中 Browser 会按相同用户文本去重，尚未收敛为 official Item/client identity 的纯投影 |
| Skill、Plugin、MCP、Tool 执行 | Codex Runtime | Runtime 执行；每个 Copilot 包声明一个小型常驻 Root Skill、可选 Root Agent config、可发现任务 Skill、原生 child Role 与 MCP policy。Adapter 每个 Root Turn 只显式选择 Root Skill 的完整正文；`[root].task_skills` 显式决定该 Root Thread 是否在 Runtime Catalog 发现任务 Skill，child Role 的 scope 由其原生 Role config 决定。启用的任务 Skill 通过 Runtime Catalog 注入 name、description、可选 short-description 和 locator；Host 文件型 Skill 只有命中后才按精确 locator 打开正文及其 references。选中的 Capability Root 与 Role 启用的 MCP server 是不可扩大的权限上限；server 内所有已声明 Tool 都可用，但一律省略初始 `direct` surface，使 Runtime 原生 `tool_search` 只按已授权元数据检索。ToolSearch 仍由 Runtime 原生搜索、注册与分派；Chat bridge 编码当前 Runtime 生成的 canonical `Prompt.tools`，并在 wire_api=chat 下把 canonical history 中已完成的 client `ToolSearchOutput` 配对为 exact namespace/name reverse target 与 request-scoped function schema；该 projection 不修改 canonical `Prompt.tools`，相同 target/schema 稳定去重，失败、取消和孤立结果不产生目标，Core ToolRouter 仍是最终执行权限，Responses transport 完全不受影响。新 Turn 或 resume 只在所需 Tool 未出现在当前 request 时重新搜索；历史已加载且当前 request 可见的 Tool 直接复用。对 `wire_api = "chat"` Provider，Provider 配置/可信目录必须显式声明 `supports_function_tools`；缺失能力在 Core tool planning 以 typed ProviderFunctionToolsUnsupported 终止，ToolSearch 仍只以模型级 `ModelInfo.supports_search_tool` 为事实。Retained Chat bridge 把原生 client ToolSearch 映射为普通 function call，并保持 Runtime 搜索、审批、Tool identity、registry 分派与 namespace collision owner。根级 Tool 包用 `tool.toml`/`runtime.toml`、直接项目 manifest/hash lock 与领域代码声明运行需要；多个独立 Copilot 可引用同一个 Tool package，但彼此没有引用或通信。SDK generic provisioner 在 Runtime 启动前于 Profile/Tool/Workspace 外准备环境并产出内部 descriptor；Runtime 启动与用户对话期间不安装依赖。产品路径不扫描 cwd/Workspace 或按 Prompt 猜 Copilot |
| Profile 进程与 `CODEX_HOME` | Profile Host | 单 Profile 进程已存在；应用发现的 active Copilot 包在冷启动前共同收敛各自 managed Skill 与 child Role destination。Root Agent source 只生成 package-keyed Thread config，不安装成 child Role。其他 Skill、Role 和 `config.toml` 不覆盖；进程 `HOME`/`USERPROFILE` 与 neutral cwd 均按 Profile Host 隔离；Server 仍有显式宿主认证导入路径 |
| Provider 定义、模型目录与 Profile 的未来 Thread 默认选择 | Codex Profile config + Runtime；Platform 保存 Browser catalog 投影 | `config/batchWrite` 持久化 Runtime 配置；Runtime 的 `supports_function_tools` 与 `ModelProviderInfo.models` 都是显式 typed 配置事实，未声明时安全解析为 false。Platform 对可编辑 Provider 实施单一产品规则：`wireApi=chat` 时写入 `supportsFunctionTools=true`，Responses 写入 false；Web 不再提供第二个 Function tools 复选框，现有数据库 Chat Provider 由迁移自动改为 true 并受约束保护。模型级 `supportsSearchTool` 仍按 exact `model_id` 独立编辑和持久化；单模型修改和启动恢复都从数据库完整模型目录生成一次原子 `models` 写入，不能逐模型覆盖。启动恢复写回目录后必须安排一次安全边界 Runtime 重启，使 startup-scoped ModelsManager 使用新目录。Runtime Provider catalog 必填投影 function-tools bool；Platform/Web 收到缺字段或非 boolean 时拒绝整个当前协议响应。Runtime 的 `providerFunctionToolsUnsupported` 仍保护手工或外部 Runtime 配置，其他 bad request 继续使用通用脱敏文案。refresh 保留既有 capability，空/失败结果不覆盖 |
| Provider Secret | Platform encrypted Secret store 或显式环境凭据 | Direct credential 只进入 Profile/provider scoped Secret store；环境凭据只持久化变量名称并由 owned Profile process 注入。Codex config、Browser DTO、日志和文档不保存明文 |
| Workspace 授权、执行根与普通文件 | Platform + Runner | Workspace 独立于 Thread/Run；Task 已固定唯一授权 Workspace，Run/fork/recovery 受数据库和服务端不变量约束；真实 Runtime Probe 已证明 Root/child 使用同一 native `sandboxCwd`，同时观察到 macOS `/var` 与 `/private/var` 的同 inode 词法差异；physical-path join 的通用收敛进入后续 backlog，当前仓网链继续以既有 Workspace denial gate 为边界；通用文件 Web 产品流已统一到 `/workspaces/{id}/files` 与 `GitRuntime` |
| MCP Resource 内容与生命周期 | 各个 MCP provider | Codex 按 Thread/Turn 当前 server inventory 执行 list/read，official Tool Item 保存 ResourceLink/structuredContent；供应链 Data/Network 是同一 `supply_chain` provider Resource 域的两个 Tool，字节保存在 Profile 私有、按 canonical Workspace 隔离且内容寻址的 `ResourceStore`。Resource 只表示可复用的确切处理结果，不表示来源可信、业务准入或“最新”。Platform 不复制内容或提供通用 Resource API；只从完成 Tool Item 的 ResourceLink 投影 exact identity/provenance，供同 Profile+Workspace 的用户显式选择，并逐字段复验。地图卡片另保存 bounded renderer、exact `map_card_spec` ref 及父卡片 ref；浏览器按组织与 Run 授权通过 producing Thread 的 official `mcpServer/resource/read` 即时取得 GeoJSON |
| 通用 Copilot Resource/Workspace 基础合同 | Platform/Workspace authority 与 `packages/copilot-provider-sdk` 分工拥有 | Workspace 授权与 Artifact 物化属于 Platform/Runner；独立、领域无关的 provider SDK 提供 `ResourceRef` envelope、expected-schema 校验、canonical codec、payload bounds、typed errors、Workspace canonical/no-follow writer 与 provider load/publish primitives。Tool 用 `runtime.toml.platform_packages` 声明受 SDK registry 管理的平台包；generic provisioner 从已安装 SDK distribution 构建并注入 Tool 环境，不读取仓网路径或在 Runtime 启动时安装 |
| Task、Run、Approval、Artifact、Audit | Platform | Task 固定一个显式选择的 `copilot_package_id`；Run、Thread 创建与后续 Turn 沿用该 ID，Task 不保存路径或运行配置。持久 Artifact 只接受 active package registries 合并后的 exact producer、固定 typed kind/schema/MIME/verifier 和 Workspace-relative descriptor，并按 producing Item provenance 物化；中间 Resource 永不注册 Artifact。两个仓网包声明相同地图/报告交付，meeting 包声明 Markdown 报告；Platform 不理解业务字段。producer-time verifier snapshot 随 Artifact 持久化，恢复和下载不依赖届时 active registry |
| Codex Inline Visualization | Codex Runtime + Platform 授权快照投影 | Runtime 仍生成原生 `visualize`/`file` 引用和 Thread-scoped 文件；Platform 将执行器绝对路径投影为 basename，并且只在完成的 Agent Message 出现严格的 `workspace_file` 指令时，经权威 Run/Thread/Workspace 和 no-follow 文件读取，把受限 HTML 快照进该 Thread 的原生目录后改写为官方 `file` 引用。浏览器只允许当前授权 Profile/Thread 读取。Web 直接支持原生 HTML 与静态 PNG/JPEG/GIF/WebP；HTML 复用 Codex viewer assets 并运行在无 same-origin 权限的脚本沙箱/CSP 中，图片验证扩展名、大小与文件签名。SVG、Markdown 和任意 Artifact 脚本不进入该表面；额外 typed 卡片只来自 active Copilot 声明的固定 delivery kind，当前实例是仓网 `map.v3` |
| 用户输入 | Runtime 请求，Platform Approval 投影 | Root 官方输入路径已在真实 E2E 中通过 |
| Agent execution | Runtime 事件，Platform projection | 有等待、输入、完整终态和有界、裁剪后的 reasoning 文本投影；属于可重建视图，不暴露 encrypted reasoning |
| Capability Draft/Release/Installation | 无当前生产 owner | Catalog/Studio/Python publish crate、route、DTO、client、UI 与无 owner 数据表均已删除，不参与启动或 readiness |
| Work State 与 Platform coordination | 无当前 production owner | work-state-service、route、MCP、gate、schema 与第二控制面已由 Slice 4B.2-A 删除；不建设替代状态机 |
| Data Intake | 无当前 owner | 生产 route、validation crate、SourceAsset/Dataset/Task binding 表与产品入口已删除；阶段一不建设替代状态机 |
| 仓网规划能力 | 供应链 Python 包的 domain owners | 当前阶段统一拥有 demand/facility/candidate/location/lane/route/cost records，Data mapping/normalization/geography、route/cost fact 与 matrix build/validate/partial reuse，actual/optimized baseline、open/close/relocation scenario、assignment/service/cost evaluation、p-median、before/after comparison、分配覆盖 GeoJSON、地图卡片数据和确定性 Markdown 简报。来源预览显式区分样本数与完整总数；完整准备结果返回精确候选仓总数和有界目录。成本 Tool 可在 exact prepared input 内对完整报价按层派生 `warehouse_quote_mean_calculation.v1`，并逐字段校验用户脚本证据后绑定 bounded provenance；单 Agent 只有在任务 Skill 授权或用户明确要求时才可用脚本对同一 prepared input 做确定性聚合，脚本不拥有标准化或求解。需求、候选、现网仓、实际分配、路线或报价事实变动必须完整归一化；生成输入、导航请求、计算证据和最终文件分别只能进入 `outputs/warehouse-network/{prepared,requests,calculations,deliverables}/`。不完整路线返回 typed `needs_input` 且不发布可继续消费的 ready 引用；只有 Root 通过 Runtime 原生 `request_user_input` 收集业务选择。Network 覆盖线从精确分配和坐标派生，并按用户所选服务目标发布 `attained/missed/unassigned` 状态，不含城市/仓库名称分支；Maps 只消费该状态生成样式，不重算 SLA。该领域层不引入 Platform workflow、缓存或第二份业务状态。`solve_p_median` 的 `minimum_feasible` 在一个总时间预算内执行最多两个 CP-SAT stage：先最小化新增仓数，再在该仓数下最小化成本，返回 `selected_number_to_open`、最小仓数证明状态和最终 coverage，避免 Agent 循环调用多个求解 Tool。报告输入以 discriminated typed contract 区分单一 baseline 评估和 generic before/after comparison；comparison v2 接受同一 prepared identity 下任意 baseline、scenario 或 facility-location 结果，不为交付伪造方案结果。结构化计算结果与业务简报分离，不拥有通用 Resource/Workspace infrastructure。Stage E 已删除 Case/NetworkSnapshot/source_ref/ArtifactRef 与多层 hashes 旧偏离。未来若第二个供应链 Copilot 证明存在稳定公共领域合同，再评估内部提取 |

地图卡片读取 provider-owned GeoJSON 时，Platform 从授权 Task 取得固定
`copilot_package_id`，并在第一次 cold resume producing child 时携带同一
Root execution config；这样 `close_agent` 后仍能恢复 child Role 的 MCP inventory，
而浏览器不能选择或扩大该配置，Platform 也不复制 Resource 内容。

同一 Task、同一业务 Role 的 child 默认复用稳定 target。Root 对已关闭 child 先调用
`resume_agent`，再以 `send_input.items` 重新传入当前 Runtime Catalog 中的精确任务 Skill
和新任务 Text；Skill item 的重新注入与 Tool schema 历史复用是两件独立的事。Chat bridge
从 canonical history 配对所有有效 completed/client ToolSearch call-output，并把已加载 schema
投影到当前 Chat request；新 Turn 或 resume 只有在所需 Tool 未出现在当前 request 时才搜索。
失败、取消和孤立输出不产生目标；历史可见但已被当前 Runtime 移除的 Tool 仍由 Core ToolRouter
typed 拒绝。Root 不创建 Platform Tool 搜索、缓存或回放层。

Codex Runtime 仍是模型可见对话状态的唯一权威。Platform events、execution、activity
和协作状态都是投影，不应成为第二个 Thread、Memory 或 Supervisor。

## 3. Capability 创作、发布和安装的当前形态

阶段一已经删除旧 `capability-catalog`、`supervisor-catalog` crate，Agent/Supervisor/
Instruction Policy definition routes，Python capability validate/test/publish，Workspace
capability package publisher，以及对应 Rust/Browser DTO、client、Settings Studio 和 Sidebar
入口。普通 Workspace `tools/` 内容不再被平台识别、发布或迁移；它只是普通文件。

当前 schema 已删除 Capability Catalog Draft/Release/Installation、Agent definition/release/
run binding、Workspace package release 与 Supervisor instruction policy release 等无 owner
对象。产品启动只走 Standard Thread；Task 只读取当前单 Profile local/private package desired
installation 和自身固定的 package ID，不读取旧 Catalog Release、Agent/Supervisor binding 或
governed execution selection。Slice 4B.2-A 已由 migration 55 删除 Supervisor policy
snapshot/binding、definition/revision/release 和 continuation 六表；它们不再构成 Catalog、
发布或协作表面。Runtime agent projection 是现役 Platform 可重建投影。Slice 4B.2-B 已删除
本地 capability manifest、`profile_capabilities` 和 Web contract bundle；Profile Host 只
typed 校验官方 `initialize` 的四字段，并只对 `codexHome` 执行 Profile owner 安全校验。

当前开发者 Copilot 包不经过旧 Catalog/Draft/Release 路径。部署者显式配置可信
`--copilots-root`、`--copilot-prepared-root` 与 `--copilot-build-store-root`；Server 只枚举 Copilot 根的一级子目录和其中严格的
`copilot.toml`，package ID 来自 manifest，descriptor 只按该 ID 从 prepared root 解析，所有 command/dependency
binding 还必须来自同一 trusted shared build fingerprint。Browser
只接收/提交 package ID，不能提交服务器路径。存在可用包时，新建 Thread 必须显式选择一个包；
没有默认包、Prompt 分类或显示名推断。Task 持久该选择，Run 与 Turn 复用它。

当前本地迁移实测把两个仓网包和 meeting 包的 package-local Tool 环境从约 12.2 GiB 历史 build
收敛为 descriptor-only package 根与 405 MiB shared build store；未变化 Tool 的热 prepare 为 1 秒。
这些目录都是 SDK-owned disposable build/cache，不包含 Workspace、Profile、数据库或用户业务文件。

安装表以 `(profile_id, package_id)` 为主键，只持久 `desired_active`、source/configured revision、
精确 managed Skill/child Role ID 与安全 failure code，不持久或伪造 Runtime ready。所有 available
包在 clean Profile 建立 desired 安装，数据库 desired state 在冷启动时权威收敛。每个包只有一个
`[root]`；可选 Root Agent TOML 只投影成以 package ID 为键的官方 Thread config overrides，不
安装成 child Role。Server 不编译任何仓网源码。

`scripts/run-local.sh` 在 real Server 启动前遍历同一 Copilot 根，对每个包调用 generic SDK
`copilot prepare`。Copilot 的 `[[tools]]` 可用 `{id, package}` 引用根级 `tools/` 注册表中的严格
`tool.toml`；仓网共享实现当前位于 `tools/warehouse-network-planner` 和
`tools/warehouse-network-maps`。SDK 读取其 `runtime.toml`、直接项目 manifest 与 hash lock，
在平台 data root 下准备 Python/Node 环境并写入内部 `prepared-tools.v1.json`。Server 只消费该
typed descriptor，按当前 Profile 解析 typed binding，再把 transport 与 Root/child Role policy
合成官方 config。缺失或冲突明确 unavailable/failed；平台不扫描 Workspace、猜语言、server 或
Prompt。开发期 `dev`/`test` 默认复用 SDK 本机 Tool 环境缓存。
普通 `run-local` 启动与重启始终无损，不自动删除数据库或 prepared Tool 环境。开发 checkout
发生不兼容的数据库或 prepared composition 变化时，操作者可显式使用
`--rebuild-development-database`：该入口只接受仓库默认 data directory 与固定 loopback
`open_web_codex` 开发数据库，先停止该目录精确记录的 Server，重建数据库并恢复 Provider
定义、加密 Provider 凭据、默认模型选择和加密 Maps 凭据；Profile home、Workspace 文件、master key
与构建缓存保持不变。外部数据库 URL、URL file、data directory override 一律拒绝。`run-local`
始终使用当前 checkout 按精确构建指纹生成的 Codex；不接受
外部 `CODEX_BIN` 或 `--codex-bin`。这是因为当前平台依赖 Patch Map 中保留的 Runtime seam，
通用 Codex binary 即使能够启动，也不能被当作具备相同的 child Role/MCP 冷恢复能力。

Profile Host 只允许 `$CODEX_HOME/skills/<id>/SKILL.md` 和
`$CODEX_HOME/agents/<role>.toml` 两种
typed destination，不写 `config.toml`，不复制工具代码、venv、Node 依赖、Mock 或缓存。
普通 startup file 在 clean Profile 缺失时使用 create-new 原子落盘，已存在文件继续保留。
所有 active Copilot 包把声明的 Skill ID 与非 Root child Role name 作为 package-managed
destination；不同包目标冲突时整个 composition 明确失败。Profile Host
在 app-server 启动前先预检所有目标并 stage 全部写入，再批量 publish；失败时回滚
本批已发布变更。停用只删除持久安装事实记录的精确 managed destinations，用户使用其他 ID
创建的 Skill、Role 和 `config.toml` 不受影响。运行中 activate/deactivate 只保存 desired state
并返回 `restartRequired`，下一次冷启动完成文件与 Runtime 收敛。源码先移除时，持久 installation
record 仍允许停用；下一次 Profile Host 成功启动后才清除该包拥有的 Skill/Role，并保留安全的
`Unavailable` 记录供同 ID 恢复。Web 的最小 Copilot 管理面只展示状态并修改 desired state，
不接收源码路径、安装依赖或重启 Runtime。当前 Runtime instance 的 GET
状态只保留 persisted/source-revision 的 `Installed`/`Configured`/`Unavailable`/`Failed`；当前 Runtime
若存在 trusted Workspace，每次 status 最多做一次官方 `skills/list(forceReload)`，把该结果按 package
声明 Skill 交集投影。Role/MCP 执行能力不伪造为 Copilot readiness，由真实 Task/child MCP Item 证明；Task
selection 不依赖 status discovery。
运行中的 Skill watcher 与下一次 Role spawn 仍由 Codex 原生语义拥有。仓网验收 composition 在
`app-server` 启动前使用 Codex 官方、进程级 `--disable` feature override 关闭
`plugins`、`remote_plugin`、`apps` 与 `tool_suggest`；不改写持久 Profile 配置，也不在
Platform 侧过滤 Runtime discovery 或 Tool。

多 Agent 仓网包的 Root、Data、Network Role TOML 都显式关闭上游 multi_agent_v2，只使用当前已验证的原生协作合同。Data/Network Role TOML 只持有
`plugins.<tool>.mcp_servers.<server>` 下的精确 allowlist 与 Tool 级审批策略；其 Root 没有全局
仓网 MCP。独立的单 Agent 仓网包由一个 Root Agent 直接启用同一 Data/Network/Maps Tool policy，
并以 `features.multi_agent=false` 禁止 child Agent；它显式保留 Runtime 原生 shell，只允许任务 Skill
对 exact ready prepared input 做有界确定性聚合，脚本和结果 create-new 写入 calculations 目录，不能读取
任意 raw 文件、修改 prepared input 或替代 Planner 求解。两者只共享根级 Tool package，不互相引用或
通信。Data 的四个有界本地 Tool（发现、检查、完整准备、地理补全）统一预批准；Network 以 `prompt` 为默认，
预批准路线/成本/验证/分析/选址/比较、coverage GeoJSON 等本地 Tool 与最终 Markdown 报告 `publish_network_planning_report`；`map_utils` 预批准 `create_network_map_card`、`create_map_card` 与 `revise_map_card`。
外部导航、距离矩阵和 final Workspace 地图导出继续保留 official approval。Data→Network 输入是 Data Tool
create-new 的 `prepared_network_input.v2` Workspace 相对路径与 `input_identity`；Network Tool 在同一授权
Workspace 内自行验证和读取该文件，Role 不枚举、猜测、扫描 raw 数据或重解析其来源。fresh prepared candidate
只有在 inspect 输入中显式出现且 selected raw/admin provenance 仍 fresh 时复用；未选来源变化不阻塞当前目标。路线、成本和方案
继续是 provider-owned Resource，并逐一绑定同一输入身份。Platform 不为当前原生协作链建立消息或数据平面。
同时在 Tool annotations 中声明 read-only、destructive、idempotent 和 open-world 事实，Role
policy 只裁决该 child 的精确允许面，不修改全局 `approvalPolicy`。prepared transport 不固定 MCP
process cwd；Runtime 使用该 Thread 已授权的 Workspace 作为 stdio MCP cwd。依赖环境与
Tool state 均在源码树之外，Tool source 不复制进 Profile/Runtime projection且不被写入，SDK 仅在 owned build root
生成 staged copy/wheel。provider Resource scope 仍复用官方 Thread Workspace cwd；
maps 状态根由 `tool_state_root` 绑定解析到当前 Profile 的 owned MCP state root。旧专用 launcher
probe 的结果不作为新合同证据；通用 prepare、descriptor 消费与 real startup 只由当前工作树的
SDK/Server/run-local gate 证明。

3B.2 的真实 Runtime gate 已使用当前 checkout 构建的 Codex、生产 Profile seed composition
和本地 mock Responses provider 证明：clean Profile 的官方 `skills/list(forceReload=true)`
精确发现三项仓网 Skill；Standard Root 不发现任何仓网 MCP；原生 Data/Network spawn 只看到
各自 Role Skill 与精确 MCP/tool inventory。平台在每个 Standard Root Turn 通过官方 typed Skill
input 显式选择 Supervisor 正文，并关闭 Root 的通用 Skill 目录，因此 Root 不再通过 shell 读取
`SKILL.md`；child 仍由 Role 将目录收窄为唯一业务 Skill。native wait/mailbox、同 child follow-up 和终态正常。
这是多 Agent 包的既有真实证据；新单 Agent 包当前只有静态组合、Root config 和本地 Tool 合同
证据，不把它宣称为新的真实业务 E4。
修改 Profile Skill 会发出 `skills/changed`，force reload 读取新内容；修改 Profile Role 只影响
下一次 spawn，既有 child 不变。既有 child 对无配置 delta 的官方 MCP reload 是 safe-boundary
no-op，status 仍返回同一 inventory，不伪造 startup transition。单独的 malformed Role gate
收到 `configWarning`，spawn 明确返回 `unknown agent_type`、不创建 child，Root 仍正常终结。
3B.3-B2 又删除了 `RunStartPreflight`、Governed Agent/Supervisor mode、request-scoped Role
SHA/inventory 校验、Profile `platform-agents` writer/verifier 和相应 Server preflight。唯一启动链
现在是 authorized Task Workspace → Standard start/fork → Codex 原生 Profile Skill/Role/MCP。
Indonesia/Thailand 真实探针证明 Standard Root 与原生 child 都从各自 Turn `sandboxCwd` 读取
同一授权 Workspace，Workspace 中没有 capability 文件。prepared transport 不携带 cwd；
Runtime 使用该 Thread 已授权的 Workspace 作为 stdio MCP cwd。依赖环境、缓存和
Tool state 仍由 Profile/SDK 的 owned root 持有，不写入 Workspace。
3B.3-B4 已从当前 schema 删除 DB-only readiness 与 Catalog/Installation 等无 owner 对象；
Slice 4B.2-A 又由 fresh PostgreSQL 全迁移链断言 Work State 十表、Supervisor 六表和
provider speculative hash 四列不存在，同时保留 provider_call_metrics 与 Runtime agent
projection 表。2026-08-12 clean real Web 的 S1/S2/S3 已完成阶段一正常业务闭环。

## 4. 当前协同和 Run 生命周期

当前已存在以下可保留的基础：

- Runtime Root/child Agent 生命周期和精确 Runtime Role；
- Approval、Root 官方 `request_user_input`、回答持久化和输入队列；
- Agent execution 的稳定身份、等待状态和完整终态。

Slice 4B.2-A 已删除 Platform Work State/coordination 控制面、Supervisor
snapshot/binding/continuation 以及 Event Projection 的领域 continuation dispatcher 和主动
Root Turn。当前投影只保存 Runtime 事实：native Agent、activity、execution、Approval 和
Artifact；它不再负责协作调度。fresh PostgreSQL security gate 与原生 child projection
gate 均证明删除后 Runtime child lifecycle、Artifact 和资源投影继续可用。

Browser 的当前 Workspace 与每个 Workspace 的 Root Thread 选择只作为 session-scoped
presentation state 保存；reload 后必须先用授权 Workspace/Thread 列表校验，再通过既有
`selectThread` 恢复 official history、Agent activity、overview 和 durable pending approval。
2026-08-12 的真实 Web 门在 final report approval pending 时刷新，恢复了同一 Root Thread 和
唯一可操作 approval，接受后原 Network child 继续完成唯一 Markdown Artifact。

Supervisor 的延续策略仍由 Skill 拥有：当一次后续分析已经有完整 exact Resource refs、全部
普通业务参数、用户许可和交付要求时，使用 `fork_turns=none` 的新有界 child，避免把前一轮
完整模型历史重复送入 Provider；只有正确性确实依赖尚未结构化的原 child 判断时才使用同 child
follow-up。缺少当前 Tool 的 required input 时返回 typed `needs_context`，不由 Platform 搜索或
重建 Resource，也不增加持久 handoff/ledger。

## 5. 当前文件、Resource 与旧数据边界

通用 Workspace 文件 route、Runner 与 Web 文件面板现在统一提供列举/搜索、单文件上传、
文本查看、二进制下载和删除。每个上传请求只接受一个 multipart file；目标存在时返回 typed
conflict，由 Web 对该文件逐项选择覆盖、改名或取消，避免未报告的批量部分成功。GitRuntime
执行授权 Workspace、普通相对路径、大小、regular-file 与 symlink 边界检查，并用同目录
临时文件和原子落盘语义写入。

Task 已在创建时固定一个授权 Workspace，Browser 不能在 Run 启动时改选；真实 Runtime
Probe 已证明 Root 与 native spawned child 的 FastMCP 调用都读取 Turn 的
`sandboxCwd`，而不是 Plugin 进程 cwd。集成测试进一步证明同 Workspace 两个 Task 读取
同一普通文件、没有 Task files API，另一个租户不能列举、读取或删除该 Workspace 文件。

Platform 的 SourceAsset、Data Intake session/draft/mapping/gate、Dataset Release、Task
attachment/binding/snapshot/projection 生产路径和数据库对象已删除，没有引入 replacement
registry、revision、binding、fingerprint 或 cache。Platform Work State 与其
WorkResourceReference 已随 Slice 4B.2-A 完全删除。供应链旧 `CaseRepository`、Case
models/services 及其测试已在 Stage E 删除；当前 provider 不再维护第二套 source、mapping、
revision、operation、dependency、readiness 或 deliverable 状态。

当前 `ResourceStore` 以内容寻址 URI 为同一 logical `supply_chain` provider 的 Network
能力提供不可变 Resource 内容，并以 Profile 私有、canonical Workspace 隔离的物理目录保存
字节；Data inspection 不发布 Resource。Data4 与 Network 的 active surface 以 inline inspection
identity、Workspace 路径和 typed Network Resource 各司其职：Data Server 的 inspect 返回有界
`workspace_source_profile.v2`、精确 source units、预览样本数与完整总数、`workspace_source_inspection.v2` identity 及精确
inspected paths；角色评估不是全局业务缺口，只有选中 unit 的业务语义无法确定时才询问。prepare 使用
`source_selections` 和必要的显式 mapping，若来源在握手期间变化则返回 `source_changed` 且不写文件，否则以
`prepared_network_input.v2` 原子写入 `outputs/warehouse-network/prepared/`，保存 selected provenance、roles、role_counts 和有界 warnings；`prepare_network_geography` 再从该文件生成新的完整输入。Network
Tool 只接受这个确切路径，并为矩阵与方案 Resource 绑定输入内容身份；不存在 candidate delta 或
Data-to-Network ResourceRef 交接。
CaseRepository、NetworkSnapshot、
ArtifactRef 和其 wrappers 已在 Stage E 原子删除；不能通过新增第二套 resolver 或兼容包装
恢复该切换。通用的 scope/ref/store/codec/bounds/error/writer 已迁到
`packages/copilot-provider-sdk`；仓网包只保留数据准备、网络规划、地图与报告等领域模型和算法。
provider SDK 提供可复用实现，但 Resource 字节与生命周期仍由实际 MCP provider 拥有，Platform
不复制内容，也不因此成为 Resource Broker。

Data4 active surface 不再接受 `source_ref/sourceRef/source_refs/sourceRefs/source/source_file`、
`mappings/entities` 或 `fields/field_mappings/fieldMappings` aliases。discover 返回普通 source descriptors
和可验证的 prepared candidates；inspect 直接返回 inline source profile，`prepare_network_input` 使用 exact
`source_selections` 并可在缺少坐标时返回 `needs_geography`；`prepare_network_geography` 再接收已确认的行政区 catalog 与可选
overrides。Network active surface 读取经校验的准备输入路径，provider-owned intermediate 继续使用
typed MCP Resource；旧 Case/ArtifactRef compatibility tail 已删除。Platform 不建立数据
revision/binding/fingerprint/cache
或通用 Broker。`resource_ref_projections` 是当前已实现的有界投影：只保存 completed official Tool Item
ResourceLink 的 producer event/item、ordinal、`{server, uri, resource_schema}`、Tool 与显示名；不保存
Resource content、摘要语义、版本列表或“最佳/最新”判断。它只支持同 Profile+Workspace 的用户显式选择，
Server 在 Turn 前重验所有字段。

用户输入中的完整路线距离、时长和来源方法由 Data Tool 保存在同一个
`prepared_network_input.v2` Workspace 文件中；Network provider 按明确仓库范围将其物化为
`route_matrix.v3`，矩阵与验证器共享其 discriminated `warehouse_scope` 与精确 `warehouse_ids`，并对该范围内缺失、重复和范围外
pair 给出显式验证结果。覆盖口径及未覆盖城市由
Network Tool 确定性计算，同时区分城市数量和需求量加权指标；这些领域事实不进入 Platform
DTO、数据库工作流或 Skill 中的案例规则。

当前 `artifacts`、`artifact_task_grants`、exact Item provenance 和物化字节是 Platform
最终交付 owner。generic ResourceLink 注册、跨 child Resource 搜索、Artifact 输入回流和旧
`report.v1` JSON inline renderer 已删除；正式 comparison 简报使用
`network_planning_report_markdown.v2`，单独 baseline 评估保留独立的 baseline report bundle 口径，最终均作为 built-in final Tool 产生的中文 Markdown 文件登记并下载。对话内 `map.v3` 只保存 bounded renderer 与 exact producing Resource
引用，原生 Codex inline visualization 则从授权 Profile/Thread 目录即时读取，二者都不会把
中间 MCP Resource 提升为 Artifact。`content_sha256` 只记录复制后字节完整性，不参与业务复用
或准入。`map.v3` 的 provider-owned、内容寻址 `map_card_spec.v1` 只保存精确 GeoJSON refs、图层、视角和可选父 spec ref；
GeoJSON ref 的紧凑 profile 还声明每个字段的观测值类型但不复制任何值，Maps 因此能拒绝全空字段、类型
不匹配的 source expression，以及当前 renderer 未声明 image asset 的 `icon-image`，而非发布假 Ready 卡片。纯样式
修订复用原 GeoJSON，新增覆盖线先由 Network 产生新的 GeoJSON 再创建 child spec。Platform 的
`inline_map_cards` 只投影该 exact spec ref 和父卡片关系，浏览器不接收 GeoJSON 内容。

## 6. Profile 与认证的当前边界

Profile Host 已为 Runtime 创建独立目录和进程。每个 Host 以系统临时根下独立的 0700
neutral directory 作为 app-server process cwd，初启与 restart 共用该目录；`HOME` 和
`USERPROFILE` 指向 Profile 私有 process home，因此不会通过宿主 HOME 或源码/Runner 祖先
发现用户 Skill、Plugin 或项目 `.codex`。业务 Workspace cwd 仍只由官方 Thread/Turn 参数
传入。Server 的单 Profile 初始化在没有显式来源时仍会默认读取服务器操作者的
`$HOME/.codex/auth.json`，并在目标 Profile 缺少认证文件时复制；这条显式认证导入仍不是
未来多用户身份隔离合同。

## 7. 当前浏览器产品表面

生产构建只有一个浏览器入口：`index.html` 加载 `browser/browser-entry.ts`，入口完成平台会话
后直接渲染 `WebApp`。仓库没有 desktop/Tauri Runtime；源码中的 `@tauri-apps/*` specifier 由
Vite 映射到 browser shim。当前 production import graph 在展开该 alias 后只包含 75 个本地
non-test TS/TSX/CSS 文件，其中 Markdown 外链仍通过一个 opener shim；仓库另有 466 个 non-test
Browser 文件不在该图内，但 `tsconfig` 仍会把它们全部编译。`src/main.tsx`、`App`、`MainApp`
及其 hooks/facade/styles/tests 因而仍构成非生产维护面。`PlatformClient` 又以一个完整 class 进入
生产 bundle，未使用的 legacy methods 和 endpoint 字符串不会被 tree-shake，形成浏览器 API
第二表面；这些事实不能作为第二产品入口或兼容合同继续扩展。
历史 `check-main-ui-parity` 字节级第二 UI truth 及其手工 SHA/overlay CI gate 已删除；Web 交付
使用类型检查、组件/合同测试、生产构建与真实浏览器验收。`check:no-desktop` 继续作为
browser-only 边界门。

Settings 只保留 Codex 原生 Runtime Agents、Skills/MCP 等当前运行配置表面；旧 Agent
Catalog、Tool & Skill Studio、Supervisor Studio 与 Sidebar Agent Studio 已删除。当前阶段没有
公开 Capability authoring/publish/install 产品入口。

SDK 当前提供单/多 Agent `init`、共享 `tool init`、`validate/prepare/dev/test` 和聚合
`check`；仓库入口 `scripts/copilot.sh` 在隔离 SDK 环境中固定可信 Tool registry、当前 checkout
Runtime 与共享 build store，`sync` 通过完整 `check` 后把冷重启交回 `run-local.sh`。`test`
支持 Root 直调和 child Role、多用例隔离，并从 canonical Thread history 验证 Tool 与终态。
当前仍没有公开安装/发布、持久 Runtime readiness、Web 源码创作链路或任意模型质量矩阵。
公开 SDK、Studio 和 Copilot Builder 已后移到阶段二；它们不是阶段一缺口或退出门。

## 8. 当前最重要的边界偏离

1. Profile Runtime 的 HOME 与 process cwd 已隔离，但 Server 仍可默认导入宿主认证；未来
   多用户身份隔离尚未成立。
2. 当前 history overlay 按 Tool 名称或 approval message 推断插入位置，Browser 又以相同用户
   文本合并 optimistic/live/history 消息；这些启发式在 Runtime exact Item ID 之外形成第二历史
   关联规则，连续相同消息会被误合并。当前 Codex subtree 还保留旧 legacy response-tool/history
   materialization seam，尚未接受 latest official paginated history 全量实现。
3. `run_events` 的 sequence 和 run/thread/turn/item provenance 是合理的持久投影，但当前
   `project_item` 会把未统一限长的 agent text、reasoning、command output、diff 与 Tool result
   同时写入 PostgreSQL 和 WebSocket，且 unknown Runtime method 仍可能被 Browser JSON summary
   放入对话；当前没有 event retention/prune owner。一次真实长 Run 观察到事件 delta 写放大，
   pending approval replay 仍从 sequence 0 分页扫描；它们是性能 backlog，不是阶段一正常链门。
这些都是当前事实，不是应继续兼容的接口，也不是已经完成的阶段一正常链前置。后续按 owner
处理 Browser legacy、完整 Thread/history/lease、approval replay、event retention 和多用户隔离；
不再把它们插回已经通过的仓网正常路径。
阶段一边界由 [ADR-018](adr/018-built-in-network-copilot-runtime-closure.md)、
[ADR-019](adr/019-task-selected-copilot-packages-and-shared-tools.md) 与仓网当前
[ADR-025](adr/025-source-unit-prepared-v2-reuse.md) 约束；具体顺序只进入
[开发计划](development-plan.md)。

## 9. 证据入口

- 当前能力和真实 E2E 范围：[能力基线](capability-baseline.md)
- 本轮源码与失败链审计：[临时审计](temporary-copilot-platform-refactor-audit-2026-08-08.md)
- 当前阶段所有权与协同合同：[ADR-018](adr/018-built-in-network-copilot-runtime-closure.md)、
  [ADR-019](adr/019-task-selected-copilot-packages-and-shared-tools.md)、[ADR-025](adr/025-source-unit-prepared-v2-reuse.md)
- 当前下一步：[开发计划](development-plan.md)
