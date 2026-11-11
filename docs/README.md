# open-web-codex 文档导航

这组文档共同回答五类不同问题：产品最终去哪里、目标架构为什么这样设计、当前
代码真实是什么、下一步按什么顺序建设，以及系统如何运行和验证。

它们不是同一份“大而全”文档的拆页。每篇文档只拥有一种主要时间视角和一种权威
责任，避免长期目标、当前事实与计划状态互相覆盖。

## 文档分层

| 层级 | 权威文档 | 回答的问题 | 不负责 |
| --- | --- | --- | --- |
| 产品北极星 | [产品愿景](product-vision.md) | 最终服务谁、解决什么问题、形成什么产品 | V1 功能清单、当前代码状态 |
| 产品要求 | [产品设计](product-design.md) | 当前 V1 的用户流程、功能、状态和验收标准 | 证明能力已经实现 |
| 目标架构推演 | [企业多 Agent 平台架构](enterprise-agent-platform-architecture.md) | 为什么需要这些能力，方案如何比较并收敛 | 充当当前代码说明或任务清单 |
| 多 Agent 专项演进 | [协同演进](multi-agent-collaboration-evolution.md) 与 [信息交换演进](agent-information-exchange-evolution.md) | 两条核心能力怎样分阶段演进、当前处于哪里、每阶段怎样验收 | 改写全项目路线图或提前声明条件能力 |
| 阶段二研究输入 | [Supervisor、Agent、Skill 与 Tool](supervisor-agent-skill-tool-architecture.md) | 公开 Copilot 创作平台曾评估过哪些对象与边界 | 充当阶段一架构或实施合同 |
| 当前架构 | [系统架构](architecture.md) | 当前系统组成、事实所有者和运行边界 | 保存长期设想和实现历史 |
| 安全边界 | [安全模型](security-model.md) | 资产、信任边界、授权链和不可削弱的安全属性 | 宣称某项门禁已经通过 |
| 能力证据 | [能力基线](capability-baseline.md) | 当前构建中什么已实现、验证到什么程度、还缺什么 | 描述产品愿景或未来任务 |
| 中期顺序 | [产品与工程路线图](roadmap.md) | 各阶段交付什么结果、进入和退出条件是什么 | 保存逐文件任务和完成历史 |
| 近期执行 | [开发计划](development-plan.md) | 当前与下一个里程碑的任务、阻塞和验收 | 重复全部长期路线图 |
| 阶段二候选输入 | [Copilot 开发平台实施计划](agent-capability-lifecycle-plan.md) | 公开 Tool、Skill、Agent、Supervisor、Copilot 平台怎样按 owner、合同、删除门和验收门实施 | 充当阶段一当前计划或宣称能力已经成立 |
| 运行操作 | [本地运行手册](mvp-runbook.md) | 如何启动、配置、验证和排错 | 定义产品或架构 |
| 开发约束 | [项目 Agent Guide](../AGENTS.md) | 修改代码时必须遵守的所有权、流程和验证规则 | 代替面向读者的产品文档 |

## 专项文档

### Codex 上游与本地修订

- [Codex 上游同步手册](codex-upstream-sync.md)：如何同步官方
  `openai/codex`。
- [本地 Codex Patch Map](custom-codex-patch-map.md)：必须保留的本地 seam、原因、
  影响文件、验证方式和退出条件。
- Git 历史记录每次具体修改；Patch Map 不承担提交日志职责。

### 协议与实现规格

- [Chat/Responses 转换规格](chat-responses-translation-spec.md)
- [Chat/Responses 转换计划](chat-responses-translation-plan.md)
- [app-server 事件适配](app-server-event-adaptation.md)

这类文档服务一个明确合同。合同成为当前实现后，应同步更新系统架构和能力基线；
完成过程不继续堆入规格正文。

### 扩展指南、仓网教程与开发者入口

- [Copilot 开发者中心](developers/copilot/README.md)：仓库内部第一次开发 Copilot 的唯一入口；
  统一使用 `./scripts/copilot.sh`，先完成业务中性的单 Agent，再进入真实仓网。
  - [30 分钟做出第一个 Copilot](developers/copilot/first-copilot.md)
  - [开发仓网 Copilot](developers/copilot/warehouse-copilot.md)
  - [测试与排错](developers/copilot/testing-and-troubleshooting.md)
- [领域 Agent 扩展架构](domain-agent-extension-architecture.md)
- [Skills、MCP 与自定义 UI 扩展](custom-skills-mcp-ui-guide.md)
- [仓网 Copilot 新手教程](multi-agent-development-tutorial.md)：从上传数据开始，完成
  12 小时时效、指定新增仓和最少新增仓三类业务分析；教程使用当前的 Workspace、Task、
  单 Agent 仓网 Copilot 包和地图交付合同，不要求使用者理解内部工具名称。
  - [10 分钟跑通完整示例](tutorials/indonesia-network-quickstart.md)
  - [认识要上传的数据](tutorials/indonesia-network-01-data.md)
  - [查看 12 小时时效和地图](tutorials/indonesia-network-02-service-baseline.md)
  - [评估新增 Balikpapan 仓](tutorials/indonesia-network-03-two-level-cost.md)
  - [找出达到 90% 的最少新增仓](tutorials/indonesia-network-04-optimization-map.md)
  - [审批与故障恢复](tutorials/approvals-and-recovery.md)
- [Hello Agent：第一次真实 Tool 调用](tutorials/hello-agent-quickstart.md)：开发者练习，不是
  仓网使用前置条件；它演示原生 Plugin 路线，不是 Copilot SDK 包。

