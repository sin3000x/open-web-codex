# open-web-codex 安全模型

| 字段 | 内容 |
| --- | --- |
| 文档性质 | 规范性安全边界与发布门禁 |
| 适用范围 | Browser、Platform、Profile Host、Codex Runtime、Workspace、Git、Artifact、MCP |
| 当前验证状态 | 以 [能力基线](capability-baseline.md) 为准 |
| 产品要求 | [产品设计](product-design.md) |
| 系统所有权 | [系统架构](architecture.md) |

本文定义任何产品阶段都不能削弱的安全属性。它不因为源码中存在检查就宣称安全能力
已经通过；真实实现、负向测试和发布证据仍由能力基线记录。

## 安全目标

平台必须保证：

1. 用户只能发现、读取和控制自己被授权的资源；
2. Profile、Thread、Workspace、Secret、Artifact 和事件不会跨用户或组织串流；
3. 浏览器输入不能成为可信服务器绝对路径、Runtime 身份或授权决定；经授权 Workspace
   解析和边界检查后的相对路径可以作为普通文件定位输入；
4. Codex 和子 Agent 不能通过 Prompt、自报身份或 Tool 参数扩大权限；
5. 凭据只在拥有其生命周期的服务端进程或企业 Tool 边界中出现；
6. 审批、取消、恢复和交付具有稳定身份、明确终态和不可抵赖审计；
7. 页面刷新、断线、重启、并发和乱序不会恢复出更高权限或陈旧决定；
8. Runtime 能力未知、未验证或不兼容时失败关闭，不由平台 fallback 模拟。

安全模型不承诺阻止所有模型错误。它要求模型错误不能绕过确定性的身份、权限、
资源和执行边界。

## 保护资产

| 资产 | 主要风险 |
| --- | --- |
| 用户 Session 与组织身份 | 冒用、固定会话、跨组织切换错误 |
| Profile Home / `CODEX_HOME` | Thread、Memory、Provider 和扩展状态泄漏 |
| Provider 与 Git Secret | 浏览器泄漏、日志泄漏、跨 Profile 注入 |
| Workspace 与仓库内容 | 路径穿越、越权读取、错误删除、分支破坏 |
| Thread/Turn 上下文 | 被平台复制、跨用户恢复、错误绑定 |
| Approval 与 Control Lease | 重放、并发双决策、旧进程请求复用 |
| Artifact | 猜测 ID、跨任务读取、内容类型混淆、过期引用 |
| Workspace 文件与 MCP Resource | 路径逃逸、未授权 Resource ref 发现/读取、Tool 越权、SSRF、权限放大 |
| 事件、审计和评价 | 丢失、乱序、伪造成功、无法追责 |
| Codex 构建与 Provider transport | 供应链污染、能力漂移、协议误判 |

## 信任边界

```text
Untrusted Browser
  -> authenticated typed Platform API
  -> authorization + durable platform state
  -> Profile Host / typed Codex adapter
  -> Codex app-server / Runtime
  -> sandboxed tools and approved MCP connections

Platform
  -> authorized Workspace / Git resources
  -> encrypted Secret store
  -> authorized Artifact store
  -> enterprise Tool/MCP gateway
```

### 浏览器

浏览器是不可信输入端。它可以提交稳定平台资源 ID、业务参数和 Workspace 相对路径，
但相对路径必须由服务端在当前授权 Workspace 下重新解析。浏览器不能提交可信的：

- 服务器本地路径或 `CODEX_HOME`；
- Copilot package root、prepared descriptor 路径或其他应用 source 注册参数；
- Copilot Root Skill/Agent、Tool package 路径或运行配置；Browser 只能提交列表中稳定的 package ID；
- `organization_id`、`profile_id`、AgentPath 或 Runtime Role 身份；
- app-server request ID 或原始 JSON-RPC；
- Secret 明文或配置文件路径；
- 任意 Artifact 存储 URI；
- 声称已经完成的审批、执行或能力状态。

浏览器只接收有界、类型化、脱敏的平台 DTO。

### Platform

Platform 是身份、授权、Task/Run、Workspace 路径边界、审批、Artifact、审计和浏览器
合同的信任边界。
它可以调用 Runtime，但不能伪造 Thread 历史、模型上下文、Agent 状态或 Tool
执行结果。

地图凭据配置只能通过已授权的 typed configuration API 提交到加密 Secret Store。
受管 Maps MCP 通过显式 platform credential source 与一次性 loopback URL 接收 Server 送达，
不得从本地记忆恢复另一份 Key，不得将 Key 放入模型可见 Tool 参数、事件或聊天记录。
独立 CLI 的 local source 与 Web 管理模式由 Tool manifest/CLI 显式选择，不能在投递失败时切换。

