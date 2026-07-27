# open-web-codex 开发计划

## 当前状态

| 字段 | 内容 |
| --- | --- |
| 更新日期 | 2026-07-25 |
| 当前分支 | `codex/agent-architecture-features` |
| Codex 基线 | `openai/codex` `6e5a2d6b8d148a5554fdceb6f399ca45bd1c78d9` |
| 上游待同步 | 126；观测到的 official main 为 `cba0e2701c9e3e67a877a16dbbd7a577d477a630` |
| 当前工作 | 以 1421 WebApp 为唯一前端，先收口单用户、单 Profile、单主 Profile Host 的真实 Runtime 闭环；多 Profile Router 暂缓到单 Profile smoke 稳定后 |

当前 Codex 基线上的定制仍按 patch map 分类；official main 已前进 126 个提交，
下一轮必须通过专用 `codex/sync-upstream-*` 分支同步。1421 WebApp 的 CSS、页面布局
和交互保持既有产品形态；当前单用户入口不显示登录或注册，浏览器自动取得本地
Session；差异集中在该入口、`src/services/webClient.ts` Server
适配层，以及三个由完整文件哈希锁定的非视觉 Thread 上下文接线文件。平台具备原生 Profile Host、Provider 服务、
加密 Secret、持久审批、Git workspace 与租约式 Run 编排。桌面运行时、sidecar、
4732/4733 daemon Gateway、原始浏览器 RPC/SSE 和桌面发布链已经移除。认证后的
根入口与 `/web` 都只加载同一个 WebApp；旧根 App/Bridge 源码不进入生产构建，
清理工作按当前范围暂缓。

本文只记录当前有效状态和下一步。完成项必须有代码、测试或可重现运行证据。

## 执行规则

- `[x]` 已完成并有证据；`[-]` 正在执行；`[ ]` 尚未完成；`[!]` 外部阻塞。
- Codex 源码变化前运行 `scripts/codex-upstream-status.sh` 和
  `scripts/codex-customization-status.sh`，新增差异必须先进入 patch map。
- 接受上游结构后，固定按 Chat transport、Provider metadata/cache、app-server
  Provider API、TUI Provider、legacy history、Capability Manifest、生成物顺序重放。
- 平台不得复制 Thread/Turn、Memory、multi-agent、Skills、Plugins 或 MCP Runtime。
- 浏览器不得接收 raw JSON-RPC、app-server request ID、凭据、Profile/Workspace
  路径或不受限 Runtime payload。
- 每个功能实现前必须写明 owning layer、输入输出、Capability gate 和验证方式；
  若实现需要跨 WebApp、Platform app-server、Profile Host 和 Codex Runtime，必须拆成
  独立 owner 的小变更，禁止用 WebApp 拦截或启动脚本写配置绕过 Runtime 发现链路。
- 数据库、授权、协议或恢复变化必须覆盖拒绝、重试、并发或重启路径。
- Canonical 文档只描述现态；历史决策只保留在 ADR/Git 历史中。

## 代码归属

| 层 | 目录 | 当前职责 |
| --- | --- | --- |
| Official Runtime | `codex/**` | Thread/Turn、工具、记忆、多 Agent、Skills、Plugins、MCP、Provider/TUI retained seams |
| Browser UI | `apps/web/src/WebApp.tsx`、相关组件/CSS | `main` 1421 WebApp 的既有页面、布局、交互和 UI 状态 |
| Browser transport | `apps/web/src/services/webClient.ts`、`apps/web/browser/client.ts` | WebApp 兼容方法到类型化 REST/WebSocket 的窄适配 |
| Platform server | `apps/web/server/**` | HTTP/WS、授权、DTO、服务组合、静态资源 |
| Profile | `apps/web/crates/profile-*` | 私有 `CODEX_HOME`、单主进程、app-server JSONL 生命周期 |
| Workflow | `apps/web/crates/run-orchestrator` | 幂等 Run、DB lease、heartbeat、恢复、取消 |
| Git | `apps/web/crates/git-runtime` | 私有 mirror、授权 workspace、status、选择性 Commit；当前 per-Run 绑定待迁移为独立 Workspace 资源和官方 Codex `cwd` 授权 |
| Security | `apps/web/crates/auth`、`approval-service`、`secret-store` | Session/RBAC、持久审批、加密凭据 |
| Contract | `apps/web/crates/*contracts`、`apps/web/contracts` | 浏览器 DTO、生成协议、Manifest、fixtures |
| Capability packages | `tools/**`、plugin/skill/MCP 包 | Runtime 可发现的工具、Skill、Plugin、MCP 声明；不得修改 Profile `config.toml` 或由 WebApp 伪造发现结果 |

