# Copilot SDK

> **适合谁**：维护 SDK 本身，或需要快速查仓库内 Copilot 开发入口的工程师
> **预计时间**：3 分钟
> **前置条件**：位于仓库根目录，Python 3.11+
> **完成结果**：使用唯一包装器创建并验证包，不绕过仓库 owner

## 定位

Copilot SDK 负责源码包的 `init`、静态合同验证、Tool 环境准备、真实 app-server discovery 和本地
确定性原生测试。它不拥有生产 Profile 安装、真实模型质量、Web 编辑器、Marketplace 或业务算法。

## 一条命令开始

```bash
./scripts/copilot.sh --help
```

包装器会准备隔离 SDK 环境，并固定当前仓库的共享 Tool registry、Runtime 与 build store；不要先
`pip install -e`，不要设置 `PYTHONPATH`，也不要改用裸 `copilot` 命令。

## 常用命令

```bash
# 生成业务中性的单 Agent 包
./scripts/copilot.sh init ./copilots/record-review --name record-review

# 生成一个尚未接线的根级共享 Tool
./scripts/copilot.sh tool init ./tools/record-review --name record-review

# 静态验证
./scripts/copilot.sh validate ./copilots/record-review

# validate → prepare → dev → test
./scripts/copilot.sh check ./copilots/record-review --workspace "$PWD"

# check 通过后冷重启本地服务
./scripts/copilot.sh sync ./copilots/record-review --workspace "$PWD"
```

完整参数见 [CLI 参考](../../docs/developers/copilot/reference/cli.md)。

## 边界

- 本地 Tool 在 manifest 中使用精确 `root` + `runtime`；共享 Tool 必须用根级 registry 的
  `package`，不能写跨目录路径。
- Tool 的唯一运行声明是 `runtime.toml`；依赖在 Turn 之前准备，Runtime 启动时不安装。
- `dev` 只证明 Skill/MCP discovery；`test` 只证明本地确定性正常链；`check` 不是生产模型或 Web
  质量证明。
- `sync` 只接受 `copilots/` 下的直接子目录，并且先跑全部 case；失败包不会触发重启。
- Browser 不读取内部 Tool 环境信息；Secrets、绝对路径和原始 Runtime payload 不进入输出。

## 文档入口

- [Copilot 开发者中心](../../docs/developers/copilot/README.md)
- [30 分钟做出第一个 Copilot](../../docs/developers/copilot/first-copilot.md)
- [开发仓网 Copilot](../../docs/developers/copilot/warehouse-copilot.md)
- [`copilot.toml` 参考](../../docs/developers/copilot/reference/copilot-manifest.md)
- [`runtime.toml` 参考](../../docs/developers/copilot/reference/tool-runtime.md)

成功信号：`./scripts/copilot.sh check ...` 四阶段全部 passed。失败按
[阶段排错](../../docs/developers/copilot/testing-and-troubleshooting.md#按阶段排错)，不要添加兼容、
重试或假 ready。

### MCP tool exposure policy

Role MCP policies may omit `omit_tools_from`. This leaves direct versus deferred
exposure to Codex's typed model/provider capabilities; function-tool models do
not need ToolSearch merely to access authorized business tools. An explicit
`omit_tools_from = ["direct"]` requires a usable discovery/code-mode surface and
is appropriate only when the deployment supplies it. The SDK preserves that
explicit policy; it does not infer capabilities from model names. Warehouse
reference roles omit the restriction. Approval policy and server scope are
independent and remain unchanged.
