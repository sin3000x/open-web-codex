# open-web-codex 产品需求文档

## 0. 文档信息

| 字段 | 内容 |
| --- | --- |
| 文档状态 | V1 产品与研发评审基线 |
| 更新时间 | 2026-07-25 |
| 产品形态 | 单组织、多用户、自托管 Codex Web Harness |
| 用户客户端 | 标准浏览器 |
| Agent Runtime | 服务端定制 Codex `app-server` |
| 关联架构 | `docs/architecture.md` |
| 能力事实 | `docs/capability-baseline.md` |
| 研发计划 | `docs/development-plan.md` |

本文档定义产品目标、用户、业务对象、页面、流程、功能需求、权限、状态机、非功能指标和版本验收。运行时能力是否已经存在，以能力基线和实际构建生成的 Capability Manifest 为准；本文档描述产品需要什么，不宣称服务器已经支持什么。

## 1. 产品摘要

`open-web-codex` 是面向可信研发团队的自托管 Codex Web 工作台。用户在浏览器中导入 Git 项目、创建编码任务、观察 Codex 与子 Agent 的执行过程、处理审批、审查变更，并完成 Commit 或 Push。

产品由两个明确边界组成：

- Web 平台负责账号、项目、Task/Run、权限、Profile 生命周期、Workspace、审批、审计、Git 和浏览器体验。
- Codex Runtime 负责模型调用、Thread/Turn、上下文、多 Agent、记忆、Skills、Plugins、MCP 和工具协议。

平台不得创建第二套 Agent 调度器、Thread 历史或 Memory Engine。平台可以保存 Codex ID、事件投影和检索索引，但恢复模型可见上下文必须以 Codex Profile 为事实来源。

### 1.1 核心价值

1. 浏览器即可使用完整 Codex 工作流，不要求用户安装桌面客户端或本地 CLI。
2. 每个成员拥有隔离且持久的 Codex Profile，身份、Provider、Thread、记忆与集成可以跨 Task 延续。
3. Thread/Chat 使用 Codex 官方 `cwd` 语义在经授权的 Workspace 中工作；Workspace
   独立于 Thread/Run，可被多个 Thread 使用，平台不为每个 Thread 或 Run 隐式创建
   checkout；高风险行为经过可追踪审批。
4. 团队可以观察、接管、审查和恢复 Agent 工作，而不改变 Codex 原生执行语义。
5. Codex 构建通过版本化合同接入，官方上游升级可验证、可灰度、可回滚。

### 1.2 产品原则

- **Codex 原生优先：** Runtime 已提供的能力不在平台重复实现。
- **官方路线优先：** Workspace、Thread/Turn 与工具语义跟随官方 Codex；平台只增加多用户授权、持久工作流和浏览器安全边界。
- **事实来源唯一：** 平台、Codex Profile 与 Git 各自拥有明确的数据边界。
- **默认隔离：** 用户、Profile、Workspace、Secret 和事件流默认互相隔离。
- **人在回路：** 命令、文件变更、权限提升和结构化输入必须可审查。
- **可恢复：** 页面刷新、网络断开、进程退出和服务重启不能产生无明确终态的 Run。
- **能力协商：** UI 只启用当前构建明确声明且平台适配的能力。
- **显式交付：** 平台不自动 Commit、Force Push、Merge 或删除远端分支。
- **渐进交付：** 先完成浏览器纵向闭环，再增加多用户和 Studio；所有新能力沿平台边界演进。

### 1.3 最终目标的完成定义

最终产品不是把 Codex UI 简单搬到浏览器，也不是在 Web Server 中实现一个兼容 Codex 的新 Agent Runtime。完成状态必须同时满足：

1. 标准浏览器可以完成创建项目、长期对话、运行控制、审批、Diff、Commit/Push、恢复和 Codex Studio 管理，不依赖用户本机 CLI、桌面进程或浏览器扩展。
2. Codex 原生 Thread/Turn、上下文压缩、记忆、多 Agent、模型、Skills、Plugins、MCP 和工具语义通过 app-server 桥接复用；平台只提供授权、生命周期、持久工作流和安全投影。
3. 每个用户的身份、Profile Home、配置、Secret、Thread、记忆与扩展状态隔离；
   Workspace 作为独立授权的执行目录存在，Codex Thread 保存当前 `cwd`，平台验证其
   位于用户有权使用的 Workspace 内；Run 的事件、审批和交付权限独立审计，并有跨
   用户负向测试证明。
4. Runtime 与 Web 通过生成的版本化合同和 Capability Manifest 协商；不支持或未验证的能力在 UI 中明确禁用，不能由平台 fallback 实现。
5. `codex/` 能持续同步官方 `openai/codex/main`。产品定制集中在稳定桥接 seam，任何上游高频文件修改都有必要性、测试和 patch map 记录。

这五项是架构和里程碑取舍的最高优先级；局部功能如果破坏复用、隔离或可同步性，即使短期可用也不算目标实现。

## 2. 目标与非目标

### 2.1 V1 目标

