# 审批与故障恢复：安全地继续或停止任务

本篇教你处理两类最容易误判的情况：

1. Runtime 正在等待用户决定，而不是“卡住”；
2. 某一层失败后，应恢复原任务还是创建新任务。

预计用时 15–20 分钟。本篇复用上一教程的任务，不需要新的模型调用，不要求关闭
安全策略，也不要求执行危险命令。

建议先完成[仓网 Copilot 新手教程](README.md)。返回
[仓网教程入口](../multi-agent-development-tutorial.md)。

## 完成后的能力

你应该能够：

- 判断一个动作为什么需要或不需要审批；
- 从任务主界面或 Agent Activity 处理子 Agent 审批；
- 区分 Accept、Deny、失败、取消和过期；
- 在刷新、断线或服务恢复后继续同一个 Thread；
- 根据错误所属层选择正确的恢复方式；
- 收集诊断信息时不泄露 Secret、路径或内部协议数据。

## 1. 审批不是“每一步确认”

审批的目标是让用户决定有实质影响的动作，不是为每次 Tool 调用增加按钮。

| 动作类型 | 默认处理原则 |
| --- | --- |
| 已评审、只读、有界、确定性 MCP Tool | 可以在 Plugin 合同中预批准 |
| 只创建不可变内部 Resource 的受控计算 | 可以在完整 Server 经过评审后预批准 |
| 读取授权 Artifact | 由平台授权，通常不需要逐次人工确认 |
| 文件修改 | 根据 Runtime、Workspace 和审批策略决定 |
| 命令执行 | 根据命令影响、沙箱和审批策略决定 |
| 凭据或 OAuth | 必须通过明确的凭据/授权流程 |
| 权限扩大、外部发布、发送消息、支付 | 必须显式决策 |
| 非幂等写入 | 不得因失败自动重复，除非合同和幂等键明确保证 |

预批准不是全局 `never ask`。它只适用于一个经过审查且完整 Tool 集均为低风险的 MCP
Server，并继续受 Role MCP server scope、capability root、数据根和平台授权约束。

混合风险 Server 不能因为其中几个 Tool 是只读，就把整个 Server 默认批准。

## 2. 怎样阅读审批卡

审批卡应告诉你：

- 发起动作的 Agent；
- 动作类型和安全摘要；
- 它准备影响什么；
- 可选择的 Accept 或 Deny；
- 提交后的处理状态。

受治理任务中，子 Agent 的未决审批会投影到：

1. 任务主界面的审批队列；
2. 右侧 **Agent activity** 中对应 Agent 的审批卡。

两处代表同一个持久审批，不是两次不同请求。点击一次后按钮会锁定，防止浏览器重复
提交。

不要在对话框里手动回复“批准”或“拒绝”。聊天文本不会代替平台审批决策。

地图工具缺少凭据时，URL 审批卡应显示 **配置 Key**，由安全配置窗口选择
Mapbox 或 Google Maps 并提交凭据；已有配置会通过服务端送达当前请求。
平台投影保留有界的 `credentialKind: "maps"` 分类标记，实时事件与历史恢复使用
同一标记。它不包含 Key；实际凭据和一次性本地提交 URL 不进入事件或对话。
若卡片显示不支持安全配置流程，应检查审批事件投影，不能通过聊天粘贴 Key 绕过。

## 3. Accept 前的四个问题

只有四个问题都能回答时才 Accept：

1. **谁发起？**

   是 Root、预期的子 Agent，还是不认识的角色？

2. **为什么需要？**

   当前任务目标是否真实依赖该动作？

3. **影响范围是什么？**

   文件、命令、数据源、外部系统和权限范围是否明确且有界？

4. **失败或重复会怎样？**

   动作是否幂等，是否可能重复发送、覆盖、发布或产生费用？

以下情况应 Deny：

- 目标或影响范围不清楚；
- Agent 请求模板没有声明的能力；
- 请求包含凭据、内部路径或用户未授权的数据；
- 一个只读分析任务突然要求外部发布或写入；
- 非幂等动作没有幂等保证；
- Agent 用“完成任务需要”为理由要求扩大权限，但没有合同依据。

Deny 是正常结果，不是系统故障。Agent 应保留已经完成的 Artifact，说明哪些结果仍然
有效、哪些步骤没有执行。

## 4. 正常等待与真正卡住

Agent Activity 可能显示：

| 状态 | 含义 | 用户应该做什么 |
| --- | --- | --- |
| Running | Agent 正在处理当前 Turn | 等待进度或查看 Tool 状态 |
| Waiting for Agent updates | Supervisor 进行有界等待 | 不要重复启动任务 |
| Waiting for approval | Runtime 等待用户决策 | 阅读并处理审批卡 |
| Completed | 当前 Agent 工作周期结束 | 查看返回结果和 Artifact |
| Failed | 当前工作周期失败 | 查看结构化错误和所属层 |
| Cancelled | 用户或系统明确取消 | 确认是否需要新 Turn 或新任务 |

Supervisor 的有界 `wait` 完成后可能再次等待。这是 Runtime 协作语义，不代表任务已经
停止。

判断“卡住”时，不要只看经过时间。检查：

- 是否存在未决审批；
- 是否仍有 Tool 或 Agent Turn 处于 Running；
- Runtime 是否持续报告状态；
- Provider 是否可用；
- Thread 是否仍能从服务端读取；
- Artifact 是否停留在 `materializing` 且有明确失败原因。