### Codex Runtime

Codex 拥有模型可见上下文和执行语义，但不是企业授权系统。Runtime 提出的 Tool
调用、Agent 委派和资源引用必须经过平台或企业 Tool 边界的确定性检查。

### Workspace、Git 与外部系统

Workspace、Git remote、MCP Server、企业 API 和模型 Provider 都位于独立外部
边界。连接成功不等于调用已经获得业务授权；每次资源访问仍需校验当前主体、范围和
动作。

## 身份与授权链

任何资源操作必须能够沿以下链路解析，不能只凭最终资源 ID 命中：

```text
session
  -> user
  -> organization membership
  -> project permission
  -> profile / workspace grant
  -> task / codex thread
  -> run / approval / event
  -> artifact grant + producer provenance
```

### 身份原则

- 身份来自认证 Session 和服务端记录，不来自模型或浏览器参数；
- 用户默认只绑定自己的持久 Profile；
- Platform Admin 的基础设施权限不自动获得业务 Prompt、Thread 或 Artifact 读取权；
- Control Lease 只决定谁能向 Task 发送控制操作，不替代 RBAC；
- Runtime Agent 的 AgentPath 和 Runtime Role 是执行事实，不自动成为企业数据身份；
- 执行授权上下文不能被模型自由填写或转发。

### 授权响应

为避免资源枚举，跨租户或无权访问的资源默认返回不可区分的拒绝结果。审计记录可以
保存内部原因，但浏览器错误不能暴露另一个组织、Profile、路径或对象是否存在。

## Profile 与 Runtime 隔离

- 每个用户的 Profile 拥有独立、持久 `CODEX_HOME`；
- 单 Profile Copilot status/activate/deactivate 与 Task create 只接受应用已发现的 package id，并先
  执行 Session → Organization → owner Profile 授权；可信 Copilot/prepared/Tool registry 根只来自
  Server 启动配置。Server 只枚举可信 Copilot 根的一级 manifest，不扫描 Workspace、cwd 或用户路径。
- 当部署存在 available Copilot 时，Task 必须显式固定一个 package ID；Server 不接受 Prompt、
  display name、目录名或缺省值作为选择，不把 Root Agent 配置暴露给 Browser。
- 安装表的 desired/configured/failure 是 Platform 持久事实；`ready` 只能来自当前 Runtime
  instance 的官方 discovery，不能由数据库行、文件存在或路径扫描推断。
- 根级共享 Tool 只由 SDK 通过显式 registry 和严格 `tool.toml` 解析；Copilot 之间不存在隐式
  路径引用或通信。Root Agent source 只生成该 package 的 Thread config，不安装成可被其他包 spawn
  的 child Role。
- 同一 Profile 同时最多有一个主 app-server 进程；
- Profile 只能从平台显式授权的 Provider/Secret 与能力配置初始化；不得默认复制、
  挂载或读取服务器操作者的 `$HOME/.codex/auth.json`、Skills、Plugins、MCP 或 Memory；
- Profile 进程、缓存、Provider 目录、Secret 注入和 Runtime request 映射必须以
  Profile 与进程实例为 key；
- Runtime 重启后，旧进程的 request ID、审批和响应能力立即失效；
- Thread 必须属于当前授权 Profile；平台不能通过复制数据库事件恢复模型上下文；
- Profile Runtime 配置只能经 official typed config write/CAS/reload 合同修改；Browser 不能提交
  whole-file `config.toml`，Platform 也不能绕过 Runtime 直接替换该文件。Profile instruction、
  Skill 和 Agent authoring 必须各自使用明确、受限的文件或 Runtime 合同，不能借通用 Profile writer；
- 多 Profile 路由完成前，单 Profile 模式只能作为受限部署形态，不能宣称多用户
  进程隔离已经成立。

## Workspace 与 Git 安全

Workspace 是独立授权执行根，不属于 Thread、Task 或 Run。

必须满足：

- 浏览器使用 Workspace ID，服务端解析规范化根路径；
- Task 创建后固定一个授权 Workspace；start、resume、follow-up 和 child Thread 都使用
  同一原生 Workspace `cwd`，Run 不得另选执行根；