- 邀请制成员登录、单组织成员与角色管理。
- 通过 Git URL 创建项目并验证仓库、凭据和默认分支。
- 创建、排队、运行、继续、取消、归档 Task 和 Run。
- 展示消息、计划、工具、命令输出、Diff、审批与多 Agent 轨迹。
- 每用户独立、持久的 Profile 与 `CODEX_HOME`。
- 支持 OpenAI 与第三方 Provider，模型列表按 Provider 隔离并可刷新。
- Thread/Chat 可以选择经授权的可写 Workspace，并通过 Codex 的 `cwd` 在其中完成
  Diff、Commit 和 Push；Workspace 不归 Thread 所有，同一 Workspace 可以承载多个
  经授权的 Thread。
- 支持持久化审批、Control Lease、事件补发和审计。
- 以能力门控逐步开放 Profiles、MCP、Plugins、Memory、Agents 和 Skills Studio。
- 固定并验证 Codex 构建，支持官方上游同步、灰度和回滚。
- GA 时用户核心流程只依赖浏览器。

### 2.2 V1 非目标

- 公共注册、匿名访问、计费、订阅和公共多租户 SaaS。
- 自建 Agent Planner、子 Agent Scheduler、Memory Engine、Skill Interpreter、Plugin Runtime 或 MCP Runtime。
- 浏览器操作用户个人电脑上的仓库、终端或文件系统。
- 完整在线 IDE、任意文件编辑器或无限制交互 Shell。
- 跨地域多活、零停机升级或 Kubernetes 强制依赖。
- 自动 Commit、Force Push、自动合并或自动删除远端分支。
- 兼容任意未固定版本的 Codex CLI。
- 在平台数据库中复制并替代 Codex Thread 或 Memory。

## 3. 用户与角色

| 角色 | 核心诉求 | 默认能力 |
| --- | --- | --- |
| Organization Owner | 管理团队、安全、平台策略 | 全部组织权限、Owner 管理 |
| Project Admin | 管理仓库、成员与项目策略 | 项目设置、成员、Task 管理 |
| Developer | 使用 Agent 完成开发任务 | 创建/控制 Task、审查、Commit/Push |
| Reviewer | 审查执行和代码变更 | 查看、评论、按策略审批 |
| Viewer | 了解进展与结果 | 只读项目、Task、Diff 和审计摘要 |
| Platform Admin | 维护运行基础设施 | Profile/Runner/容量/故障管理，不默认读取业务 Prompt |

### 3.1 角色约束

- Organization Owner 至少保留一名，最后一名 Owner 不可被降权或禁用。
- Platform Admin 的基础设施权限不自动赋予项目内容权限。
- Reviewer 处理审批不自动获得发送 Agent 指令、Commit 或 Push 权限。
- Control Lease 只控制 Agent 消息与 Run 操作，不替代 RBAC。
- 用户只能使用与自己绑定的个人 Profile；管理员执行恢复操作时不得读取 Profile Secret 明文。

## 4. 核心术语与事实来源

| 术语 | 定义 | 事实来源 |
| --- | --- | --- |
| Organization | 部署内用户与项目的权限边界 | PostgreSQL |
| Project | Git 仓库、成员和执行策略 | PostgreSQL + Git Remote |
| Task | 用户可见的长期目标，稳定映射一个 Codex Thread | PostgreSQL 映射 + Codex Thread |
| Run | Task 中一次可调度、可终止、可审计的执行尝试 | PostgreSQL |
| Profile | 用户级持久 Codex 身份与运行目录 | PostgreSQL 映射 + Profile Home |
| Thread | Codex 对话与模型可见上下文 | Codex Profile |
| Turn | Thread 中一次模型执行 | Codex Profile |
| Workspace | 独立于 Thread/Run 的经授权执行目录；可以是管理员登记的现有目录，也可以是用户显式创建的托管 clone/worktree | Platform authorization + filesystem/Git |
| Provider | 模型服务、Wire API、模型目录和上下文配置 | Codex Profile |
| Approval | Codex Server Request 的持久平台决策记录 | PostgreSQL |
| Control Lease | 控制 Task/Run 的短期租约 | PostgreSQL |
| Artifact | 具有独立身份、授权和保留周期的日志、测试报告、补丁、附件或可视化数据；生产 Run/Thread/Turn/Item 只记录来源 | Object Storage/本地受控存储 + PostgreSQL 授权 |
| Capability Manifest | 构建实际支持的方法、事件、版本和限制 | Codex 构建产物 |

### 4.1 Profile 与 Workspace 边界

- 一个成员默认绑定一个个人 Profile。
- 一个 Profile 拥有独立 `CODEX_HOME`、身份、Provider、配置、Threads、Memory、Skills、Plugins 和 MCP。
- 同一 Profile 同时最多运行一个主 app-server 进程。
- 一个 Profile 可以处理多个已授权 Workspace。Profile Host 必须验证传给 Codex 的
  `cwd` 位于当前用户和 Profile 有权使用的 Workspace 内。
- Workspace 拥有独立生命周期，不属于 Thread、Task 或 Run。多个 Thread 可以选择
  同一个 Workspace；Codex Runtime 负责保存和更新每个 Thread 的当前 `cwd`。
- 托管 clone/worktree 只能由用户或平台策略显式创建、保留和删除，不能在新建
  Thread、恢复 Thread 或创建后继 Run 时隐式生成。

### 4.2 多用户隔离键

所有可持久化或可恢复资源必须能沿以下链路完成归属校验：

```text
authenticated user
  -> organization membership
  -> project membership and action permission
  -> profile / workspace grant
  -> task / codex thread
  -> run / approval / event
  -> durable artifact grant + producer provenance
```