## 当前边界债务 TODO

这些是当前实现中仍需按边界复审或迁移的项；在完成前不得把它们宣传为完整能力：

1. [x] map-card 已迁移为通用 Inline Visualization Artifact：Tool 只生成
   `open-web-artifact` envelope 与 `::codex-inline-vis{artifact="..."}` 短代码，
   Assistant Message 决定位置，Web 在同一消息内组合 Markdown/Artifact。旧
   Tool-attached `replyCard` 投影、双写、旧历史恢复和文本兜底已删除。实时与历史
   使用同一 renderer DTO；真实 DeepSeek/Mapbox 浏览器用例已验证
   “文字—地图—文字”、Thread 切换和刷新恢复。renderer capability negotiation、
   按用户配置隔离和 Responses Provider 真实浏览器 smoke 仍是后续独立门禁。
2. [ ] `apps/web/src/features/threads/hooks/useThreadMessaging.ts` 中 `/apps`、`/status`、
   `/fast` 等本地命令需要逐项边界复审：纯 UI 状态命令可保留；凡是查询 Runtime
   capability、工具、MCP、Skills、Plugins 或模型上下文的命令必须改为 Runtime/typed
   app-server 合同，不能由 WebApp 生成模型式回答。
3. [ ] `apps/web/src/services/tauri.ts` 仍是浏览器适配兼容层命名，需在不改变 1421 UI
   行为的前提下拆名或迁移，避免继续暗示桌面/Tauri 边界存在。
4. [ ] Capability Manifest 仍有手工 Alpha 子集；必须继续收敛到由 Codex 生成事实驱动，
   Web feature policy 只能消费这些事实，不能自行声明 Runtime 支持。
5. [ ] 旧根 App/Bridge 未引用源码和 browser shims 仍待裁剪，避免未来功能误接回旧桥。
6. [ ] 第三方 Chat transport 仍用“是否同时存在 Tool call”推断
   `AgentMessage.phase`，会把普通用户可见 preamble 误标为 `commentary`。实施时必须
   按 `docs/chat-responses-translation-spec.md` 移除该推断，保持标准 Chat
   `content` 的 phase 未指定、Reasoning 独立以及 Item 首次出现顺序；分阶段实现和
   验证矩阵见 `docs/chat-responses-translation-plan.md`。

## 单 Profile 收口目标

近期目标是先让一个真实用户使用一个持久 Profile 可靠跑通，再扩展多 Profile。
该目标是部署范围收窄，不改变所有权边界：WebApp 不发现、不启动、不模拟
MCP/Skills/Plugins；Server/Profile Host 只负责单 Profile 生命周期、Workspace
授权和安全诊断；Codex Runtime 继续拥有 Thread/Turn、当前 `cwd`、Provider、
Skills、Plugins 和 MCP。

单 Profile 运行合同：

1. [ ] 启动期必须显式确定唯一 `profile_id`、`CODEX_HOME`、允许使用的
   Workspace roots、Runner root 和 source root；Real mode 缺少 `CODEX_HOME`、
   Codex `cwd` 不在授权 root 内或 root 不一致时失败并给出可诊断错误。
2. [-] Server health/profile status 返回安全摘要，能确认当前 Profile Home
   identity、Profile Host state、Codex build/protocol/capability digest、Provider
   登录/模型目录状态和 MCP startup diagnostics；浏览器仍不得接收本地路径、凭据或
   raw JSON-RPC。当前已新增 Profile runtime status 安全摘要，包含 Profile Home
   fingerprint、Runtime health、capability 计数和 MCP server status 投影；Provider
   模型目录诊断已接入安全摘要，仍需用官方 OpenAI smoke 覆盖 file-backed auth 与远端刷新错误。
