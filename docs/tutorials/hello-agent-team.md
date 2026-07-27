# 第二篇：把仓网问题拆成数据与规划两项职责

## 场景

华东区运营负责人问：

> 我们目前的一日达覆盖怎么样？在讨论新仓之前，请先确认数据可靠，并告诉我现有
> 两个仓在不新增设施时能做到什么程度。

这句话里其实有两个问题：

1. 订单数据是否完整，需求和历史履约怎样分布；
2. 在路线、容量和一日达口径下，当前仓网能覆盖多少需求。

第一项做错，要回到数据源和统计口径；第二项做错，要回到容量、路线和算法。把它们
拆开是为了让证据可追踪，而不是为了凑出两个 Agent。

本篇先在同一个任务（Codex Thread）中跑通两项职责，再解释 Supervisor 和真实子
Thread 应该在哪里出现。

开始前请完成[第一篇](hello-agent-quickstart.md)。返回
[教程总入口](../multi-agent-development-tutorial.md)。

## 本篇交付物

完成后，你应得到一页基线摘要：

```text
数据范围：6 行订单，100 个规划需求单位
数据质量：结构有效；10 个需求单位缺少实际履约时长；30% 与促销相关
历史履约：有观测的 90 个需求单位中，55 个达到一日达
当前关系覆盖率：75 / 100 = 75%
现有仓优化覆盖率：70 / 100 = 70%
解释：当前关系存在超容量；严格满足容量后，现有总容量只能承接 70 个单位
```

历史履约和规划覆盖率不是同一个指标。前者来自已经发生的订单，后者来自仓库、路线、
容量和服务政策模型。教程会要求 Agent 把标签写清楚。

## 开始前

本篇沿用第一篇的真实 Codex 平台。开始前确认：

- 平台运行在真实模式，Provider 和模型可用；
- 已选择授权工作区；
- 你已经理解 Tool、Skill、MCP Server 和任务 / Thread 的区别。

## 第一步：准备环境并认识演示数据

准备仓网规划环境：

```bash
tools/supply-chain-network-planner/bin/setup-env
```

它会创建独立 Python 环境并安装仓网示例、MCP 和维护测试所需的依赖。第一次体验
场景不要求先运行测试；先理解数据和目标，遇到连接问题或准备修改规则时再使用本篇
末尾的检查命令。

演示订单位于
[`warehouse-network-fixture.json`](../../tools/supply-chain-network-planner/examples/data-sources/warehouse-network-fixture.json)。
它只有六行，可以先人工核对：

| 城市 | 需求单位 | 已知实际时效 | 达到一日达 |
| --- | ---: | ---: | ---: |
| 上海 | 40 | 40 | 40 |
| 苏州 | 35 | 35 | 15 |
| 杭州 | 25 | 15 | 0 |
| 合计 | 100 | 90 | 55 |

杭州还有 10 个需求单位缺少实际时效；上海和苏州合计有 30 个促销相关需求单位。这些
都不一定让数据失效，但可能影响决策，因此必须进入质量提示。

## 第二步：让“数据准备”先回答事实问题

在真实 Codex 模式下新建任务。展开侧栏 **MCP Servers**，确认
`supply_chain_data` 和 `supply_chain_planner` 都是 `ready`，然后发送：

```text
我们要评估华东仓网。请使用 $prepare-planning-dataset，
读取 source_id `warehouse-network-fixture`。

先检查数据范围，再构建并验证 planning-dataset.v1。
告诉我总需求、城市分布、历史一日达表现、缺失数据和促销影响。
不要开始选址，也不要把原始订单逐行复制到回答中。
```

调用轨迹中应依次出现：

```text
supply_chain_data.inspect_planning_source
  → supply_chain_data.build_planning_dataset
  → supply_chain_data.validate_planning_dataset
```

为什么不是一个 `analyze_everything` Tool？