- 数据库查询不能只凭资源 ID 命中后返回，必须同时验证组织、成员关系、状态和动作权限。
- Profile Host 必须验证 `profile_id + user_id` 和 Thread 当前 `cwd` 的 Workspace
  授权；Runner 必须验证 Run、Task、Thread 与所用 Workspace 权限一致。浏览器不能
  提供可信本地路径。
- Codex Thread ID、app-server request ID 和 Profile 路径只能作为内部映射，不能成为绕过平台资源归属的公共 API 标识。
- 缓存、事件订阅、模型目录和 Secret 引用的 key 必须包含 Profile 或用户作用域；禁止使用跨用户全局“当前 Profile/Provider”。
- 自动化测试必须覆盖相邻用户、相邻项目和猜测 ID 的拒绝路径，不能只验证正常用户流程。

## 5. 信息架构

### 5.1 主导航

1. **工作台：** 我的 Task、运行中、待审批、异常和最近项目。
2. **项目：** 仓库、Task、成员、策略和设置。
3. **审批中心：** 当前用户有权处理的待审批、已决与过期请求。
4. **Codex Studio：** Profiles、Providers、MCP、Plugins、Memory、Agents、Skills。
5. **团队设置：** 成员、邀请、角色、会话和组织安全。
6. **平台管理：** Runners、队列、容量、版本、审计和系统健康。

### 5.2 路由

| 路由 | 页面 | 权限 | 主要操作 |
| --- | --- | --- | --- |
| `/login` | 当前不暴露 | — | 单用户阶段由根入口自动建立本地 Session；多用户登录与邀请暂不进入当前界面 |
| `/onboarding` | 初始化向导 | 首位 Owner | 组织、Profile、Git、首个项目 |
| `/dashboard` | 工作台 | 已登录 | 发现待办、恢复 Task、创建 Task |
| `/projects` | 项目列表 | 已登录 | 搜索、筛选、创建项目 |
| `/projects/:id` | 项目概览 | 项目成员 | 活跃 Task、仓库状态、成员 |
| `/projects/:id/tasks` | 项目 Task | 项目成员 | 筛选、创建、归档、恢复 |
| `/projects/:id/settings` | 项目设置 | Project Admin | 仓库、成员、策略、保留期 |
| `/tasks/:id` | Task 工作区 | Task 可见 | 对话、控制、审批、Diff、交付 |
| `/approvals` | 审批中心 | 有审批权限 | 同意、拒绝、查看上下文 |
| `/codex/profiles` | Profiles | 已登录 | 健康、认证、重启、恢复 |
| `/codex/providers` | Providers | Developer | 创建、编辑、选择、刷新模型 |
| `/codex/mcp` | MCP 与 Tools | Developer | 配置、OAuth、状态、测试 |
| `/codex/plugins` | Plugins | Developer | 浏览、安装、升级、停用、卸载 |
| `/codex/memory` | Memory | Developer | 健康、连续性、导出、重置 |
| `/codex/agents` | Native Agents | Developer | 管理 Agent 与多 Agent 参数 |
| `/codex/skills` | Skills | Developer | 创建、验证、测试、发布、回滚 |
| `/settings/team` | 团队设置 | Owner | 邀请、角色、禁用、会话吊销 |
| `/admin/runners` | Runner 管理 | Platform Admin | 暂停、排空、恢复、版本检查 |
| `/admin/audit` | 审计 | Owner/授权管理员 | 查询、导出安全事件 |

### 5.3 Task 工作区布局

桌面使用三栏：

- 左栏：Task 信息、Run 历史、子 Thread 树和状态。
- 中栏：活动流、消息、计划、工具与 Composer。
- 右栏：Changes、Files、Logs、Approvals 和 Run Details。

平板将右栏改为可切换 Inspector；手机使用 Activity、Changes、Approvals、Details 四个底部 Tab。手机端必须支持观察、回复、停止和审批，不要求完成复杂多文件 Diff 审查。

## 6. 核心用户流程

每个流程均要求正常路径、异常路径和明确终态。

### WF-01 首次初始化

- 前置：部署中不存在 active Organization。
- 正常：首位管理员创建组织，建立个人 Profile，配置 Provider/身份，验证 Git 凭据并创建首个项目。
- 异常：Profile 验证失败可返回修改；Git 验证失败可保存草稿但项目不可运行；初始化完成后入口永久关闭。
- 终态：Organization active，至少一名 Owner；Profile 为 ready 或明确 blocked；项目为 ready 或 setup_failed。

### WF-02 邀请加入

- 前置：邀请有效且用户未被禁用。
- 当前正常流程：Server 确保隐式本地 Owner、Organization 与 Profile 绑定，浏览器自动
  获取本地 Session 并进入工作台，不显示登录或注册界面。
- 当前异常流程：本地 Owner、Membership、Profile 或 Session 无法建立时显示启动错误，
  不回退到登录或注册表单。
- 当前终态：本地 Session 绑定 Organization；服务端仍按 Session、Profile 和资源归属
  执行授权。邀请、多成员登录和成员禁用流程暂不进入当前单用户界面。

### WF-03 创建项目

- 前置：拥有 `project.create` 和可用 Git Credential。
- 正常：输入 Git URL，测试连接，读取默认分支，创建只读镜像，配置成员与执行策略。
- 异常：DNS、认证、仓库不存在、分支不存在分别返回结构化错误；重试不重复创建项目。
- 终态：Project 为 ready 或 setup_failed；只有 ready 可创建 Run。