3. [x] `tools/maps-mcp` 只通过 plugin/MCP 声明进入 `selectedCapabilityRoots`；
   不写入 Profile `config.toml`，不由 `run-local.sh` 注入，不由 WebApp 读取或拦截。
   真实新 Thread rollout 已记录 `local-maps-mcp` environment root；plugin 对整个
   `map_utils` server 设置 `default_tools_approval_mode = approve`，普通地图 Tool
   调用不逐次审批，缺少 Provider 凭据时仍走独立的类型化 elicitation。
4. [-] MCP startup failure 归类并投影为安全诊断：capability root 未选择、`.mcp.json`
   缺失、`cwd` 解析错误、command 不存在、权限不足、Python/venv/pip 失败、
   package import 失败、MCP initialize 失败或 timeout。当前 runtime status 已投影
   Runtime 的 MCP server status；`map_utils` 依赖准备已移到平台启动期的共享
   maps MCP venv，launcher 在对话期只做快速 import 校验并失败快返；即使 MCP 子进程未继承平台环境变量，
   也会回退到仓库级共享 venv，并将 repo root、cwd、args、venv、Python 版本、import check 失败摘要
   和 server stderr 写入 launcher log；launcher smoke
   覆盖 initialize、tools/list、`create_map_card` 的 `map.v3` 输入 schema
   （平台管理的 GeoJSON sources、官方 Mapbox layer JSON、标准 camera 字段和
   可选 hover/legend extensions）以及官方 Style Spec warning/error 行为；
   下一步要把 Runtime
   failureReason 归一到上述分类。
5. [x] 新建 Thread 的单 Profile 真实链路已验证 `selectedCapabilityRoots` 包含
   `local-maps-mcp`，Runtime 能发现 `map_utils` 的五个工具，启动
   `./bin/maps-mcp-launcher`，并调用 `create_map_card` 返回
   通过 `outputSchema` 验证的 `open-web-artifact` /
   `inline-visualization.v1` `structuredContent` 和 Tool 生成的 embed code。
6. [-] 第三方 Provider smoke 使用真实 Codex Runtime 工具调用链验证：模型可见
   `map_utils` tool schema，Provider 返回标准 tool call，Runtime 执行 MCP tool，
   Server 从 Tool `structuredContent` 注册类型化 Artifact，Assistant 只复制
   Tool 生成的短代码决定展示位置。真实 Web/Profile Host 的 DeepSeek 新 Thread
   与独立 CLI smoke 已通过迁移前合同；严格类 Mapbox authoring schema 已通过
   单元测试和真实 launcher MCP smoke，CLI prompt 已更新，仍需在可用第三方
   Provider 凭据下重跑模型调用；
   `scripts/smoke-third-party-map-card-mcp.sh` 覆盖 Codex Runtime + Chat provider +
   `map_utils.create_map_card`，浏览器渲染由 `scripts/smoke-map-card-rendering.sh`
   覆盖。
7. [-] Inline Visualization 纵向链路已完成：`create_map_card` 返回通用 Artifact
   envelope 和 Tool 生成的 embed code，Platform 只登记 Artifact，Assistant Message
   在目标位置引用，Web 使用与上游 TUI 对齐的独立行/代码块/流式解析规则渲染。旧
   Tool-attached `replyCard` DTO、投影、历史恢复和前端分支已删除；第三方 Chat
   Provider 的“不引用不显示”自动化断言和“文字—地图—文字”真实浏览器用例已通过。
   Responses Provider 的真实浏览器矩阵仍待执行。
8. [-] 官方 OpenAI Provider smoke 验证 `codex login` 与 Web 使用同一个
   `CODEX_HOME`，模型列表按当前 Profile/Provider 刷新且错误状态可诊断。当前新增
   `scripts/smoke-openai-provider-models.sh` 验证 file-backed auth 和 `model/list` 非空。单
   Profile 过渡期允许在 Profile 缺少 `auth.json` 时，从
   `OPEN_WEB_CODEX_IMPORT_CODEX_AUTH_FROM` 或默认 `~/.codex` 导入 file-backed
   登录态；多用户阶段必须替换为 Profile-scoped auth 设计。