扩展指南和教程不能重新定义 Profile、Workspace、Artifact、Agent Definition、Runtime Role
等核心术语。仓网教程只描述当前已验证的使用流程；历史原型和已移除的发布流程只作为明确
标注的归档记录或 Git 历史保留，不作为新手操作依据。

### 当前纵向切片

- [Enterprise Supervisor Copilot 冻结迁移输入](enterprise-supervisor-copilot-plan.md)：
  保存仓网 6.0 原型的角色、算法和旧 Case/E2E 输入；已冻结，不再作为当前合同或新增
  补丁入口，迁移完成后删除。
- [Copilot 开发平台实施计划](agent-capability-lifecycle-plan.md)：
  公开 SDK、Studio、Catalog 和 Marketplace 的阶段二候选实施输入；当前不据此推进
  阶段一实现。

### 多 Agent 历史研究输入

- [多 Agent 协同：从单一执行到可治理协作](multi-agent-collaboration-evolution.md)：
  保存协同成熟度推演。当前阶段只采用其中与 Codex 原生 Root/child、wait/mailbox、
  steer 和终态一致的原则，不采用 Work State、Assignment 或 Platform 调度方案。
- [Agent 信息交换：从消息传递到可追溯的协作事实](agent-information-exchange-evolution.md)：
  保存旧 typed resource/Artifact/Ledger 方案比较。当前合同已经收敛为 Runtime 消息/上下文、
  provider-owned MCP Resource、普通 Workspace 文件和仅用于最终交付的 Artifact，不采用 Platform
  Resource Broker/Ledger。
- [Supervisor、Agent、Skill 与 Tool 的分层架构](supervisor-agent-skill-tool-architecture.md)：
  保存公开 SDK/Studio/Catalog 的阶段二研究输入；其中 Work State、Data Intake、
  Resource Broker、Run Completion 和 Root-only 输入设计已被 ADR-018 否决。

这三篇不再拥有当前阶段标记、实施顺序或验收门。阶段一只以 ADR-018、ADR-019 和开发计划为准；
“是否已经实现”只以能力基线和代码验证为准。

### 架构决策

[ADR 索引](adr/README.md) 保存已经接受且会长期约束实现的决策。目标架构报告中的
方案比较不自动成为 ADR；当某项选择进入代码实施时，再创建简短 ADR，引用报告中的
论据并记录最终决定、后果与替代方案。

ADR-008 与 ADR-011 保存旧教程/仓网原型的历史决定，其具体 Workspace 安装、扫描和
发现路径已被后续 ADR 替代。ADR-017 的 Clean Spine A/B 已被替代；当前阶段一的接受
基线是 [ADR-018：Codex 原生机制驱动的内置仓网 Copilot](adr/018-built-in-network-copilot-runtime-closure.md)
与局部替代单包/default 假设的
[ADR-019：Task 显式选择独立 Copilot 包，Tool 使用根级共享注册表](adr/019-task-selected-copilot-packages-and-shared-tools.md)。
仓网当前最小 Tool/Skill 数据握手、prepared v2 和 fresh reuse 由
[ADR-025：Source Unit 选择与 Prepared v2 新鲜复用](adr/025-source-unit-prepared-v2-reuse.md) 补充。

## 事实与冲突处理

不同问题使用不同权威来源：

1. “产品为什么存在、最终去哪里”以产品愿景为准。
2. “V1 要交付什么”以产品设计为准。
3. “当前代码由谁拥有、如何连接”以系统架构和代码为准。
4. “当前能力能否对外声称可用”以能力基线及其验证证据为准。
5. “接下来先做什么”以路线图和开发计划为准；当前阶段一以 ADR-018、ADR-019 和开发计划为准，
   公开 Copilot 平台的 owner、合同与删除门留待阶段二重新裁决。
6. “为什么接受某个长期技术决定”以 ADR 为准。

代码、生成合同和可复现测试是实现事实的最终证据。文档与代码冲突时，不能默默把
目标描述成现状：先确认事实，再更新对应权威文档并明确验证缺口。

## 更新规则

| 发生变化 | 必须复审 |
| --- | --- |
| 目标用户、产品边界或长期价值变化 | 产品愿景、产品设计、路线图 |
| 所有权、组件关系或持久化边界变化 | 系统架构、安全模型、相关 ADR |
| Runtime、Web 或平台能力新增或降级 | 能力基线、开发计划 |
| 阶段一工作切片、功能验收或可信风险变化 | ADR-018/019、开发计划；仓网冻结迁移输入变化另复审 Enterprise Supervisor 文档；形成证据后再更新能力基线 |
| 多 Agent 协同方式、Agent 责任或异常收敛规则变化 | 多 Agent 协同演进、能力基线、相关计划 |
| 阶段一上下文、Workspace 文件、MCP Resource、Artifact 或 Agent 交换边界变化 | ADR-018/019、开发计划、系统架构、能力基线；历史信息交换文档不拥有当前合同 |
| 阶段一 Supervisor、Agent、Skill、MCP、Copilot package 或共享 Tool 关系变化 | ADR-018/019、开发计划、系统架构；阶段二研究文档不拥有当前合同 |
| 阶段顺序或进入条件变化 | 路线图、开发计划 |
| 启动参数、部署拓扑或故障处理变化 | 运行手册 |
| `codex/` 非生成差异变化 | Patch Map、相关测试、必要 ADR |
| 协议或浏览器 DTO 变化 | 规格、生成物、系统架构、能力基线 |

Canonical 文档只描述当前有效的目标、事实、顺序或操作。已经被替代的过程说明留在
Git 历史；只有仍然约束未来实现的决定进入 ADR。