- Thread 当前 `cwd` 必须包含在授权 Workspace 内；
- Root/child `cwd` 与授权 Workspace 的等值和 containment 必须基于服务端解析后的 canonical
  physical path，而不是原始字符串或词法前缀；macOS `/var` 与 `/private/var` 等同 inode alias
  必须视为同一位置，同时继续拒绝 symlink 和规范化后的逃逸；
- 用户、Skill、模型和 Tool 可以使用 Workspace 相对路径；绝对路径、`..`、symlink 逃逸、
  含义不明的 `source_ref` alias 和历史 asset/Dataset ID 必须被拒绝；
- 文件、Terminal、Git、GitHub 和 Runtime 操作分别重新检查 Workspace grant；
- symlink、`..`、嵌套 Git root 和路径规范化不能逃逸授权根；
- 单 Agent 的脚本执行只允许任务 Skill 授权的确定性计算，输入必须是 Data Tool 返回的 exact ready
  prepared input；脚本不得扫描 raw 数据、覆盖输入或替代领域求解，脚本与 bounded 结果只能 create-new
  写入 `outputs/warehouse-network/calculations/`。报价均值脚本必须输出
  `warehouse_quote_mean_calculation.v1`，由 Planner 重新读取完整报价、校验 input identity/总数/分层数/公式/均值后绑定到成本矩阵；
  当前 Codex shell 的文件创建/覆盖动作仍由 Runtime 原生 Workspace sandbox 和 Skill 共同约束，Planner
  能确定性拒绝目录逃逸、非 JSON 和未绑定证据，但不能回溯证明通用 shell 没有先覆盖同名文件；该残余风险不改变
  标准化输入和成本矩阵的单一 owner，后续应由专用计算执行器收紧。
- 托管 clone/worktree 只能通过显式 Workspace 生命周期创建和删除；
- Run 取消、失败、租约过期或恢复不能隐式删除 Workspace；
- 删除前检查活动 Run、Thread 使用、Terminal、子 worktree 和未交付变更；
- Push、保护分支和远端变更始终需要显式产品操作，禁止 Force Push。

同一 Workspace 的所有授权 Task 看到同一组普通文件。Platform 不建立 Task→文件授权、
Task 私有副本、COW/overlay/snapshot、数据 revision、registry、binding 或语义 cache；
Task 间也没有消息、上下文、结果或数据 API。跨 Task 复用由用户显式选择同 Workspace
普通文件，或同一授权 Profile/provider 内有精确 official Item provenance 的 MCP Resource ref。

MCP Resource 不是 Workspace 路径。内容由 provider 持有，Codex 按当前 Thread 已配置的同名
server 执行发现和读取。跨 Task 可发现由专用授权 selector 完成：`resource_ref_projections` 只从
已授权、完成的 official Tool Item `ResourceLink` 投影 producer event/item、ordinal、Tool、显示名和
exact `{server, uri, resource_schema}`，不复制 Resource content、ResourceLink description 或业务摘要。
Browser 选择时提交 producer event ID、ordinal 和 exact ref；Server 重新校验
user/organization/Profile/Workspace、producer provenance 以及 `{server, uri, resource_schema}` 逐字段相等。
普通 RunEvent WS/HTTP 不广播 raw URI，但 URI 本身不是 Secret 或宿主路径；不得另造 opaque handle、
latest alias、相似性匹配或通用 Resource registry，也不得从模型文本或标题猜测。不同
Workspace/Profile 的选择明确拒绝。

SHA 只用于字节完整性、ETag 或 provider 物理去重，不能替代授权，也不能用整数据集指纹
裁决业务复用。

共享 Workspace 会引入并发写入和状态竞争。平台必须通过锁、策略或明确冲突结果
处理，不能通过重新引入“每 Run 私有 Workspace”掩盖问题。

## Secret 与 Provider

- Secret 以平台身份和作用域加密，数据库只保存密文与安全元数据；
- 主密钥由外部 Secret Manager 提供，不能提交到源码、文档或日志；
- Profile Secret 只注入对应 Profile 进程的私有环境；
- 企业 Tool 凭据只注入拥有资源授权判断的服务端边界；
- 浏览器、Artifact、Agent Definition、Supervisor Policy、Prompt 和审计不保存
  Secret 明文；
- Provider 模型目录和选择按 Profile/Provider 隔离，切换时不能复用其他
  Provider 的模型或凭据缓存；
- Secret 删除、轮换和进程替换必须处理活动 Turn、未决 Server Request，以及尚未
  产生首个官方 rollout 的持久 Thread；不能为了立即生效破坏运行一致性或留下无法
  恢复的 Thread 身份。

## Approval、Lease 与异步操作

每个异步操作需要稳定身份和明确终态。