9. [ ] 删除当前每 Run 创建 writable workspace 的所有权模型，改为独立的
   Workspace 资源：用户显式选择已有授权目录，或在 Thread 之外显式创建托管
   clone/worktree；Platform 通过官方 `thread/start`、`thread/resume`、
   `thread/settings/update`/Turn 合同传递并校验 `cwd`。多个 Thread 可共享同一
   Workspace，Thread 和 Run 都不拥有 checkout。

短期 smoke 命令：

- `scripts/smoke-maps-mcp-launcher.sh`：验证 maps MCP launcher 可启动、声明
  `outputSchema`、声明 GeoJSON Resource template，并生成带 `map.v3` renderer 和
  embed code 的 Inline Visualization Artifact。
- `scripts/smoke-third-party-map-card-mcp.sh`：使用 `THIRD_PARTY_PROVIDER_*`/`DEEPSEEK_API_KEY`
  等环境变量临时创建 `CODEX_HOME`，验证第三方 Chat provider 通过 Codex Runtime 调用
  `map_utils.create_map_card`，断言 Tool 返回 Artifact/envelope，后续 Assistant
  embed code 才决定地图位置。
- `scripts/smoke-openai-provider-models.sh`：导入 file-backed `auth.json` 到临时 Profile，
  通过 app-server `modelProvider/list` 和 `model/list` 验证官方 Provider 模型目录非空。
- `scripts/smoke-map-card-rendering.sh`：运行官方指令流式解析、消息内分段、
  未引用不显示、live/history 一致性、MapReplyCard 和授权 GeoJSON 相关前端测试，
  并验证 camera/fit viewport 与点线面样式。

完成以上 smoke 后，再进入 M2 的按授权用户动态路由持久 Profile 和跨用户隔离矩阵。

## A. Codex 上游同步与定制收敛

- [x] 当前分支集成官方 main 到 `6e5a2d6b8d14`。
- [ ] 通过新的 `codex/sync-upstream-*` 分支集成已观测到的后续 126 个官方提交。
- [x] 将全部非生成差异分类为 `retain-core`、`upstreamed`、`move-out` 或
  `drop`，机器清单与 patch map 一致。
- [x] Chat DTO、Responses-to-Chat 转换、工具名反向映射和 SSE 翻译集中到
  `codex-api`；`core` 仅保留 `WireApi` transport dispatch。
- [ ] 按 Chat/Responses 转译规范修正输出语义：标准 Chat 文本不再根据 Tool
  call 推断 phase，流累计器保持 Message/Tool 首次出现顺序，Provider-specific
  phase/reasoning 扩展默认关闭并按 Provider 隔离；真实第三方地图卡片 Turn 的
  live/history 对照必须通过。
- [x] Provider metadata、模型目录/缓存、app-server Provider API 与 TUI Provider
  workflow 按 owning layer 集中并有 scoped tests/snapshots。
- [x] 将 TUI Provider 配置与 onboarding 实现从高冲突 dispatcher/auth 文件拆入
  Provider 专用模块；上游父模块只保留窄挂接点，完整 TUI 3,233 用例通过。
- [x] 恢复官方 `ToolName` 实现，将 Chat namespace flattening 仅保留在
  `codex-api` transport 边界，避免污染官方协议语义。
- [x] Profile Home 创建、授权、Secret、Provider CRUD 和浏览器 DTO 移出
  `codex/`，由 Web 平台承担。
- [x] Schema、TypeScript、Manifest、fixtures 与真实 app-server smoke 对齐。
- [x] 当前 Runtime 验证矩阵通过：format、Provider、config、MCP、protocol、
  app-server、TUI focused tests 和真实 initialize smoke；最新 auth routing、
  thread fork、Turn diff、apply-patch 与 `PathUri` 回归均通过。

同步门禁：`scripts/codex-upstream-status.sh`、
`scripts/codex-customization-status.sh`、patch map、生成物 drift、Web contract、
真实 app-server smoke 必须同时通过。

