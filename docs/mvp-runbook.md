# Web 平台本地运行手册

当前本地链路与生产边界一致：

```text
Browser -> open-web-codex-server -> PostgreSQL
                              \-> Profile Host -> codex app-server
                              \-> Run Orchestrator -> Git workspace
```

浏览器只访问同源 REST 和认证 WebSocket。仓库中没有本地 sidecar、无认证
Gateway、原始 JSON-RPC 路由或桌面应用。

## 前置条件

- Node.js 20+、npm、稳定 Rust、Git。
- PostgreSQL Server 已运行；Release 部署器可以创建或连接固定名称的
  `open_web_codex` 数据库，开发脚本要求该数据库已存在。
- 真实模式需要当前仓库构建的 Codex，或通过 `CODEX_BIN` 指定兼容 Binary。

开发脚本的默认数据库连接为：

```text
postgresql://$USER@127.0.0.1:5432/open_web_codex
```

## 启动

单机 Release 部署使用统一入口：

```bash
./scripts/deploy.sh
```

它按锁文件安装依赖，构建优化后的 Web、平台 Server 和仓库 Codex，停止开发
Vite，后台启动 Server，并在健康检查通过后展示 Web/API 地址、运行模式、PID 和
日志位置。安装与编译详情写入固定的
`.local/open-web-codex/logs/deploy.log`，终端只展示阶段进度；失败时才显示日志
尾部。生产页面由 Server 同源托管在 `http://127.0.0.1:4800/web`。

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

1421/4800 双进程仅用于开发和 UI 联调：

用 Fake Runtime 验证 1421 WebApp 与 Server：

```bash
./scripts/start-all.sh --fake
```

用真实 Codex 启动：

```bash
./scripts/start-all.sh
```

脚本在 `4800` 启动平台 Server，并在 `http://127.0.0.1:1421/web`
启动 main 基线的独立 WebApp。WebApp 同源调用类型化 REST 和认证 WebSocket；
不启动 4732/4733 daemon，也没有独立 Gateway 进程。真实模式要求仓库 Codex
Binary 已构建；Fake 模式只用于 Server/WebApp 联调。
本地 Secret Store 主密钥首次运行时生成在
`.local/open-web-codex/master-key`，权限为仅当前用户可读；生产部署必须从外部
Secret Manager 注入 `OPEN_WEB_CODEX_MASTER_KEY`。

已有兼容 Binary 时可以显式指定：

```bash
CODEX_BIN=/absolute/path/to/codex ./scripts/start-all.sh
```

含密码的数据库 URL 推荐放在仅当前用户可读的文件中：

```bash
printf '%s\n' 'postgresql://user:password@host:5432/open_web_codex' > .local/database-url
chmod 600 .local/database-url
DATABASE_URL="$(<.local/database-url)" ./scripts/start-all.sh
```

后台管理：

```bash
./scripts/start-all.sh
./scripts/run-local.sh --status
./scripts/start-all.sh --stop
```

确认已有构建输出为最新时可使用 `--no-build`。日志位于
`.local/open-web-codex/logs/server.log`。

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

1. 打开 `http://127.0.0.1:1421/web`。
2. 首次运行选择初始化，创建首位 Owner；以后使用登录入口。
3. 创建 Git Project，平台只接受受控 Git URL，不接受浏览器本地路径。
4. 创建 Task 和 Run。Runner 创建私有 mirror 与该 Run 独占的可写 workspace。
5. 向运行中的 Task 发送消息，事件先持久化再通过 WebSocket 投影。
6. 处理待审批请求；浏览器不会看到 app-server request ID 或服务器路径。
7. 在 Changes 中选择文件并显式 Commit。

## 配置

| 变量 | 作用 |
| --- | --- |
| `DATABASE_URL` | PostgreSQL 连接 |
| `DATABASE_MAX_CONNECTIONS` | 连接池大小，默认 10 |
| `CODEX_MODE` | `real` 或 `fake` |
| `CODEX_BIN` | Codex Binary |
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
- 已支持隔离 workspace、状态、选择性 Commit；Push 与高级 Diff 尚未作为浏览器
  资源开放。
- Session 当前使用 Bearer token；生产发布前仍需完成 HttpOnly Cookie、CSRF、
  限速、备份恢复和 Runner 强隔离门禁。
