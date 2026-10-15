# Web 平台本地运行手册

本文只说明当前单机环境的启动、配置、纵向验证和排错操作。它不定义产品范围，也不
把本地启动成功视为生产或安全门禁通过；当前能力与发布缺口分别见
[能力基线](capability-baseline.md) 和 [开发计划](development-plan.md)。

当前本地链路与生产边界一致：

```text
Browser -> open-web-codex-server -> PostgreSQL
                              \-> Profile Host -> codex app-server
                              \-> Workspace/Git -> authorized execution roots
                              \-> Run Orchestrator -> schedule and audit Runs
```

浏览器只访问同源 REST 和认证 WebSocket。仓库中没有本地 sidecar、无认证
Gateway、原始 JSON-RPC 路由或桌面应用。

## 前置条件

### 更换对话的 Provider 与模型错误排查

点击新建（如有 Copilot 则先选包）后先进入空白草稿，此时可选择 Provider 和模型，
输入内容在切换时保留；首条消息发送时才创建并绑定 Runtime Thread。
已有对话使用 Runtime 保存的 Provider；同 Provider 内可直接更换模型。
修改默认 Provider 不会迁移已有对话。
跨 Provider 的旧 Task 选择会在发送前被明确拒绝，不会继续把新模型名发往旧地址。

终态错误显示脱敏分类和可用的 HTTP 状态码；只有 `willRetry=true` 才显示重连。
`Model metadata ... not found` 是另一个问题：Runtime 没有找到 exact model ID 的元数据，
不代表 API Key 无效。应核对该 Provider 的模型目录与配置是否已加载；仅有模型 ID 或
一次 `hi` 成功不能证明上下文限制、ToolSearch 等能力。不要为了消除 warning 修改模型名、
猜测能力或屏蔽 warning。

### 在网页中配置或更换地图 Token

点击左下角齿轮 **Settings → 地图服务 → 配置 / 更换地图 Token**，选择 Mapbox 或 Google，
在专用密码框中填写 Key 并保存。当前只有具备配置权限的组织 owner/admin 可以修改。
不需要先产生地图卡片或等待审批，也不要把 Key 作为普通聊天消息发送。
保存后回到原对话重试；保存成功仅表示配置已写入，不表示外部服务鉴权通过。

Web Tool 的 manifest 显式选择 `--credential-source platform`；每次凭据操作通过已有 MCP
URL elicitation 从平台取得当前配置，不读写 provider-local memory，因此修改后下一次操作
立即使用新 provider/key。实际 Key 由 Server 直接送达 MCP，不进入模型上下文。

### 运维导出当前地图 Key

只需复制单文件 `scripts/export-maps-key.py` 到生产主机，使用 Python 3.10+ 运行，
不需要仓库或 Rust/Cargo。数据库模式按当前 Secret Store 的 AES-256-GCM/AAD 合同只读
`maps.active_credential`，不迁移数据库或重启服务。标准输出只有明文 Key，
provider 与错误写到标准错误，不要把输出提交到源码或日志。

默认读取 `OPEN_WEB_CODEX_DATA_DIR`（未设置时为当前目录）中的
`database-url` 和 `master-key`；`DATABASE_URL`、`OPEN_WEB_CODEX_MASTER_KEY` 优先。
生产 Secret Manager 注入的主密钥必须与服务端一致，Key version 使用
`OPEN_WEB_CODEX_MASTER_KEY_VERSION`（默认 `v1`）。也可显式指定文件：

```bash
python3 export-maps-key.py --data-dir /srv/open-web-codex/data
python3 export-maps-key.py --database-url-file /run/secrets/database-url --master-key-file /run/secrets/master-key
```

数据库模式需要 `psycopg[binary]==3.3.6` 与 `cryptography==46.0.3`；当前 Python 缺少依赖时，
脚本联网安装到临时 venv，子进程结束后清理，不改系统 Python。离线主机可预装依赖并使用
`--no-install`。独立运行且显式使用 local credential source 的 MCP 可导出其 v2 凭据文件：
`python3 export-maps-key.py --credential-file /path/to/maps-tool-memory.json`，此模式只用标准库。
它不是 Web 平台当前 Key 的来源；Web Key 必须从上面的 Secret Store 模式导出。