- `inspect` 先判断来源、范围和规模是否适合任务；
- `build` 才进行确定性聚合；
- `validate` 在交给下一项职责前对账。

如果失败，你能知道问题发生在“数据选错了”“聚合失败了”还是“交接物不一致”，而
不是只得到一句笼统的分析错误。

### 第一次遇到 Resource

`build_planning_dataset` 不会把完整数据塞进消息，而是返回类似：

```text
supply-chain-data://resources/<opaque-id>
```

这是一个 **MCP Resource**：

> MCP Server 保存的一份可读取内容，Agent 通过 URI 引用它，而不是在对话中反复
> 复制整份数据。

`<opaque-id>` 表示由 Server 生成的内部标识；Agent 只应原样使用，不需要解析或自己
编造。

这份 Resource 中包含 `planning-dataset.v1` 和下游需要的 `network_input`。数据准备
职责交付的是“结构化结果 + 引用 + 质量说明”，不是一句无法核对的“数据没问题”。

## 第三步：把同一版数据交给“网络规划”

继续在同一个任务中发送：

```text
现在只评估现有上海仓和南京仓。

请读取刚才已经验证的 planning-dataset.v1，使用其中未改动的 network_input。
从 `tools/supply-chain-network-planner/examples/route-matrix-input.json`
只取两个现有仓到三个需求点的 6 条演示路线。

使用 $prepare-network-baseline 和 $evaluate-network-scenario：
1. 创建并验证网络快照；
2. 注册并验证完整路线矩阵；
3. 计算当前分配关系下的覆盖率；
4. 计算不新增仓、允许重新分配后的覆盖率；
5. 验证所有用于结论的结果。

分别标注两个覆盖率，不要把历史履约率混进来。
```

这里的 **交接合同** 是 `planning-dataset.v1`：

> 交接合同是一方承诺提供、另一方可以校验和消费的稳定结构。它比“请继续分析”
> 这样的自然语言转述更不容易丢字段或改口径。

规划过程还会创建：

- `network_snapshot.v1`：本次计算使用的需求、设施、费率和服务政策；
- `route_matrix.v1`：每个仓到每个需求点的距离与时长；
- `current_coverage_result.v1`：当前关系和现有仓优化结果。

这些名字不需要背。只要理解它们把“用了什么输入”和“得到了什么结果”固定下来，
后续方案才能在同一口径上比较。

## 第四步：读懂 75% 与 70%

预期结果是：

| 指标 | 结果 | 回答的问题 |
| --- | ---: | --- |
| 当前关系覆盖率 | 75% | 保持订单记录的当前仓库关系，时效能覆盖多少需求 |
| 现有仓优化覆盖率 | 70% | 不新增仓、严格满足现有容量时，最多能服务多少需求 |

为什么重新优化后反而从 75% 降到 70%？

当前所有 100 个需求单位都指向上海仓，但上海仓容量只有 40。当前关系计算会把这个
超容量事实作为问题报告，同时仍告诉你这些关系中的时效覆盖；优化计算则严格遵守：

```text
上海容量 40 + 南京容量 30 = 70
```

所以最多只能分配 70 个需求单位。Agent 如果只说“优化使覆盖率下降 5 个百分点”，
却不解释容量口径，就没有完成任务。

## 第五步：现在才引入 Supervisor

到目前为止，仍然只有一个 Thread。这个 Thread 中的 Agent 先后使用了两组能力：

```text
当前 Thread
├── 数据准备步骤
└── 网络规划步骤
```

两个 Skill 或两个 MCP Server 并不会自动变成两个 Agent。MCP Server 只负责执行
Tool，没有自己的任务上下文。

当任务足够复杂，需要把两项工作交给独立上下文时，当前 Thread 中负责总目标的 Agent
可以成为 **Supervisor**：

> Supervisor 是负责拆分总目标、委派子任务、检查交接物、处理失败并汇总结果的上层
> Agent。它是一种运行职责，不是第三个业务 Tool，也不是必须新建的服务。