### WF-04 配置 Profile 与 Provider

- 前置：用户已登录。
- 正常：创建隔离 Profile Home，配置 OpenAI 或第三方 Provider，使用 Secret 引用注入凭据，刷新 Provider 模型列表，选择默认模型。
- 异常：Base URL 非 HTTPS、模型接口不兼容、凭据错误、模型目录为空时保存草稿但 Profile 不进入 ready。
- 终态：Profile 为 ready/degraded/auth_required/incompatible；任何响应不含 Secret 明文。

### WF-05 创建 Task

- 前置：Project ready、Profile 可用、用户拥有 `task.create`。
- 正常：输入目标，选择基线分支、Provider/模型、推理等级、Agent、附件和审批策略；通过幂等键创建 Task 与 queued Run。
- 异常：分支消失、附件失败、能力不兼容、配额不足时保留草稿并给出修复入口。
- 终态：Task active，Run queued；创建者获得初始 Control Lease。

### WF-06 排队、Workspace 选择与 Runtime 准备

- 正常：用户从有权使用的 Workspace 中选择执行目录；需要新的托管 clone/worktree
  时先显式创建独立 Workspace。调度器领取 Run 后验证授权，Profile Host 启动或
  复用 app-server，并通过官方 `thread/start`、`thread/resume` 或
  `thread/settings/update` 合同传递 `cwd`。创建或恢复 Thread 不创建 checkout。
- 异常：Workspace 未授权、`cwd` 越界或目录不可用时拒绝启动；容量不足保持
  queued；凭据失败进入 blocked/failed；取消 provisioning 必须停止后续步骤。
- 终态：Run running/cancelled/failed，不允许永久停在 provisioning。

### WF-07 运行中交互

- 正常：Lease 持有者发送、Steer 或排队消息；所有成员观察有序事件；子 Agent 在 Thread 树中展示。
- 异常：重复请求按幂等键去重；Lease 失效拒绝写入；断线后通过游标补发；不认识的事件保留并标记 unknown。
- 终态：Turn completed/interrupted/failed 或等待审批/输入。

### WF-08 审批与结构化输入

- 正常：Codex Server Request 先落库，再通知有权限用户；用户查看风险和影响范围后同意、拒绝或提交输入。
- 异常：并发决策只有首个有效；过期、Run 已终止、Profile 已重启时进入 expired/cancelled；不得复用旧 request ID。
- 终态：Approval approved/denied/expired/cancelled，并产生不可变审计。

### WF-09 Control Lease 转移

- 正常：当前持有者释放，或其他成员申请并经授权接管；草稿不随 Lease 转移。
- 异常：持有者失联后按 TTL 回收；强制接管要求额外权限和原因。
- 终态：同一 Task 同一时刻最多一个有效 Lease。

### WF-10 故障恢复与继续

- 正常：浏览器重连补发事件；Server/Host 重启后从数据库、Profile 和 Git 三方
  核对状态；用户可创建后继 Run，Codex 恢复原 Thread 及其当前 `cwd`，平台重新
  验证对应 Workspace 权限。
- 异常：Thread 缺失、Workspace 损坏、版本不兼容分别进入 blocked，并给出只读诊断或新建 Thread 选择。
- 终态：Run 恢复 running/waiting，或进入 interrupted/failed/blocked，不保持伪 running。

### WF-11 Diff、Commit 与 Push

- 正常：用户审查文件、二进制和大文件摘要，填写 Commit Message，平台再次读取 Git 状态并提交；Push 前验证远端领先和保护策略。
- 异常：工作树变化、无变更、身份缺失、远端领先、认证失败分别处理；禁止 Force Push。
- 终态：Commit/Push 成功并审计，或保持可恢复错误且不丢变更。

### WF-12 归档与清理

- 正常：归档 Task，终止活动 Run，保留审计与 Thread 映射，并释放 Task 持有的
  Workspace 授权引用。Workspace 不随 Thread/Task 归档自动删除；显式删除托管
  Workspace 前必须确认没有其他授权引用、活动进程或未交付变更。
- 异常：清理失败进入重试队列并告警；不得删除 Profile Home 或其他 Task 数据。
- 终态：Task archived；Workspace 保持原有状态，除非另一个显式 Workspace
  生命周期操作改变它。

### WF-13 Codex Studio 操作

- 正常：UI 先读取 Manifest，再启用支持的 Provider/MCP/Plugin/Memory/Agent/Skill 操作；写操作展示影响与权限。
- 异常：unsupported、experimental 未授权、版本漂移或操作部分失败时不得伪装成功。
- 终态：Runtime 原生状态与平台显示一致，操作产生审计和刷新结果。

### WF-14 Codex 上游升级

- 正常：创建专用同步分支，解决冲突，生成合同，运行回放/Smoke，构建带摘要镜像，进入 canary，再扩大部署。
- 异常：合同破坏、Profile 恢复失败或错误率上升时回滚 Web Feature Policy 和 Codex 构建，不改写 Profile Home。
- 终态：兼容矩阵记录 compatible/incompatible/rolled_back。

## 7. 功能需求

优先级：P0 为对应版本门禁；P1 为版本内应完成；P2 可延期。

### 7.1 认证、组织与权限