没有当前配置、数据库不可达或密钥不匹配时明确失败，不搜索其他 Profile 或历史凭据。

### 运行依赖

- Node.js 20+、npm、稳定 Rust、Git。
- PostgreSQL Server 已运行；Release 部署器可以创建或连接固定名称的
  `open_web_codex` 数据库，开发脚本要求该数据库已存在。
- 真实模式由 `run-local` 使用当前仓库按精确构建指纹生成的 Codex。外部 Binary 即使能够启动，
  也不能证明包含本项目 Patch Map 保留的 Runtime seam，因此本地启动入口不接受 `CODEX_BIN`。

开发脚本默认优先读取受保护的本地连接文件：

```text
.local/open-web-codex/database-url
```

也可以通过 `DATABASE_URL`、`--database-url` 或 `--database-url-file` 覆盖。连接文件
必须只允许当前用户读取（权限 `600`）。

## 启动

单机 Release 部署使用统一入口：

```bash
./scripts/deploy.sh
```

它负责校验 PostgreSQL 和部署策略，再把 Release 构建、target 回收、服务替换及启动
健康门禁委托给唯一的生命周期所有者 `run-local.sh`。平台 Server 和 Runtime 使用与
本地启动相同的精确 Cargo 依赖指纹，未变化的 Release 二进制不会重新构建。构建成功
后才停止并替换现有 Server，失败不会提前中断当前服务。部署策略输出写入
`.local/open-web-codex/logs/deploy.log`，构建与启动详情写入
`.local/open-web-codex/logs/run-local.log`。生产页面由 Server 同源托管在
`http://127.0.0.1:4800/web`。

没有 `DATABASE_URL`、`--database-url-file` 或已保存配置时，交互部署会询问：

1. 使用已有 PostgreSQL：输入主机、端口、用户名和密码并先验证连接。
2. 创建数据库：输入管理员凭据、应用用户名和应用密码；只在角色或数据库缺失
   时创建，不覆盖已有角色密码或数据库所有权。

数据库名固定为 `open_web_codex`，不能通过环境变量或参数改成其他名称。密码输入
不回显，也不会出现在 Server 进程参数或部署日志中。生成的连接配置保存在
`.local/open-web-codex/database-url`，权限为 `600`；之后重复部署自动复用。
非交互环境缺少配置时会在编译前失败，必须显式提供受保护的 URL 文件或
`DATABASE_URL`。

```bash
./scripts/deploy.sh --check
./scripts/deploy.sh --status
./scripts/deploy.sh --stop
```

外部管理的含密码连接建议使用 `--database-url-file`。生产主机必须从 Secret Manager
提供稳定的 `OPEN_WEB_CODEX_MASTER_KEY`，并在 `127.0.0.1:4800` 前配置 HTTPS
反向代理。脚本是当前单机 Release 部署入口；OS 服务守护、备份恢复和滚动升级
仍属于 GA 门禁。

用 Fake Runtime 启动同源 WebApp 与 Server：

```bash
./scripts/run-local.sh --fake --background
```

用真实 Codex 启动：

```bash
./scripts/run-local.sh --background
```

脚本在 `4800` 启动平台 Server，并在 `http://127.0.0.1:4800/web`
同源提供 WebApp、类型化 REST 和认证 WebSocket；不启动独立 Vite、
4732/4733 daemon 或 Gateway 进程。真实模式默认使用仓库 Codex Binary；
Fake 模式只用于 Server/WebApp 联调。
本地 Secret Store 主密钥首次运行时生成在
`.local/open-web-codex/master-key`，权限为仅当前用户可读；生产部署必须从外部
Secret Manager 注入 `OPEN_WEB_CODEX_MASTER_KEY`。