## 5. 刷新和断线后的恢复

浏览器是投影，不拥有 Thread。遇到页面断线或刷新：

1. 保留原 Thread；
2. 刷新页面；
3. 在左侧重新选择同一个 Thread；
4. 等待历史和状态恢复；
5. 打开 **Agent activity**；
6. 检查未决审批、Agent 状态和 Artifact；
7. 只有确认原任务已终态失败且不能继续时，才考虑新建任务。

不要因为 UI 短暂显示空白就再次提交相同目标。重复任务可能重复模型费用、Agent 工作
和外部副作用。

## 6. 常见错误应该从哪一层处理

### 创建 Thread 失败

页面可能显示：

```text
Run failed before its Codex Thread was ready
```

先检查：

- Provider 和模型是否可用；
- Profile Host 是否健康；
- Workspace 是否仍授权；
- 受治理任务引用的 Supervisor/Agent Release 是否存在；
- 所需 capability root 是否可发现；
- Runtime 前置条件是否满足。

修复根因后使用页面提供的重试动作。不要在启动路径加入猜测版本、伪造 Thread ID 或
自动切换实现。

### Codex Runtime 无法启动 Turn

先检查：

- 当前 Thread 是否已经正确 materialize；
- Provider/model 是否仍与该 Thread 的选择一致；
- Runtime 进程是否正在安全替换；
- 是否存在未决 Server Request；
- 错误是否来自能力缺失、Provider 或 Thread 状态。

不要用一个新 Thread 掩盖原 Thread 的持久状态问题。

### MCP Server 或 Tool 失败

先运行该能力包的：

1. 单元测试；
2. stdio smoke；
3. Skill/Plugin 校验。

如果底层失败，Agent 必须报告失败，不能转而用模型“模拟” Tool 结果。

### Agent 缺失或失败

正确行为：

- 缺少精确 Agent Release/Role 时启动前失败；
- 不使用 `default` Agent 代替；
- 不让 Root 接管专业 Tool 并声称分工完成；
- 已有 Artifact 保持可见；
- 报告未完成的交付合同。

### Artifact 长时间未就绪

检查：

- Tool completion 是否包含受支持的 Resource link；
- MCP Resource 是否能通过正式 read 读取；
- 内容类型、JSON、Schema 和大小是否有效；
- 物化任务是否记录失败；
- 页面是否只是投影未刷新。

不要把 `materializing` 手工改成 `ready`。

## 7. 允许的恢复与禁止的兜底

允许的可靠性行为：

- 有次数或时间上限的重试；
- 断线重连；
- 幂等事件重放；
- 持久 Thread、审批和 Artifact 投影恢复；
- 在明确幂等合同下重试同一操作；
- 保留原始错误原因的类型化状态转换。

禁止的兜底：

- 吞掉错误并显示成功；
- 根据错误文本猜协议版本；
- 用默认 Agent 替代缺失 Role；
- 用默认值掩盖缺失数据；
- MCP 不可用时让模型伪造 Tool 输出；
- 同时维护新旧 API 或双写字段；
- 能力缺失时隐藏切换另一套实现。

恢复的目标是继续权威状态，不是绕开权威状态。

## 8. 收集诊断信息

报告问题时记录：

- 可读的 Task/Thread 标题；
- 发生时间；
- Provider 和模型名称，不包含 Key；
- 绑定的 Supervisor/Agent 版本；
- Agent 可观察状态；
- 安全的错误类型和消息；
- 哪项测试或 smoke 失败；
- Artifact Schema 和状态，不包含内部 Resource URI。

不要复制：

- Provider Key；
- Cookie、Session Token 或 OAuth 凭据；
- app-server request ID；
- 宿主机绝对路径；
- 内部 MCP Resource URI；
- 未脱敏 Tool 参数或结果；
- 模型 reasoning。

## 9. 一个安全的练习

使用已完成的仓网任务：

1. 打开 **Agent activity**，确认三个 Agent 的终态；
2. 点击面板外部区域关闭，再重新打开；
3. 刷新页面并重新选择同一 Thread；
4. 确认 Agent、Artifact 和报告恢复；
5. 查看历史中曾经等待审批或等待 Agent 的状态；
6. 解释当时应该继续等待、Accept、Deny 还是诊断失败。

这个练习不制造新的危险操作，也能验证你理解平台状态与恢复路径。

## 完成检查表

- [ ] 能解释预批准和全局关闭审批的区别；
- [ ] 能从审批卡识别发起 Agent 和影响范围；
- [ ] 知道聊天文本不能代替 Accept/Deny；
- [ ] 知道 Deny 后应保留哪些部分结果；
- [ ] 能区分 Waiting、Running、Completed 和 Failed；
- [ ] 刷新后能恢复同一 Thread，而不是重复创建任务；
- [ ] 能把 Thread、Runtime、MCP、Agent 和 Artifact 错误定位到所属层；
- [ ] 能解释有界恢复与静默兜底的区别；
- [ ] 收集诊断信息时不泄露 Secret、路径或内部协议标识。

完成本篇后，返回[教程总入口](../multi-agent-development-tutorial.md)，根据自己的领域
问题决定是继续扩展能力包，还是先完善失败与恢复证据。