| ID | P | 需求 |
| --- | --- | --- |
| AUTH-001 | P0 | 使用 HttpOnly、Secure、SameSite Session Cookie，不在 URL 保存 Token |
| AUTH-002 | P0 | 登录、登出、过期、吊销和并发 Session 有明确行为 |
| AUTH-003 | P0 | 登录、邀请、敏感操作支持速率限制和安全审计 |
| AUTH-004 | P1 | 管理员可查看并吊销成员 Session |
| ORG-001 | P0 | 单部署只能激活一个 Organization |
| ORG-002 | P0 | 支持邀请、角色变更、禁用和恢复成员 |
| ORG-003 | P0 | 最后一名 Owner 不可被降权或禁用 |
| RBAC-001 | P0 | 每个读写 API 在服务端校验组织、项目、Task 与动作权限 |
| RBAC-002 | P0 | 不存在和无权限资源对普通成员使用相同外部表现 |
| RBAC-003 | P1 | 支持项目级成员覆盖和只读 Reviewer/Viewer |

### 7.2 项目、仓库与凭据

| ID | P | 需求 |
| --- | --- | --- |
| PRJ-001 | P0 | 仅允许通过受控 Git URL 创建项目，不接受服务器任意本地路径 |
| PRJ-002 | P0 | 结构化区分 URL、DNS、认证、仓库和分支错误 |
| PRJ-003 | P0 | Repository Mirror 与可写 Workspace 是独立资源；托管 clone/worktree 只能显式创建，生命周期不绑定 Thread 或 Run |
| PRJ-004 | P0 | Git Credential 以 Secret 引用保存，API 不返回明文 |
| PRJ-005 | P1 | 支持项目默认分支、成员、Profile/模型和审批策略 |
| PRJ-006 | P1 | 支持重新验证、Fetch、归档和受控危险操作 |

### 7.3 Profile、Provider 与模型

| ID | P | 需求 |
| --- | --- | --- |
| PROF-001 | P0 | 每个成员默认一个隔离持久 Profile Home |
| PROF-002 | P0 | 同一 Profile 主 app-server 进程使用跨进程锁保证唯一 |
| PROF-003 | P0 | Profile 状态、健康、构建版本、能力和最近错误可查询 |
| PROF-004 | P0 | Host 重启后使用原 Profile Home 恢复 Thread 和配置 |
| PROF-005 | P1 | 支持停止、重启、重新认证和受控重置 |
| MOD-001 | P0 | 支持 OpenAI 与 OpenAI-compatible 第三方 Provider |
| MOD-002 | P0 | Provider 配置包含 ID、名称、Base URL、Secret 引用、Wire API 和默认模型 |
| MOD-003 | P0 | 模型目录按 Provider 隔离，支持强制刷新和缓存身份校验 |
| MOD-004 | P0 | Turn 必须携带或解析到明确 Provider，禁止错用另一 Provider 模型 |
| MOD-005 | P1 | 支持模型级上下文窗口、推理等级和能力展示 |
| MOD-006 | P1 | 编辑当前 Provider 后刷新模型；当前 Provider 不可直接删除 |

### 7.4 Task、Run 与实时事件

| ID | P | 需求 |
| --- | --- | --- |
| TASK-001 | P0 | Task 具有稳定 ID、标题、目标、Project、Owner 与 Thread 映射 |
| TASK-002 | P0 | 创建 Task 使用幂等键，重复提交只产生一个 Task/Run |
| TASK-003 | P0 | 支持归档、恢复、筛选、搜索和 Run 历史 |
| RUN-001 | P0 | Run 完整实现 queued→provisioning→running→终态状态机 |
| RUN-002 | P0 | Scheduler 使用领取租约、心跳和超时回收避免重复执行 |
| RUN-003 | P0 | 每个 Run 必须验证 Thread 当前 `cwd` 位于经授权 Workspace 内，不能创建、拥有或隐式切换 checkout |
| RUN-004 | P0 | 支持取消、继续、失败诊断和清理重试 |
| EVT-001 | P0 | WebSocket 事件具有单 Task 单调序号和恢复游标 |
| EVT-002 | P0 | 页面重连按游标补发，重复事件可幂等应用 |
| EVT-003 | P0 | 未知事件不得导致连接中断，并记录兼容性指标 |
| EVT-004 | P1 | 大输出分块、限长并转存 Artifact，事件正文有硬上限 |
| EVT-005 | P1 | Agent 回复可在正文任意位置嵌入平台结构化卡片引用；卡片 payload 由平台鉴权、持久化和限额控制，浏览器按 capability 渲染，不把原始 app-server 协议暴露给用户 |
| MAP-001 | P1 | 地理相关回复支持地图卡片，可表达点、线、面、边界、路线、距离和地理数据可视化结果 |
| MAP-002 | P1 | 地图卡片使用服务端生成并持久化的 GeoJSON Artifact 引用，避免要求 LLM 在回复中逐字输出大型 GeoJSON |
| MAP-003 | P1 | 地图卡片支持样式解析、Mapbox GL 渲染、错误占位、移动端可用布局和全屏查看 |
| MAP-004 | P1 | 地图卡片触发以提示模板和平台后处理为主；除非官方 Runtime 缺少必要边界，不在 `codex/` 增加地理业务逻辑 |

### 7.5 审批、输入与协作