前端需要热更新时，先保持 4800 Server 运行，再从 `apps/web` 执行
`npm run dev`。Vite 默认监听 `http://127.0.0.1:1420`，只作为可丢弃的开发工具，
并将 API 与 WebSocket 代理到 4800；它不属于平台服务生命周期。

含密码的数据库 URL 推荐放在仅当前用户可读的文件中：

```bash
printf '%s\n' 'postgresql://user:password@host:5432/open_web_codex' > .local/database-url
chmod 600 .local/database-url
DATABASE_URL="$(<.local/database-url)" ./scripts/run-local.sh --background
```

后台管理：

```bash
./scripts/run-local.sh --background
./scripts/run-local.sh --status
./scripts/run-local.sh --restart
./scripts/run-local.sh --stop
```

## 不兼容开发 schema 的显式重建

`run-local` 和 `--restart` 永远不会猜测、修补或删除已有数据库行。当前 schema 不再兼容
旧的内联地图卡片投影：卡片必须指向 provider-owned `map_card_spec`，不能从旧 renderer payload
反推这个引用。因此，遇到 migration 因旧开发数据停止时，必须显式重建开发库，而不是将新列改为
可空或在迁移中伪造引用。

若本地开发库使用受保护的 URL 文件、非默认数据库名或其他开发 PostgreSQL，则使用确认式重建脚本：

```bash
./scripts/rebuild-development-database.sh \
  --confirm-development-only \
  --database-url-file .local/open-web-codex/database-url
```

该脚本会重新创建 URL 指向的数据库，先导出再恢复 Provider/模型所需的组织、用户、Profile、加密
Provider 配置、默认模型选择和加密 Maps 凭据；不会保留旧 Platform 投影、Thread/Run、地图卡片或其他
不兼容的开发数据。它不会输出或解密连接凭据和 Secret。完成后重新运行
`./scripts/run-local.sh --background`，并检查健康状态。

默认本地 loopback 开发库可以在一次真实模式启动中完成相同的保留式重建：

```bash
./scripts/run-local.sh --background --rebuild-development-database
```

该选项只接受默认 data directory、默认无凭据 loopback `open_web_codex` 数据库和真实 Codex 模式；
它在当前 Server 构建完成后停止旧服务、重建数据库、恢复上述配置并启动新服务。普通
`./scripts/run-local.sh --background` 仍然不删除数据。

重建在删除数据库前先导出加密 Provider/模型与 Maps 配置，并在恢复后校验完整备份和关键
Provider、Mapbox Maps 凭据均未改变。若现有数据库的这组持久化表不完整，脚本会拒绝执行，
不会以空配置继续重建。

`--restart` 先用独立的 `dev-small` Profile 完成变更，再停止并替换后台
Server，构建失败不会中断当前进程。默认启动仍构建浏览器；平台 Server、
`codex` 与 `codex-code-mode-host` 分别检查自己的精确 Cargo dep-info 指纹。
指纹覆盖最终二进制实际使用的源文件、嵌入式迁移、build-script 目录依赖、相关 crate
manifest、workspace lock/config、工具链、构建 Profile 和会改变产物的环境。
Server 指纹匹配时不调用其 Cargo 构建；Runtime 两者均匹配时不调用 Runtime Cargo，
只有一个过期时只构建该二进制，两者同时过期时合并构建。

成功构建后的可丢弃 stamp 位于
`.local/open-web-codex/build-stamps/platform-server/<profile>/` 和
`.local/open-web-codex/build-stamps/codex-runtime/<profile>/`。stamp 只在构建成功后
原子更新；源码内容变化、工具链或 Profile 变化、产物被替换，以及 target 高低水位
回收导致二进制或 `.d` 文件缺失，都会使它失效并触发正确重建。文档和测试等未进入
对应二进制 Cargo dep-info 的文件不会触发 Server 或 Runtime 构建。

