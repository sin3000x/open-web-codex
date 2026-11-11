# ADR-019：Task 显式选择独立 Copilot 包，Tool 使用根级共享注册表

状态：Accepted
日期：2026-08-18
补充并局部替代：[ADR-018](018-built-in-network-copilot-runtime-closure.md) 中“单一内置仓网组合、启动参数逐包注册和默认包”的部分；ADR-018 的 Runtime、Workspace、MCP Resource、Artifact 与协作所有权边界继续有效。

## 背景

此前本地启动脚本显式注册仓网和会议包，并把仓网包设为默认。创建 Thread 后出现仓网 Supervisor，原因是该应用启动配置和 Root Skill 注入，不是提示词命中了仓网语义。把“单 Agent/多 Agent”做成同一 Copilot 的两个入口会继续让一个包拥有两种互斥 Root 拓扑，也会使 Task 无法稳定表达自己选择的是哪个 Copilot。

仓网 Planner/Maps Tool 原先又位于仓网 Copilot 源码树内。让另一个 Copilot 复用它们时，如果通过 Copilot 路径引用，就会形成包间依赖，并容易被误解为跨 Copilot 通道。

## 决定

### 1. 一个 Copilot 包只有一个 Root

`copilot.toml` 的 `[root]` 必须声明一个 Skill，并可选择一个 Root Agent 配置。包不提供“entrypoint/mode”列表。一个 Task 只持久化一个稳定的 `copilot_package_id`，该选择在 Task 生命周期内不变；Run、Thread 启动和后续 Turn 都从该 Task 读取同一个 ID。

当应用发现至少一个可用 Copilot 时，新建 Thread 必须由用户从列表中显式选择；不能按提示词、显示名、目录名或默认包自动选择。只有部署没有发现任何 Copilot 时，普通无 Copilot Task 才允许省略该字段。

### 2. 发现范围是显式可信应用根，不是 Workspace 扫描

Server 只接收部署者配置的 `--copilots-root` 和 `--copilot-prepared-root`。它只枚举 Copilot 根目录的一级子目录，并只把含严格 `copilot.toml` 的目录作为包候选；包 ID 来自 manifest，prepared descriptor 按该 ID 从服务端根解析。Browser 只收到有界 package ID、display name 和可用状态，不能提交目录或 descriptor 路径。

这不是提示词分类、cwd/Workspace/source-repository 猜测，也不是 Catalog 或 Marketplace。无效 manifest、重复 ID、缺失 descriptor 或不匹配 revision 返回 typed unavailable/failure；不回退到某个仓网包。

### 3. 单 Agent 和多 Agent 是两个独立 Copilot

> 本节的“两个仓网 Copilot 并存”已被 [ADR-026](026-single-agent-warehouse-copilot-and-business-question-tools.md) 替代：仓网只保留单 Agent 包。“一个包只有一种 Root 拓扑”的原则继续有效。

- `copilots/warehouse-network` 是多 Agent 仓网 Copilot：Root Supervisor 只协调原生 Data/Network child Role，Root 不持有仓网 MCP。
- `copilots/warehouse-network-single-agent` 是单 Agent 仓网 Copilot：一个 Root Agent 直接拥有 Data、Network、Maps MCP policy，并显式关闭 `multi_agent`，不安装或调用 child Role。

两者拥有不同 package ID、Root Skill 和源码目录，但提供相同仓网规划、地图与报告交付合同。单 Agent 的 Root Agent TOML 只被投影为该 Thread 的官方 config overrides，不安装成 Profile child Role。

### 4. Tool 是根级共享包，不是 Copilot 通道

共享仓网 Tool 包位于仓库根级 `tools/warehouse-network-planner` 和 `tools/warehouse-network-maps`。每个 Tool 包用严格 `tool.toml` 声明 package ID 和 `runtime.toml`。Copilot manifest 只用 `{id, package}` 声明依赖；SDK 只能通过调用者显式给出的根级 Tool registry 解析一级 Tool 包。

Copilot 之间没有引用、消息、上下文、结果或采用协议。两个 Copilot 使用同一 Tool 实现，只代表依赖复用；MCP provider 按现有 Profile+Workspace 授权提供 Resource，Platform 不把它改造成 Copilot-to-Copilot 数据通道。

### 5. Profile 与 Runtime 组合

同一 Profile 的 active 包在 app-server 冷启动前一次性收敛。Skill ID 和可安装 child Role ID 必须全局唯一；冲突使组合明确失败。每个包产生一个以 package ID 为键的 Root execution config。Thread 创建只把已授权 Task 的 package ID 映射到对应 server-owned config；没有多包默认值。

安装表以 `(profile_id, package_id)` 为主键保存 desired/configured/failure 和精确 managed destination；`ready` 仍只来自当前 Runtime observation。Task 不保存 Skill、Role、Tool 路径或运行配置。

## 验证

- SDK 单元测试覆盖共享 Tool 必须通过显式 registry 解析，缺 registry 明确失败；所有 checked-in Copilot 都通过同一 `validate --tool-registry-root tools`。
- run-local 合同测试证明启动脚本遍历 Copilot 根、遍历使用 manifest ID、使用根级 Tool registry，且不含具体 Copilot source/default 参数。
- Rust contract、migration、冷启动 composition 与 Adapter 测试覆盖多安装记录、package-keyed Root config、Root Agent config flatten 和不把 Root Agent 安装成 child Role。
- Web typecheck/组件测试覆盖列表展示、显式单选和只提交 package ID。

## 后果

- 新增 Copilot 只需新增一个合规一级包目录并完成 Tool/descriptor 准备；Web 不增加硬编码入口。
- “单/多 Agent”不再是平台级模式。Agent 拓扑由所选 Copilot 包的 Root 合同拥有。
- 共享 Tool 可被多个独立 Copilot 复用，但 Tool ID、server ID、Skill/Role destination 和 delivery producer 冲突仍在组合阶段显式拒绝。
- 本决定不引入公开 Catalog、动态用户安装、Workspace capability 扫描、跨 Copilot 通信或第二 Agent 控制面。