| ID | P | 需求 |
| --- | --- | --- |
| APR-001 | P0 | Server Request 在通知用户前持久化并绑定 Profile/Task/Run/Thread |
| APR-002 | P0 | 支持命令、文件、权限、用户输入和 MCP elicitation 类型 |
| APR-003 | P0 | 决策使用 CAS/版本号保证并发只有一个成功 |
| APR-004 | P0 | 过期、Run 终止和 Profile 重启后不得错误复用请求 |
| APR-005 | P0 | 展示命令、路径、权限变化、风险、发起 Agent 和超时 |
| COL-001 | P0 | 一个 Task 同时最多一个有效 Control Lease |
| COL-002 | P0 | Lease 具有 TTL、续约、释放、申请和强制接管审计 |
| COL-003 | P1 | 评论与 Agent 消息分离，评论不进入模型上下文 |
| COL-004 | P1 | 通知覆盖待审批、完成、失败、Lease 请求和系统告警 |

### 7.6 Git 与交付

| ID | P | 需求 |
| --- | --- | --- |
| GIT-001 | P0 | Changes 展示新增、修改、删除、重命名、二进制和大文件 |
| GIT-002 | P0 | Commit 前重新读取状态并确认选中文件仍一致 |
| GIT-003 | P0 | Commit 作者、Message、文件数和结果进入审计，代码正文不进入分析 |
| GIT-004 | P0 | Push 前检测远端领先、认证与保护分支，禁止 Force Push |
| GIT-005 | P1 | Push 失败保留本地 Commit 并给出 Fetch/Rebase/人工处理建议 |
| GIT-006 | P1 | 支持测试报告、补丁和日志 Artifact 下载权限 |

### 7.7 Codex Studio

| ID | P | 需求 |
| --- | --- | --- |
| CAP-001 | P0 | 所有 Studio 模块由 Manifest 能力、版本、状态和策略共同门控 |
| CAP-002 | P0 | incompatible/unsupported 显示原因、要求版本和修复入口 |
| MCP-001 | P0 Beta | 展示 Server 状态、认证、Tools、Resources 和错误 |
| MCP-002 | P0 Beta | 支持 Secret 引用配置、Reload、OAuth 和 elicitation |
| MCP-003 | P1 | 测试调用展示 Schema、权限和结构化结果，调用受审计 |
| PLG-001 | P1 | 支持 Marketplace、Plugin list/read/install/uninstall |
| PLG-002 | P1 | 安装/升级前展示来源、完整性、能力和权限变化 |
| MEM-001 | P1 | 展示 compaction、连续性、容量和错误，不展示 Memory 正文 |
| MEM-002 | P1 | 导出和重置为危险操作，要求二次确认与审计 |
| AGT-001 | P1 | 管理原生 Agent 配置并展示校验/Reload 结果 |
| AGT-002 | P1 | 展示父子 Thread、角色、状态、委派、等待、返回和失败 |
| SKL-001 | P1 | 支持个人与项目 Skill 的读取、创建、验证、测试和发布 |
| SKL-002 | P1 | 项目 Skill 通过 Git 版本化；个人 Skill 保存在 Profile |

### 7.8 平台管理

| ID | P | 需求 |
| --- | --- | --- |
| ADM-001 | P0 | Runner 展示 healthy/draining/offline/version_mismatch |
| ADM-002 | P0 | 支持暂停领取、排空、终止卡死 Run 和清理重试 |
| ADM-003 | P0 | 展示队列、磁盘、Profile 进程、版本和合同健康 |
| ADM-004 | P1 | 审计按用户、项目、Task、动作、结果和时间检索 |
| ADM-005 | P1 | 敏感审计导出需要额外权限并生成导出审计 |

## 8. 状态模型

### 8.1 Profile

```text
creating -> auth_required -> starting -> ready
                  |            |         |
                  v            v         v
                failed      degraded   stopping -> stopped
                                           |
                                           -> starting

任意可运行状态 --版本不兼容--> incompatible
```

- `ready` 才能领取新 Run。
- `degraded` 可以继续已验证的低风险操作，但不得启用缺失能力。
- `incompatible` 不启动新 Run，只允许诊断、导出和回滚。

### 8.2 Run

```text
queued -> provisioning -> running -> completed
  |            |             |  \-> waiting_approval -> running
  |            |             |  \-> waiting_input -> running
  |            |             |  \-> interrupted -> queued(continue)
  |            |             \----> failed
  |            \------------------> failed/cancelled
  \-------------------------------> cancelled
```

终态：`completed`、`failed`、`cancelled`。`interrupted` 是可继续状态，不伪装为 running。

### 8.3 Workspace

```text
creating -> ready -> in_use -> retained -> removing -> removed
    |          |        |          |           |
    \----------+--------+----------+----------> cleanup_failed
```

Workspace 状态独立于 Thread 和 Run。`in_use` 表示存在活动使用者，不表示被某个
Thread 独占；删除是单独的授权操作，不由 Thread/Task 归档隐式触发。

### 8.4 Approval

```text
pending -> approved
        -> denied
        -> expired
        -> cancelled
```

所有终态不可逆；重复决策返回当前终态而不是再次调用 Codex。

### 8.5 Control Lease

```text
requested -> active -> released
                    -> expired
                    -> revoked
```

Lease 使用数据库时间与版本号；客户端时间不能决定有效性。