### Approval

- Runtime 请求先持久化和脱敏，再通知浏览器；
- 决策校验用户权限、资源归属、版本和 Runtime 进程实例；
- 根 Thread 与授权 Agent 树中的子 Thread 使用同一任务级审批投影；浏览器只接收平台
  审批 ID、安全动作摘要和 Agent 展示身份，不接收 Runtime request ID；
- 只有全部 Tool 都是只读、有界或确定性计算的已评审 MCP Server，才可在 Plugin
  合同中声明默认预批准，并仍受 Thread capability root、Role MCP server scope 与逐 Tool 审批策略限制；
  混合风险 Server、凭据、外部副作用、权限扩大和非幂等写入不得使用该默认值；
- 并发决策只有一个成功；
- 持久或 session 级“以后允许”只能响应同一未决 official approval 返回的
  `proposedExecpolicyAmendment`，并使用 Runtime 的 typed decision；Browser 不能自报命令前缀，
  Platform 不能另设 rules 写接口或直接修改 Profile rule 文件；
- approved、rejected、expired、cancelled 等终态不可逆；
- 旧进程 request ID 不能在重启后重新获得响应能力；
- `delivery_unknown` 只允许相同决定重试，不允许改变答案。

### Control Lease

- Lease 使用数据库时间、TTL 和版本；
- 同一 Task 同时最多一个有效控制者；
- 强制接管需要额外权限、原因和审计；
- Lease 失效后，浏览器草稿或缓存不能继续发送 Runtime 指令。

### Run

- Run 是调度和审计尝试，不是授权主体；
- lease、heartbeat、取消和恢复必须收敛到明确状态；
- 审批等待属于独立对象，不能由单个 Run 状态字符串替代；
- 恢复不能自动获得新的 Workspace、Profile 或 Artifact 权限。

## Artifact 安全

目标 Artifact 具有独立身份、授权和保留周期。

- Run、Thread、Turn、Item 只记录 provenance；
- Artifact ID 不能包含存储路径或可猜测权限信息；
- 注册时校验 Schema、大小、类型、来源和内容安全策略；
- 浏览器通过授权 DTO 或短时受控 URL 读取；
- 子 Thread、后继 Run 和历史恢复分别执行 Artifact 授权；
- renderer 只处理声明并允许的类型，不执行 Artifact 中的任意脚本；
- Artifact 只承载地图、报告和明确用户交付，不作为 Agent 输入、Task 间交换、Workspace
  文件替代品或工作流门禁；
- Artifact、含义不明的 `source_ref`、asset/Dataset ID 和内容 hash 都不能替代授权合同；
  Workspace 文件用相对路径，MCP Resource 用已授权 provider 与精确 URI，两者不互相伪装；
- 删除、保留、替代和依赖失效产生审计。

原生 Codex Inline Visualization 是 Thread-scoped 临时展示，不是 Artifact 或跨 Run 成果共享：
Browser 只提交 Thread ID 与安全 basename；Server 重新验证组织、用户、Profile、Workspace 和
Root/child Thread provenance，再从该 Profile 的精确 Thread visualization 目录读取。文件必须是
非符号链接普通文件并满足 containment 与大小限制。HTML 使用 Codex viewer 资源、CSP 和不含
`allow-same-origin` 的脚本沙箱；静态图片只允许 PNG/JPEG/GIF/WebP 并校验文件签名；SVG、Markdown、
任意路径、任意 Artifact HTML 和其它内容类型显式拒绝。

Platform 可以在完成的 Agent Message 中处理严格的 `workspace_file` HTML 指令，但这不是浏览器
文件 API 或模型对 Profile 的写授权：Platform 从已解析的 Run/Thread/Workspace context 取得 scope，
通过 no-follow Workspace reader 读取安全相对 `.html` 文件并限制大小，再 create-new 快照到当前
Thread 的原生目录，将持久化浏览器投影改写成官方 `file` 指令。失败保留不可用的原指令并记录安全
诊断；不得改读绝对路径、生成截图、创建 Artifact 或把文件内容写入数据库。

`map.v3` 卡片的“基于此图修改”也不是 Resource 内容读取：Platform 仅从同一授权 Run 的
`inline_map_cards` 投影取得 exact `map_card_spec.v1` ref，并把该 ref 作为用户明确选择的 Turn 输入。
Maps provider 保存 immutable spec 和 parent ref；纯样式 revision 复用原 GeoJSON ref，涉及覆盖线的
revision 必须先由 Network provider 发布新几何。Browser payload 和 Platform database 都不得包含
GeoJSON 字节。

