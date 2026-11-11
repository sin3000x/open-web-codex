# Architecture Decision Records

ADR 记录已经接受、会长期约束实现且不能仅从当前代码推断原因的决定。

目标架构报告中的方案比较、开发计划中的任务和 Git 提交不会自动成为 ADR。只有当
一个选择进入实施并需要未来维护者继续遵守时，才创建或更新 ADR。

## 当前记录

| ADR | 决定 |
| --- | --- |
| [ADR-001](001-web-server-framework.md) | 独立 Web Platform Server |
| [ADR-002](002-database.md) | PostgreSQL 与 Migration |
| [ADR-003](003-workspace-structure.md) | 平台 Cargo Workspace 结构 |
| [ADR-004](004-codex-adapter-pattern.md) | Codex Adapter 抽象 |
| [ADR-005](005-map-reply-cards.md) | Inline Visualization Artifact 与回复引用 |
| [ADR-006](006-supervisor-capability-packages.md) | Supervisor Capability Package；旧模板/发布合同被 ADR-014/017 替代 |
| [ADR-007](007-workspace-dataset-releases.md) | Workspace Dataset Release；已被 ADR-018 替代 |
| [ADR-008](008-installable-tutorial-blueprints.md) | 版本化 Tutorial Blueprint；旧安装合同被 ADR-014/017 替代 |
| [ADR-009](009-thread-first-data-intake.md) | Thread-first Data Intake；已被 ADR-018 替代 |
| [ADR-010](010-workspace-wide-intake-discovery.md) | 已被 ADR-018 替代 |
| [ADR-011](011-explicit-demo-workspace-sources.md) | 显式 Demo 与 Blueprint 分离；旧 Workspace 发现链被 ADR-013/017 替代 |
| [ADR-012](012-explicit-development-database-rebuild.md) | 开发数据库只允许显式重建 |
| [ADR-013](013-platform-data-intake-and-work-state.md) | Platform Data Intake 与 Work State；已被 ADR-018 替代 |
| [ADR-014](014-release-installation-runtime-discovery.md) | Release/Installation；阶段一部分已被 ADR-018 替代 |
| [ADR-015](015-bounded-platform-tool-result.md) | Platform Tool Result；已被 ADR-018 替代 |
| [ADR-016](016-collaboration-context-and-root-read-model.md) | CollaborationContext/Root read model；已被 ADR-018 替代 |
| [ADR-017](017-clean-copilot-platform-spine.md) | 以干净 Copilot 主干替换仓网原型边界；已被 ADR-018 替代 |
| [ADR-018](018-built-in-network-copilot-runtime-closure.md) | Codex 原生机制驱动的内置仓网 Copilot；当前阶段基线 |
| [ADR-019](019-task-selected-copilot-packages-and-shared-tools.md) | Task 显式选择独立 Copilot 包，Tool 使用根级共享注册表；局部替代 ADR-018 的单包/default 假设 |
| [ADR-020](020-platform-native-html-inline-visualizations.md) | Platform 快照授权 Workspace HTML 为原生 Thread inline visualization |
| [ADR-021](021-deferred-platform-tool-discovery.md) | Role 以 MCP server 为权限边界，Tool 通过 Runtime deferred discovery 向 Copilot 开放 |
| [ADR-022](022-native-copilot-skill-catalog.md) | Copilot Root 常驻小 Skill，任务 Skill 经 Runtime Catalog 渐进发现与读取 |
| [ADR-023](023-workspace-prepared-inputs-and-domain-geospatial-execution.md) | Workspace 准备输入、可验证计算血缘与领域地理执行；局部替代 ADR-018 的 Data Resource 交接与手工导航导入 |
| [ADR-024](024-warehouse-copilot-contract-simplification.md) | 仓网双 Copilot 的最小运行合同、精确路线范围与两阶段最小仓数求解；局部替代 ADR-023 的检查 Resource 与 pair 复用条件 |
| [ADR-025](025-source-unit-prepared-v2-reuse.md) | Source unit 选择、Prepared v2 provenance 与 fresh reuse；局部替代 ADR-023/024 的 Data 检查与准备输入复用段落 |
| [ADR-026](026-single-agent-warehouse-copilot-and-business-question-tools.md) | 仓网只保留单 Agent Copilot，Planner 以业务问题为粒度公开分析 Tool；局部替代 ADR-019 的双仓网包与 ADR-024 的逐步串联合同 |

## 何时创建 ADR

以下情况通常需要 ADR：

- 改变权威事实所有者或跨层合同；
- 引入难以撤销的数据库、协议或运行时边界；
- 保留新的 Codex 本地 seam；
- 在多个合理方案中选择一个并接受长期代价；
- 改变安全模型、兼容策略或恢复语义。

普通实现细节、可逆重构、当前任务清单和测试结果不需要 ADR。

## 最小结构

每篇 ADR 至少包含：

1. 状态与日期；
2. 背景和需要解决的问题；
3. 最终决定；
4. 被否决的主要替代方案；
5. 后果、风险和验证方式；
6. 被替代时指向新的 ADR。

ADR 描述当时为什么作出决定，可以保留必要历史背景；当前实现状态仍以
[系统架构](../architecture.md) 和 [能力基线](../capability-baseline.md) 为准。