## 9. 权限矩阵

| 操作 | Owner | Project Admin | Developer | Reviewer | Viewer | Platform Admin |
| --- | --- | --- | --- | --- | --- | --- |
| 邀请/改角色 | ✓ | — | — | — | — | — |
| 创建项目 | ✓ | 可配置 | — | — | — | — |
| 项目设置/成员 | ✓ | ✓ | — | — | — | — |
| 创建 Task | ✓ | ✓ | ✓ | — | — | — |
| 发送/Steer/停止 | ✓ | ✓ | ✓+Lease | — | — | — |
| 查看 Task/Diff | ✓ | ✓ | ✓ | ✓ | ✓ | 需项目权限 |
| 处理审批 | ✓ | 按策略 | 按策略 | 按策略 | — | — |
| Commit | ✓ | ✓ | 按策略 | — | — | — |
| Push | ✓ | 按策略 | 按策略 | — | — | — |
| Provider/MCP 配置 | ✓ | — | 个人范围 | — | — | 仅基础设施 |
| Runner 管理 | 可授权 | — | — | — | — | ✓ |
| 查看敏感审计 | ✓ | 项目范围 | 自己 | 自己 | — | 可授权 |

所有矩阵为默认值；服务端仍需检查资源归属、状态、能力和策略。

## 10. API 与交互规则

- 浏览器只访问平台 REST/JSON API 与认证 WebSocket，不直接访问 app-server。
- 写 API 接受 `Idempotency-Key`；异步操作返回资源与状态，不返回“已启动”字符串。
- 错误格式至少包含 `code`、`message`、`category`、`retryable`、`requestId`，可选 `details` 不含 Secret。
- 持续状态使用页面状态条或资源状态，不用短暂 Toast 代替离线、重连、版本不兼容和维护状态。
- 破坏性操作展示对象、影响范围和不可逆后果；输入资源名称确认仅用于高危操作。
- 浏览器历史必须恢复 Tab、筛选和选中文件，不恢复失效写表单。
- URL 只包含稳定资源 ID 和非敏感视图参数，不包含 Token、Prompt、路径或代码正文。

## 11. 非功能需求

### 11.1 性能与容量目标

| 指标 | Alpha | Beta/V1 |
| --- | --- | --- |
| 普通 API P95 | < 500ms | < 300ms（不含外部 Git/Codex） |
| WebSocket 事件到 UI P95 | < 1s | < 500ms |
| 断线补发 1,000 事件 | < 10s | < 5s |
| Task 创建到 queued | < 2s | < 1s |
| warm Profile queued→running P95 | < 20s | < 10s |
| cold Profile queued→running P95 | < 60s | < 30s |
| 单 Task 可浏览事件 | 10,000 | 100,000，分页/归档 |
| 单组织并发 Run | 5 | 初始目标 20，压测后固定 |
| 单 Profile 并发 Thread | 以 Manifest/实测为准，不硬编码 |

### 11.2 可用性与恢复

- Beta 月度可用性目标 99.5%，GA 目标 99.9%（计划维护除外）。
- Web Server RPO ≤ 5 分钟，RTO ≤ 30 分钟。
- Runner/Host 心跳丢失后 60 秒内将 Run 标记为 suspect，租约到期后进入 interrupted/failed。
- 任何 Run 不得在无心跳、无事件、无租约时永久显示 running。
- Profile Home、数据库和仓库镜像恢复必须有定期演练证据。

### 11.3 安全

- 生产只允许 HTTPS/WSS；禁止共享 Token、查询参数 Token 和 `Access-Control-Allow-Origin: *`。
- Session Cookie 使用 HttpOnly、Secure、SameSite；写请求具备 CSRF 防护。
- Password/API Key/OAuth Token/Git Credential 使用 Secret 管理或加密存储。
- Runner 默认 rootless、最小文件系统挂载和受控出网。
- 防护 SSRF、路径穿越、符号链接逃逸、Git 参数注入、恶意归档和超大输出。
- 日志、分析和错误上报不包含 Secret、Prompt、Memory 正文或完整代码正文。
- 依赖漏洞按严重级别设修复 SLA：Critical 72 小时、High 14 天、Medium 30 天。

### 11.4 可访问性与兼容性

- 目标 WCAG 2.2 AA。
- 核心流程支持键盘、可见焦点、Screen Reader、200% 缩放和减少动态。
- 支持当前及前一主版本 Chrome、Edge、Safari；Firefox 作为 Beta 兼容目标。
- 关键状态不只依赖颜色；动态日志提供暂停自动滚动。

## 12. 数据保留与隐私

| 数据 | 默认保留 | 可配置 | 删除原则 |
| --- | --- | --- | --- |
| Session | 30 天/主动吊销 | 是 | 吊销后立即失效 |
| 审计 | 180 天 | 90–365 天 | 不允许普通用户删除 |
| Run 事件投影 | 90 天 | 是 | 不删除 Codex 原生 Thread |
| 托管 Workspace | 显式删除或 Project 保留策略触发后 7 天 | 1–30 天 | 清理前确认无活动使用者、未交付变更和其他授权引用 |
| Artifact | 30 天 | 是 | 按引用和保留策略删除 |
| Profile Home | Membership 有效期 | 管理策略 | 危险操作、备份和审计 |
| Repository Mirror | Project 生命周期 | 否 | 项目删除流程清理 |