## 多 Agent 与企业 Tool

多 Agent 会放大现有权限，因此每次委派都要保持最小能力：

| 风险 | 必须的控制 |
| --- | --- |
| Supervisor 使用不存在或漂移的 Role | Role、Skill、MCP 和 Tool 必须来自当前 Profile 的 Runtime discovery；不能由数据库状态或路径扫描冒充 ready |
| 浏览器扩大 Agent 权限 | 浏览器不能提交 Runtime Role、MCP、Tool 或 capability root；Workspace 相对路径仍需服务端边界检查 |
| 子 Agent 继承全部权限 | Role 只启用精确 Capability Root/MCP server，与 Root 使用同一 Workspace 授权；server 内 Tool 由 Runtime deferred discovery 暴露，不增加 Platform 资源裁剪协议 |
| 模型伪造 Task 或角色 | 身份由系统绑定，不接受模型提交的组织/Profile/Runtime Role |
| Agent/Task 之间复制敏感数据 | child 协作使用 Runtime 消息；跨 Task 仅显式选择相同授权 Workspace 文件或带 exact Item provenance 的授权 MCP Resource ref；Artifact 不作输入 |
| 不同 Agent 结论冲突 | Supervisor 负责补充调查和最终综合，保留各自产物与依据 |
| Agent 网络无限扩张 | Runtime 并发、深度、预算、超时和停止规则 |
| Prompt 注入影响权限 | Prompt 只提供行为指导，服务端继续执行确定性授权 |

在 Runtime 到 MCP 边界能够携带或推导不可伪造 Agent 执行身份之前，平台只能承诺
已经验证的 Task/Profile 级受限资源范围，不能提前宣称 Runtime Role 级动态授权。

## 事件、审计与恢复

- 平台事件和审批先持久化，再向浏览器广播；
- 每个 Task 的 durable event 使用单调 sequence，重连只补缺口；
- 浏览器缓存和 WebSocket 是投影，不是权威历史；
- Thread/Turn 恢复以 Codex Profile 为准；
- 审计包含主体、动作、资源、安全结果、时间和必要版本，不保存 Prompt、代码、
  Secret 或无界 Runtime payload；
- 失败、拒绝、取消、超时、恢复和部分成功必须可区分；
- 备份恢复后重新执行授权检查，不能因为记录存在就恢复旧权限。

## 供应链与 Codex 定制

- Codex 构建固定到明确 revision；
- Codex 从 Rust 类型生成的 Schema、TypeScript 和 fixtures 是协议事实；运行时可用性
  只由官方 typed operation、显式 unavailable 结果和实际边界验证证明，不维护第二份
  capability 目录；
- 本地非生成差异必须进入 Patch Map，说明原因、测试和退出条件；
- 官方同步在专用分支完成，先接受上游结构，再重放保留 seam；
- 依赖、Plugin、Skill 和 MCP 包需要来源、完整性和权限策略；
- 未验证 Capability 默认关闭，不能由浏览器或 Server 静默 fallback。

## 发布门禁

涉及安全或授权的发布至少需要：

1. 正常路径与猜测 ID、跨项目、跨用户、跨组织拒绝；
2. 页面刷新、连接重建、Server/Profile Host/Runner 重启；
3. 并发请求、重复请求、乱序事件和过期 lease；
4. symlink、路径规范化、Workspace 删除和 Git 操作 containment；
5. Secret 不进入浏览器 DTO、日志、Artifact、配置响应和测试输出；
6. Approval 重放、并发决策、旧进程 request ID 和不确定投递；
7. Artifact 类型、Schema、授权、历史恢复和 renderer 安全；
8. MCP Resource ref 猜测、未授权 Item/provider、跨 Workspace/Profile 拒绝，以及 Artifact 不回流；
9. Capability 未声明、不兼容和 Runtime Role 不可发现的失败关闭；
10. 真实 PostgreSQL、真实 Codex app-server 和适用的真实浏览器纵向验证。

具体命令和当前通过情况不写入本安全模型；它们分别属于组件开发规则、开发计划和
能力基线。

## 当前发布边界

当前单用户入口、单配置 Profile Host、Bearer Session、未完成的 fresh-schema
PostgreSQL 复验、Run/Thread 作用域 Artifact，以及尚未验证的执行授权上下文都不是
生产安全完成态。最新边界以 [能力基线](capability-baseline.md) 为准，修复顺序以
[开发计划](development-plan.md) 和 [路线图](roadmap.md) 为准。