## B. 平台纵向闭环

- [x] PostgreSQL schema 覆盖 User/Session、Organization/Membership、Profile、
  Secret、Project/Task/Run、Lease、Workspace、Approval/Audit 和 RunEvent。
- [x] 当前单用户 Server 自动确保本地 Owner，浏览器通过免密本地 Session 直接进入
  WebApp；登录和注册界面不进入当前产品流程。
- [x] Session、Organization、Profile 归属和资源授权仍在服务端执行。保留的用户名/
  密码登录使用 Argon2id，旧 SHA-256 仅在成功登录后升级，供后续多用户入口恢复。
- [x] 资源查询带 Organization/User/Profile 归属；双组织越权负向测试通过。
- [x] 原生 Profile Host 直接管理 `codex app-server`，覆盖私有 Home、单主锁、
  有界事件、原位重启、Thread resume/read 和 Capability Manifest。
- [x] Provider 服务执行受控配置写入、Provider-scoped refresh/cache、选择与模型
  更新；凭据 AES-256-GCM 加密，只注入 Profile 子进程环境。
- [x] Provider/Model 选择器按 built-in、local 与 custom 分组，built-in/local
  默认折叠并在分组标题显示当前选择；LM Studio 与 Ollama 的 `gpt-oss` 不再混入
  custom。点击 Provider 行立即切换，当前 Provider/Model 在明暗主题中均使用明确
  的选中边框、色条和徽标。模型上下文通过一个保存操作串行持久化全部改动，避免
  Provider 模型目录替换写入竞态。平台全局保存最后一次 Provider/Model 选择，新
  Workspace/Thread 自动继承，Task 表保存每个 Thread 自己的选择；模型上下文和
  自定义 Provider 目录仍由服务端 Profile 持久化。模型目录刷新或上下文窗口更新
  后，Server 在下一个安全 Turn 边界替换 app-server：不中断当前 Turn，清除旧进程
  的 Thread 绑定并恢复同一持久化 Thread，使其下一个 Turn 使用重建后的模型目录。
  新 Thread 使用官方 paginated history；未绑定 Thread 以 `excludeTurns` 恢复，
  Server 并行读取 Turn shell 与 Item 索引并按 Turn id 合并，不再走 app-server
  串行 full-item 兼容 hydrator。已有 legacy rollout 只留在隔离兼容分支；当前会话
  已加载且未变化的完成态 Thread 直接复用浏览器投影，后台 Runtime 事件会使其失效。
  Task 级 Provider/Model 从已加载目录直接投影，目录缺失时只做不阻塞历史渲染的
  后台只读补充，不改写 Profile 默认选择，也不触发 Runtime 模型目录刷新。
- [x] 已有 Thread 在 Turn 级切换 Provider 时会重建对应模型客户端；真实旧
  OpenAI Thread 切换 DeepSeek 后不再沿用 OpenAI transport。
- [x] app-server 审批先持久化并脱敏投影，再由版本 CAS 决策和审计。
- [x] 每个 app-server 进程实例拥有独立 UUID；审批响应同时校验实例和 request
  id，重启会取消旧实例请求，`delivery_unknown` 只允许相同决策重试。
- [x] Profile Host 在活跃 Turn 或未解决 Server Request 存在时拒绝凭据触发的
  重启，并覆盖 Turn 启动响应与 started 事件之间的竞态窗口。
- [x] Git Runtime 创建私有 mirror 和每 Run 独立 workspace，拒绝危险 source/ref，
  支持 lock、status、选择性 Commit 和 cleanup。
- [ ] 按官方 Codex `cwd` 路线删除上述 per-Run checkout 所有权，建立独立
  Workspace 资源及授权关系；迁移数据库、Runner、浏览器上下文、恢复和显式清理
  策略，并覆盖多个 Thread 共享 Workspace 的并发与越权测试。不得改成 Thread 独占
  checkout，也不得把当前 per-Run 实现描述为目标架构。