确认所有已有构建输出为最新时仍可配合 `--no-build` 跳过浏览器、Server 和仓库
Codex 的全部构建检查。仓库构建默认关闭 Cargo 增量编译并在已安装时使用容量上限为
8 GiB 的 sccache；Rust 测试使用独立的 `ci-test` Profile。Web 与 Codex target
合计超过 24 GiB 时，脚本按 Profile 清理到 16 GiB，并始终保留 Release 产物。
终端展示阶段、耗时和最终服务面板；交互式终端中 Rust 构建步骤还会实时显示
Cargo 构建进度。完整的构建与环境准备详情位于
`.local/open-web-codex/logs/run-local.log`，Server 输出位于同目录的
`server.log`。失败时脚本直接显示相关日志尾部。

## 真实平台端到端验证

先用独立数据库和数据目录启动真实 Server，再从 `apps/web` 运行：

```bash
E2E_BASE_URL=http://127.0.0.1:4810 \
DEEPSEEK_API_KEY_FILE=/absolute/path/to/deepseek-key \
npm run test:e2e:real-platform
```

该用例使用真实 Codex Binary 和第三方 Chat Provider，创建独立 managed
Project、主 Thread 与延时 Thread，并验证消息流事件顺序、代码执行、文件树和
文件预览、Provider 新增/切换/上下文更新、真实 stdio MCP 调用、审批请求和
决策、Thread 运行态收敛、跨 Thread 历史恢复，以及实时事件与持久重放一致性。
密钥只从环境变量或权限受限的文件读取，不写入源码、日志或测试结果。

## 浏览器纵向流程

1. 打开 `http://127.0.0.1:4800/web`。
2. 当前单用户入口自动建立本地 Owner 与 Session，不显示登录或注册页面。
3. 创建 Git Project，平台只接受受控 Git URL，不接受浏览器本地路径。
4. 显式创建或选择一个已授权 Workspace，再创建 Task 和 Run；Run 只引用
   `workspace_id`，不会创建或独占 checkout。
5. 向运行中的 Task 发送消息，事件先持久化再通过 WebSocket 投影。
6. 处理待审批请求；浏览器不会看到 app-server request ID 或服务器路径。
7. 在 Changes 中选择文件并显式 Commit。

## 配置

| 变量 | 作用 |
| --- | --- |
| `DATABASE_URL` | PostgreSQL 连接 |
| `DATABASE_MAX_CONNECTIONS` | 连接池大小，默认 10 |
| `CODEX_MODE` | `real` 或 `fake` |
| `CODEX_HOME` | 当前 Profile 的持久目录 |
| `OPEN_WEB_CODEX_MASTER_KEY` | Base64 32-byte Secret Store key |
| `OPEN_WEB_CODEX_RUNNER_ROOT` | 私有 mirror/workspace 根目录 |
| `OPEN_WEB_CODEX_DATA_DIR` | 本地状态、PID 和日志目录 |
| `OPEN_WEB_CODEX_BIND_HOST` | 监听地址，默认 `127.0.0.1` |
| `OPEN_WEB_CODEX_SERVER_PORT` | HTTP/WebSocket 端口，默认 `4800` |

## 验证与排错

```bash
curl --fail http://127.0.0.1:4800/api/health
./scripts/run-local.sh --status
```

启动失败先检查 PostgreSQL、`server.log`、Codex Binary 和 Profile Home 权限。
真实模式若配置 Provider Secret，还必须提供稳定的外部主密钥；更换主密钥版本
前需要完成 Secret 轮换，不能直接覆盖旧 key。

## 当前边界

- 当前 Server 组合入口一次启动一个 Profile Host；多用户 Beta 仍需按授权用户
  动态路由多个持久 Profile。
- 已支持独立托管 Workspace、授权、状态和选择性 Commit；现有执行根登记、共享
  Workspace 并发与真实 multi-`cwd` 验证仍未完成，Push 与高级 Diff 尚未作为
  浏览器资源开放。
- Session 当前使用 Bearer token；生产发布前仍需完成 HttpOnly Cookie、CSRF、
  限速、备份恢复和 Runner 强隔离门禁。