Supervisor 可以创建两个 **子 Thread**：

```text
Supervisor 所在的根 Thread
├── Data Agent 子 Thread
└── Network Planning Agent 子 Thread
```

子 Thread 是一段独立的 Codex 工作上下文。它应有自己的 Thread ID、消息、Tool
调用和完成或失败状态。Data Agent 返回经过验证的 Dataset 引用；Network Planning
Agent 消费同一引用并返回计算结果；Supervisor 不应靠复制粘贴重新解释数据。

## 什么时候值得拆成两个 Agent

本例适合拆分，是因为：

- 两项职责需要不同的输入权限；
- 数据准备结果会被多个方案复用；
- 规划者不应静默修改来源数据；
- 两边可以独立失败和重试；
- 交接物有明确 Schema，可以自动检查。

下面这些理由不充分：

- 有两个 Python 文件；
- 有两个 Skill；
- 想让演示看起来像多 Agent；
- 同一个很短的步骤被机械地分成“生成”和“审核”。

如果任务在一个 Thread 中就能清楚、安全、快速完成，多建 Agent 只会增加上下文、
等待和故障面。

## 给 Supervisor 的最小委派合同

真正委派时，不要只说“分析一下数据”。两项任务至少应包含：

```text
Data Agent
目标：为华东仓网规划准备可信需求基线
输入：source_id、规划周期、一日达口径
输出：validated planning-dataset.v1 Resource 引用、摘要、警告
停止条件：来源不匹配、总量无法对账、验证失败

Network Planning Agent
目标：在现有仓范围内计算当前与优化覆盖率
输入：同一 Dataset 引用、路线来源、容量和服务政策
输出：validated current_coverage_result.v1、指标解释、问题
停止条件：路线不完整、口径不一致、结果验证失败
```

Supervisor 的工作是检查这些条件有没有满足，而不是替两边重算一遍。

## 可选：排错或修改前检查计算底座

如果 MCP Server 无法进入 `ready`，或者你准备修改仓网规则，可以先运行：

```bash
.local/open-web-codex/tool-envs/supply-chain-network-planner/bin/python \
  -m pytest tools/supply-chain-network-planner/tests -q

.local/open-web-codex/tool-envs/supply-chain-network-planner/bin/python \
  tools/supply-chain-network-planner/tests/stdio_smoke.py
```

两条命令回答不同问题：

- `pytest` 检查普通 Python 中的数据、覆盖率和成本规则；
- `stdio_smoke.py` 作为最小 MCP Client，检查两个 Server 能否启动、列出和调用
  Tool。

它们由示例作者预先提供，不是开始学习前要求你先创建的文件。

## 为什么本篇停在一个任务

你在本篇真实验证了：

- 两个 MCP Server 的真实启动和 Tool 合同；
- 一个新任务对数据准备与规划能力的发现；
- 同一任务中从数据到覆盖率结果的完整链。

本篇没有要求在 Web 中创建两个子任务，所以你完成的是“职责和交接设计”，不是一条
已经运行的双 Agent 轨迹。这不影响刚才的 Tool、Skill 和交接合同继续用于真正的
Data Agent 与 Network Planning Agent。

## 完成标志

- [ ] 能从六行订单人工算出 100、90、55 三个数字；
- [ ] Data Tool 按 inspect → build → validate 的顺序运行；
- [ ] 规划使用同一版 Dataset，而不是重新编造需求；
- [ ] 能解释 75% 和 70% 为什么不矛盾；
- [ ] 能解释 MCP Server、Agent 和 Thread 的区别；
- [ ] 能用自己的话说明 Supervisor 做什么；
- [ ] 能判断当前练习是单 Thread 能力链，还不是真实双 Agent 轨迹。

下一篇把杭州候选仓加入同一份决策：

[第三篇：从业务问题到可复核的仓网建议](supply-chain-agent-tutorial.md)