- [ ] 将当前 Run/Thread 作用域的 Inline Visualization Artifact 存储迁移为持久
  Artifact 身份与独立授权；后续 Run 恢复同一 Codex Thread 时仍可解析已嵌入内容，
  生产 Turn/Item 只作为来源证明，不决定 Artifact 生命周期。
- [x] Run Orchestrator 支持 idempotency、`SKIP LOCKED` lease、heartbeat、恢复、
  cancellation/interrupt 和明确终态。
- [x] Task event 先持久化，按单 Task 单调 sequence REST replay，再组织隔离地
  WebSocket fan-out；订阅先建立，首次加载以权威导航/Thread 历史作为快照并把每个
  Task 游标推进到最新持久序号，重连只补游标后的有序 durable gap，避免刷新页面时
  把全部历史 delta 再送入 React。未决审批独立恢复，不依赖完整事件重放。
- [x] Thread/Turn 历史直接读取 Codex `thread/read` 与分页
  `thread/turns/list(itemsView=full)`；Server 保持 Runtime item 为事实来源，只按
  持久化 sequence 将平台拥有的审批投影插回对应 Turn，并按稳定 Turn 身份解析已授权
  Inline Visualization Artifact 引用。
  浏览器不再通过 localStorage 或旧实时事件自行拼装历史。

## C. 浏览器与传输收敛

- [x] 浏览器只使用 `/api` 类型化资源；原始 `/api/rpc` 不存在。
- [x] 实时通道为 `/api/events/ws`，Token 在首帧认证而非 URL；跨租户事件
  过滤测试通过。
- [x] 1421 WebApp 保持既有页面布局、字体、交互和功能分支；Thread 上下文接线
  不引入视觉或产品行为重设计。
- [x] `npm run check:main-ui-parity` 对 `apps/web/src` 执行 `main` 逐字节等价
  门禁；除 `webClient.ts` 及其测试外，三个必要接线文件以完整 SHA-256 固定内容。
- [x] WebApp 的 workspace、Thread/Turn、消息、durable replay/live、
  approval/user input、Provider/model、MCP/rate limit、文件预览和 Git status
  已切到类型化 Server 资源；Project/Task/Run/Thread 使用单次 joined context
  查询，文件、Git 与 MCP 始终跟随当前 Thread。消息渲染已接入 Runtime/Skills/MCP
  输出的 `open-web-artifact` / `inline-visualization.v1` 合同：Tool completion
  只注册 Artifact，Assistant 的 `::codex-inline-vis{artifact="..."}` 决定同一条
  Message 内的位置。大型 GeoJSON 通过 MCP Resource、官方 Resource read 和授权
  Artifact 延迟加载；一条回复可按文字顺序放置多张卡片。真实
  Codex/DeepSeek/MCP 纵向用例与核心浏览器 Thread 切换、历史恢复、运行态和文件
  预览回归通过。旧 `replyCard` 投影、双读和旧历史兼容路径不存在。
  MCP 侧栏状态直接读取 Runtime 启动通知形成的持久化轻量投影，不在 Thread
  水合时调用会枚举全部 tools/resources 的 `mcpServerStatus/list`。
- [x] 创建 Thread 时浏览器先用独立临时 ID 打开名为 `Thread` 的窗口，再按对应请求
  绑定服务端 Thread；若创建响应包含正式名称，则侧边栏与对话区标题同步替换，
  且不被紧随其后的旧占位列表覆盖。并发乱序返回不会串绑，失败窗口禁用输入并提供
  重试。首条文本消息成功启动 Turn 后，Server 会为仍使用 `Thread`/`New Agent`
  占位符的 Task 生成并持久化有界标题，在同一响应中返回给浏览器，侧边栏与对话区
  标题随即同步更新；迁移会从最早的持久化用户消息修复已有占位标题。切换已有 Thread
  时先显示加载动画，历史水合并完成一次隐藏渲染后再整体展示，避免空白与闪烁。
- [x] WebApp 外壳铺满整个视口；侧栏和对话内容宽度在 2K/4K 等大屏上渐进扩展，
  小屏断点仍保持单栏与抽屉式侧栏，不再受 1840×1120 固定画布限制。