用户删除或禁用不自动删除组织拥有的 Task/审计；Profile 删除需要单独的数据治理流程。

## 13. 产品指标

只记录资源 ID、状态、耗时和错误分类，不记录 Prompt、代码、Diff 或 Secret。

| 指标 | 定义 |
| --- | --- |
| Activation | 新用户 24 小时内完成首个成功 Run |
| Task success rate | Run completed 且无平台故障的 Task 比例 |
| Time to first useful output | 创建 Task 到首个 Agent 内容/计划 |
| Approval wait time | pending 到终态的 P50/P95 |
| Recovery success | 断线/重启后无需人工修库的恢复比例 |
| Delivery rate | completed Task 中 Commit 或 Push 的比例 |
| Runtime compatibility | 部署 Codex 构建通过全部合同门禁的比例 |
| Unknown event rate | 未识别 app-server 事件/总事件 |

## 14. 发布范围与门禁

### 14.1 Alpha

单管理员、单部署、单或少量项目。必须完成：项目导入、Profile/Provider、
Task/Run、授权 Workspace、Thread/Turn、实时事件、审批、取消/继续、Diff、Commit、
浏览器刷新恢复和 Host 重启恢复。

Alpha 不承诺多用户、Push、完整 Studio 或生产 SLA。

### 14.2 Beta

邀请制多用户。增加 RBAC、每用户 Profile、Control Lease、持久审批/事件、审计、rootless Runner、Push、Provider 管理和 MCP inventory/OAuth/elicitation。

Beta 门禁：两名用户并发故障注入无串流；备份恢复演练通过；无 Critical 安全问题。

### 14.3 V1 GA

完成容量、安全、浏览器、可访问性、升级回滚和运维手册。Studio 模块仅发布通过独立能力门禁的部分。生产形态只有浏览器与平台服务，不包含本地桌面运行时。

## 15. 风险与产品处理

| 风险 | 影响 | 处理 |
| --- | --- | --- |
| Codex 上游快速变化 | 合同漂移、同步冲突 | 固定构建、生成合同、专用同步分支、canary |
| 定制 Provider 与上游重叠 | 长期维护成本 | 优先采用上游实现，保持定制 seam 最小 |
| 单 Profile 单进程容量不足 | 同用户并发排队 | 实测限制、Profile 队列；不擅自多实例共享 Home |
| Profile/Memory 串用 | 隐私与身份事故 | 一用户一 Home、路径/Thread 归属校验、跨用户测试 |
| Agent 恶意命令 | 宿主/数据泄露 | rootless、出网策略、审批、最小凭据 |
| 事件与数据库不一致 | UI 伪 running/重复审批 | 序号、幂等、租约、巡检与三方恢复 |
| V1 范围膨胀 | Alpha 长期不可用 | Studio 不阻塞纵向闭环，按能力独立发布 |
| 平台功能回归 | 浏览器纵向闭环不可用 | 合同、PostgreSQL 集成、真实 app-server 与浏览器 E2E 共同门禁 |

## 16. 待决策项

以下事项必须在对应研发任务开始前关闭：

1. Alpha 首发仅 Linux Runner，还是同时支持 macOS Runner。
2. Git 首发采用通用 HTTPS/SSH Credential，还是优先 GitHub App。
3. Alpha 登录采用本地 Owner 凭据还是直接接 OIDC；生产必须支持可吊销会话。
4. 默认托管 Workspace、Artifact、事件和审计保留时间。
5. 第三方 Provider 是否允许组织管理员设置域名白名单与出网策略。
6. MCP/Plugin 安装来源白名单与签名/完整性要求。
7. Alpha 是否包含 Push；本 PRD 默认 Alpha 只要求 Commit，Beta 要求 Push。
8. Codex 首个兼容冻结点：先同步当前官方 main，还是选择最近稳定 Tag/构建。

## 17. 总体验收标准

V1 只有在以下条件全部满足时才能发布：

- 两名以上用户可同时运行不同 Task，Profile、事件、Secret、Workspace 无串流。
- 每个 Thread/Chat 的当前 `cwd` 都位于经授权 Workspace 内；Workspace 独立于
  Thread/Run 并可被多个 Thread 使用。创建或恢复 Thread/Run 不隐式创建 checkout，
  普通用户不能注册任意服务器路径。
- 同一 Profile 使用持久 Home 和唯一主 app-server；Host 重启后恢复 Thread、Provider 与记忆连续性。
- 页面刷新、网络断开、Server/Host/Runner 重启后 Run 进入可解释状态。
- 审批、Lease、Commit、Push 和危险 Studio 操作均可追溯。
- Provider 模型目录按 Profile/Provider 隔离，切换 Provider 不使用旧目录或旧 Secret。
- 多 Agent 父子关系和协作 Item 在 Web 可见，平台不存在自建调度器。
- MCP/Plugin/Memory/Agent/Skill 状态与固定 Codex 构建查询一致；缺失能力只禁用。
- Codex 构建通过 Schema、Manifest、Fixture、真实 Smoke、升级和回滚测试。
- 安全测试无法通过 URL Token、CSRF、SSRF、路径穿越、跨项目 ID 或日志取得凭据。
- 完整用户流程不要求桌面程序、浏览器扩展或本地桥接进程。
- 生产部署、备份、恢复、告警和升级 Runbook 已由非作者执行验证。