- [x] 根入口和 1421 `/web` 自动建立本地 Session 后只加载 WebApp；旧 App/Bridge 源码仍保留
  但不进入生产构建，生产包不包含 `/api/rpc` 或 EventSource Gateway 调用。
- [x] 平台服务同源提供生产 browser build；Vite 仅在开发时代理 HTTP/WS。
- [x] 前端类型检查、单测与生产构建通过。

## D. 桌面运行时淘汰

- [x] 删除桌面 Rust crate、独立 daemon/sidecar、4732/4733 Gateway、原始
  RPC/SSE 与桌面发布入口。
- [x] 保留既有 React 状态树、文件、终端、语音、Git 和设置 UI；删除其 Tauri
  运行时依赖并通过 `src/platform/browser` 提供 Web 语义。
- [x] 删除桌面/iOS/Windows/macOS release workflows、脚本、图标、截图、网站和
  失效的项目 Skill。
- [x] 根 Cargo/NPM/Nix 构建改为 browser + platform server。
- [x] `scripts/run-local.sh` 改为单平台进程并保留前台、后台、状态、停止、
  Fake/Real 和外部数据库配置。
- [x] `scripts/deploy.sh` 提供单机 Release 部署入口：锁定依赖、隐藏详细构建
  日志、阶段进度、健康检查、持久部署状态、服务信息框和 Cargo target
  高水位增量缓存控制；缺少数据库配置时安全引导使用或创建固定的
  `open_web_codex`，凭据不回显且不进入进程参数；1421 Vite 继续仅用于开发。
- [x] CI 增加禁止桌面代码回流的静态门禁，并构建浏览器、平台 Rust 与
  PostgreSQL 集成测试。
- [-] 旧根 App/Bridge 已退出运行与生产构建；仅为旧 App 保留的未引用源码、
  tests 和 browser shims 待 WebApp 等价回归完成后裁剪。当前不会改动 1421
  WebApp UI。

桌面删除完成标准：源码、依赖、构建产物、CI、运行手册和发布入口均不存在；
`npm run check:no-desktop` 与仓库级搜索同时通过。

## E. 本分支最终验证矩阵

- [x] `bash -n scripts/*.sh` 和本地启动脚本 help/status 路径。
- [-] 1,210 个浏览器测试、typecheck、build、no-desktop、Codex contracts，
  以及真实 Codex/DeepSeek Provider 的 10 项平台 E2E 通过；main-ui-parity
  仍会报告尚未并入参考基线的有意浏览器 UI 扩展。
- [x] `cargo fmt --all --check`、`cargo test --workspace --locked`。
- [x] PostgreSQL migration/restart、两组织安全、Git Runtime 与 Run Orchestrator
  ignored integration tests。
- [x] `npm run check:codex-generated`、`npm run check:codex-contracts`、fixtures、
  Feature Policy 和真实 `--require-manifest` smoke。
- [x] 状态脚本已复核；当前集成基线为 `6e5a2d6b8d14`，观测到的 official
  main 已前进到 `cba0e2701c9e`，126 个待同步提交留给下一专用同步分支处理。
- [x] Fake Server HTTP/static/WebSocket 端到端启动验证。
- [x] Git status/diff 审查，确认没有未分类 Codex 差异或意外用户文件。

## 当前发布边界与后续里程碑

本分支完成的是可持续同步的 Codex 定制、浏览器纵向平台边界和桌面运行时
淘汰，不等于 V1 GA。以下是当前仍真实存在的产品门禁：

### 浏览器等价语义复审清单

以下入口保留了原页面和调用行为，但受浏览器/服务端边界限制，适配完成后再决定
是否调整前端表达：

1. `Open in app` 对 HTTP/GitHub remote 可打开网页；服务器本地路径不能启动用户
   桌面应用，Reveal 当前复制服务器路径。
2. Codex 自更新与 Tailscale daemon 生命周期由部署管理，页面调用返回明确的
   deployment-managed 状态。
3. 任意 Workspace 的 Codex CLI args 可以持久化，但共享 Profile Host 不会按单
   Workspace respawn；需先定义 Profile/Workspace 级策略，并通过 Codex 官方
   Thread/Turn 合同应用 `cwd` 和环境设置，不建立 Thread 专属进程或 checkout。
4. local usage 可按 Run/Project 汇总 token 与 Turn 数；官方事件尚不提供可靠的
   model share 和 agent time，因此对应值不伪造。
5. 目录选择输入服务器路径；图片选择、拖放和导出使用浏览器 blob/download；
   浏览器不获得任意服务器文件系统访问权。

### M2 多用户 Beta

1. [ ] 将 Server 的单配置 Profile 组合改为按授权用户动态路由持久 Profile。
2. [ ] 完成 HttpOnly Cookie、CSRF、logout/revocation、登录限速和会话轮换。
3. [ ] 审批 expiry 和 operator repair workflow；进程实例隔离、重启取消和不确定
   投递重试已完成。
4. [ ] rootless Runner、出网策略、资源 quota、进程/文件系统强隔离。
5. [ ] Push 凭据、保护分支策略、显式 Push 和审计。
6. [ ] 两用户并发的 Profile/Thread/Workspace/Event/Approval/Secret 系统性隔离矩阵。

### M3 Capability-gated Studio

当前 map-card 已使用 Inline Visualization Artifact：MCP Tool 返回通用
envelope 与 embed code，Assistant Message 用
`::codex-inline-vis{artifact="..."}` 编排位置，Platform 使用通用 renderer registry，
Web 在同一消息中组合 Markdown 与 Mapbox GL renderer；旧 `replyCard` 路径已删除。
`map.v3` 使用固定 Mercator 基图。平台管理 GeoJSON source 授权，标准 Mapbox Layer
JSON 经官方 Style Spec 校验后除本地 layer/source ID 外原样交给 `map.addLayer`；
不再维护 layer type、paint/layout、filter 或 expression 白名单。文本 hover 与
circle/line/fill legend 都是 `extensions` 下的可选 Open Web 行为。卡片不展示
source/layer 数量或 camera 调试信息。
Mapbox Streets 依赖受限公开浏览器 Token；无 Token 时卡片继续显示。共享地图配置弹窗可选择
Mapbox 或 Google；平台只保存一个加密的活动 provider/key，后一次配置覆盖前一次。
浏览器只在 Mapbox 活动时读取受限公开 Token，Google Key 始终留在服务端。
`map_utils` 的一次性配置请求由 Server 在站内复用活动配置，不再弹出独立网页。
当前配置存储为全局作用域，表结构已预留后续按用户隔离。第三方 Chat 真实 Web
smoke 已通过；Responses Provider 真实 Web smoke、renderer capability negotiation、
生成式 Platform Artifact schema 和权限下载仍属于后续门禁。

1. [-] MCP inventory/config/OAuth/full-form elicitation；工具审批使用的 confirmation-form
   elicitation 已接入持久化、无 Runtime request ID 的浏览器确认卡片和类型化响应，
   `map_utils` 的本机 loopback URL elicitation 已适配到共享 Mapbox/Google 应用内弹窗；
   provider/key 加密全局保存并由 Server 直接投递到带随机路径的
   `http://127.0.0.1:<port>` 请求，不打开独立页面。拒绝、Key/API 错误、超时和网络失败
   都进入明确终态或可重试错误。arbitrary form 输入与远程 URL mode 仍未开放。
2. [ ] Plugins install/update/disable/uninstall 与来源策略。
3. [ ] Memory health/export/reset。
4. [ ] Native Agents、Skills validate/test/publish/rollback。
5. [ ] 每个模块只在生成合同、fixtures 与真实 smoke 一致后开放。

### M4 生产 GA

1. [ ] Artifact/日志存储、retention、备份恢复和灾难演练。
2. [ ] 可观测性、容量、rolling upgrade、兼容 canary 与回滚。
3. [ ] 完整 Diff/File/Logs/审查体验、可访问性和目标视口 E2E。
4. [ ] 安全评审、依赖/镜像 provenance、发布物 SBOM 和运维手册。

优先顺序固定为 M2 隔离与安全，再做 Studio 和体验扩展；不得以新增 UI
绕过 Profile、Runner、Capability 或授权门禁。
