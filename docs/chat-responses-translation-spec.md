# Chat Completions / Responses 转译规范

## 文档状态

| 字段 | 值 |
| --- | --- |
| 状态 | Proposed；实现尚未完成 |
| 适用范围 | `wire_api = "chat"` 的第三方 Provider |
| 规范所有者 | `codex/codex-rs/codex-api` |
| 下游消费者 | Codex Core、app-server、Platform event projection、WebApp/TUI |
| 实施计划 | `docs/chat-responses-translation-plan.md` |

本文定义 Chat Completions、Responses API 和 Codex 内部事件之间的规范化语义，
并规定 Assistant 内嵌可视化指令如何跨两种 wire 保持不变。它是 Provider Chat
transport 的长期转译合同；具体 Artifact 注册和浏览器渲染由
`docs/adr/005-map-reply-cards.md` 定义，不改变 Thread/Turn、工具执行、MCP 或
历史所有权。

当前实现仍用“响应中存在 Tool call”推断助手文本为 `commentary`。Chat
Completions 协议并没有提供该语义，因此这种推断会把用户可见正文误分类。本文
规定的目标合同尚未完全落地；当前能力事实以
`docs/capability-baseline.md` 为准。

## 规范关键词

本文中的“必须”“不得”“应该”“可以”分别表示：

- **必须/不得**：为了保持协议、历史和 UI 一致性必须满足的合同。
- **应该**：除非存在记录在案的 Provider 兼容性原因，否则必须采用。
- **可以**：由明确的 Provider 能力或产品策略选择。

## 核心语义

转译必须分别保存三个互不替代的维度：

| 维度 | 合法值 | 说明 |
| --- | --- | --- |
| Item kind | Message、Reasoning、FunctionCall、Tool output 等 | 内容是什么 |
| Message phase | `commentary`、`final_answer`、未指定 | 用户可见消息处于哪个回答阶段 |
| Lifecycle | added、delta、done、response completed/failed | Item 当前生命周期 |

以下关系必须成立：

1. `reasoning` 与 `commentary` 不是同义词。
2. `commentary` 是用户可见的中间说明，例如工具调用前的简短 preamble。
3. `reasoning` 是独立的推理 Item；不得由普通助手 `content` 推断产生。
4. `final_answer` 是明确标记的完成回答。
5. 未指定 phase 表示上游没有提供可靠分类，不表示 `commentary`，也不表示
   reasoning。
6. `finish_reason = "tool_calls"` 只说明本次 Chat 生成以工具调用结束，不说明
   `content` 的类型。
7. `reasoning_tokens` 只表示 Token 计数，不能恢复或分类推理正文。
8. Assistant 内嵌可视化是 Message 文本中的官方指令，不是 Tool Item、Message
   phase 或 Reasoning。转译器必须原样保留，不能在 wire 层解释或移动。

## 协议能力对照

| 语义 | Chat Completions | Responses API | Codex 内部合同 |
| --- | --- | --- | --- |
| 用户/助手文本 | `messages[].content`、`delta.content` | Message Item 的 `content[]` | `ResponseItem::Message` |
| 中间说明 | 无标准 phase 字段 | Message `phase = commentary` | `MessagePhase::Commentary` |
| 最终回答 | 无标准 phase 字段 | Message `phase = final_answer` | `MessagePhase::FinalAnswer` |
| 未分类助手文本 | 普通 `content` | phase 缺失的 Message | `phase = None` |
| 推理 | 无标准推理正文 Item | 独立 Reasoning Item | `ResponseItem::Reasoning` |
| 推理摘要 | 无标准字段 | `reasoning.summary[]` | Reasoning summary |
| 工具调用 | `message.tool_calls[]`、`delta.tool_calls[]` | 独立 Function Call Item | `ResponseItem::FunctionCall` |
| 工具结果 | `role = tool` | Function Call Output Item | `ResponseItem::FunctionCallOutput` |
| 内嵌可视化引用 | Assistant `content` 中的 `::codex-inline-vis` 文本 | `output_text` 中的同一文本 | Agent Message 原始文本 |
| 流结束 | `[DONE]`、`finish_reason` | typed completed/failed events | `ResponseEvent::Completed` 或错误 |
| 输出顺序 | Choice 内的 Message/Tool call 结构 | 有序 `output[]` | 按首次出现顺序保存 Item |

## Responses 请求转为 Chat 请求

### 顶层字段

| Responses 请求 | Chat 请求 | 规则 |
| --- | --- | --- |
| `model` | `model` | 原值传递 |
| `instructions` | 首条 `system` message | 空值不生成 Message |
| `input[]` | `messages[]` | 按本节规则转换 |
| `tools[]` | `tools[]` | 只暴露具有完整 Chat 语义的工具 |
| `tool_choice` | `tool_choice` | 无兼容工具时强制为 `none` |
| `reasoning.effort` | `reasoning_effort` | 仅发送 Provider 支持的值 |
| `service_tier` | `service_tier` | Provider 支持时传递 |
| `store`、`include`、`previous_response_id` | 无 | 不得伪造等价物 |
| `reasoning.encrypted_content` | 无 | 不得发送给第三方 Chat Provider |

### Message

| Responses Item | Chat Message | 规则 |
| --- | --- | --- |
| User Message | `role = user` | 文本按原序连接 |
| Developer Message | `role = system` | 当前 Chat 兼容合同使用 system alias |
| Assistant Message | `role = assistant` | 文本可传递；phase 在 Chat wire 上丢失 |
| Message phase | 无 | 不得写入正文或系统提示来模拟 |
| 不支持的多模态内容 | 无 | 按 Provider 能力过滤，不能伪装成文本 |

连续的同角色纯文本 Message 可以在不改变文本顺序的前提下合并。带 Tool call 的
Assistant Message 不得与相邻普通 Assistant Message 无条件合并。

### Tool

1. 顶层 Function Tool 转为 Chat Function Tool。
2. Namespace 内的 Function Tool 使用请求级唯一 wire name 展平。
3. 转译器必须保留 wire name 到原始 `{namespace, name}` 的反向映射。
4. Provider 返回 Tool call 后，必须先恢复原始 Tool 身份，再交给 Codex 执行。
5. `tool_search`、OpenAI hosted tool、Custom freeform tool 和未知 Tool kind 在没有
   完整 Chat 生命周期时必须隐藏。
6. 相邻的并行 Function Call 可以合并为同一 Chat Assistant Tool-call Message。
7. Function Call Output 必须通过 `tool_call_id`/`call_id` 与调用关联。

### Reasoning

标准 Chat wire 没有可安全承载 Responses Reasoning Item 的字段：

- Reasoning Item 默认不进入 Chat `messages[]`。
- 不得把 reasoning summary 拼入 Assistant Message。
- 不得把 encrypted reasoning 转交第三方 Provider。
- 若未来 Provider 提供明确的持久推理合同，必须在 Provider 能力中单独声明，
  不能改变标准 Chat 默认行为。

## Chat 流转为 Codex ResponseEvent

### 标准字段

| Chat 流字段 | Codex 事件 | Phase |
| --- | --- | --- |
| `delta.content` | Message added/delta/done | `None` |
| `delta.tool_calls[]` | FunctionCall added/done | 不适用 |
| `finish_reason = tool_calls` | 响应终止原因 | 不产生 phase |
| `finish_reason = stop` | 响应终止原因 | 不产生 phase |
| Usage | TokenUsage | 不产生 Message/Reasoning |
| `[DONE]` | `ResponseEvent::Completed` | 不改变已完成 Item |

标准 Chat `content` 必须转为用户可见 Message，且 phase 为 `None`。不得根据
Tool call 是否存在，把它改为 `commentary`、`final_answer` 或 Reasoning。

### Provider 扩展

Provider 扩展必须通过 Provider-scoped capability 显式启用。默认值一律为
`none`。

建议的能力模型：

```text
ChatOutputSemantics
  phase_field: none | phase
  reasoning_field: none | reasoning_content
```

启用后按以下规则处理：

| 扩展字段 | 前置条件 | Codex 映射 |
| --- | --- | --- |
| `phase = commentary` | `phase_field = phase` | Message phase Commentary |
| `phase = final_answer` | `phase_field = phase` | Message phase FinalAnswer |
| `reasoning_content` | `reasoning_field = reasoning_content` | 独立 Reasoning Item |
| 未知 phase | 任意 | 忽略该字段，Message phase 保持 None |
| 空 reasoning content | 任意 | 不创建 Reasoning Item |

Provider 扩展不得通过字段名猜测自动开启。配置、模型目录和缓存必须继续按
Provider/Profile 隔离。

### Item ID

一个逻辑 Item 在 added、delta、done 和历史恢复中必须使用同一稳定 ID。

建议 ID：

| Item | ID 来源 |
| --- | --- |
| Message | `{response_id}-message` |
| Reasoning | `{response_id}-reasoning` |
| FunctionCall | 上游 `tool_call.id` |
| 缺失 Tool call ID | 终止为协议错误，不生成可执行 Tool call |

重试或流中断后不得把一个已开始 Item 追加成第二个相同正文 Item。

### 顺序

Chat 流状态必须维护有序逻辑 Item 表，而不是分别保存“全部文本”和“全部工具”
后在 `[DONE]` 时重新排序。

1. 第一次看到非空文本时登记 Message Item。
2. 第一次看到某个 Tool-call index/ID 时登记 FunctionCall Item。
3. 同一 Tool call 的后续 name/arguments delta 合并到原 Item。
4. Item `done` 必须按登记顺序发出。
5. 同一 chunk 同时包含文本和 Tool call 时，使用稳定顺序：Message 在前，
   Tool call 按 index 升序在后。
6. 多个 Tool call 按首次出现顺序保存；index 只用于合并，不得造成已有 Item
   重新排序。
7. Response completed 只能在所有已登记 Item done 后发送。

Chat 单个 Assistant Message 内部可能同时包含文本和 Tool calls，Codex 只能在
Item 粒度保存顺序。转译器不得把一个 Message 的同一文本复制成多个 Item 来模拟
Token 级交错。

### 生命周期

| 状态 | 必须行为 |
| --- | --- |
| 首个文本 delta | 发送一次 Message added，再发送 delta |
| 后续文本 delta | 只发送 delta |
| 首个 Tool delta | 登记 FunctionCall slot；取得稳定 ID/name 后只发送一次 added |
| `[DONE]` | 校验、按序发送每个 Item done，再发送 response completed |
| SSE 解析失败 | 返回明确 Stream/Protocol error |
| Idle timeout | 返回 terminal timeout error |
| 上游提前断流 | 返回 terminal interrupted error |
| Tool call 缺 ID/name/合法 arguments | 返回 terminal protocol error |

未知 JSON 字段可以忽略；损坏 JSON、无法完成的 Tool call 和提前断流不得静默
跳过后继续显示成功。

## Assistant 内嵌可视化

官方 Codex 使用 `file` 独立行指令引用真实 HTML 文件。本项目在相同编排语法中
增加 `artifact` 属性，用于经 Platform 授权的类型化组件：

```text
::codex-inline-vis{file="chart.html"}
::codex-inline-vis{artifact="map-7d67b30d"}
```

该语法与 Chat/Responses wire 正交：

1. Chat `delta.content` 与 Responses `output_text.delta` 都把它当普通 Assistant
   文本传输。
2. Chat 转译不得把指令变为 Tool call、额外 Message、`replyCard` 或 phase。
3. Responses 消费路径不得因为原生 Item 更丰富而使用另一种地图编排合同。
4. Reasoning、Tool output、用户消息或代码块中的同样字面量不是可视化引用。
5. Artifact 必须由更早完成的 Tool 生成；指令只负责引用和位置。
6. `create_map_card` 完成但后续 Assistant 没有引用时，页面不得自动显示地图。

Chat 流累计器不需要理解指令内部字段，但必须保持 delta 字节顺序、稳定 Message
ID 和完整文本。Presentation 层负责处理跨 delta 的不完整指令，不能要求转译器
延迟或重写 Message。

## app-server 与 Platform 投影

本规范不要求新增 app-server Item 类型：

- Agent Message 继续使用现有可选 `phase`。
- Reasoning 继续使用独立 Reasoning Item。
- Tool call、Command、Approval 和 MCP Tool 继续保持各自 Item。
- Platform 只投影 `commentary` 和 `final_answer` 两个已知 phase。
- phase 缺失必须原样保留为缺失，Platform 不得根据位置、Tool 数量或 Turn 末尾
  补写 phase。
- MCP Tool 的 Inline Visualization Artifact envelope 只登记 Artifact，不直接
  产生可视卡片。
- Agent Message 继续携带完整 Markdown 和 `::codex-inline-vis` 指令；Platform
  不把它拆成新的 Runtime Item。
- 浏览器只能通过授权 Artifact DTO 解析安全 `artifact` 句柄，不能接收 MCP Resource
  URI、Profile 路径或 Runtime request ID。

## WebApp/TUI 展示合同

| Item | 展示 |
| --- | --- |
| Message + Commentary | 用户可见的执行中说明，可进入执行分组 |
| Message + FinalAnswer | 正式助手回复 |
| Message + phase None | 普通用户可见助手消息，不得展示成 Reasoning |
| Reasoning | 独立 Reasoning 组件 |
| Tool/Command/Approval | 对应的 typed presentation |
| Message 中的 Inline Visualization 指令 | 在同一 Message 的对应位置展示 Artifact |
| 产出 Artifact 的 Tool | Tool 保持 Tool；完成时不自动展示 Artifact |

实时与历史必须消费同一 Item kind、ID、phase 和顺序。历史恢复不得根据正文、
相邻 Tool 或“最后一条助手消息”重新分类。指令解析是严格的官方语法解析，不是
正文关键词或位置启发式。

## 历史与切换

1. 不重写既有 Codex rollout。
2. 不迁移或猜测既有 `commentary` Message 的真实含义。
3. 新转译规则只影响升级后新产生的 Chat Turn。
4. 既有错误分类仍按其持久化 phase 展示；这是不可无损修复的历史事实。
5. 不为旧错误分类增加正文关键词、位置或 Tool 数量兼容分支。
6. 实时 Item completed 是当前 Turn 的权威状态；历史以 Codex
   `thread/turns/list(itemsView=full)` 为权威。
7. Inline Visualization 使用同一 Agent Message 原文和 Artifact registry 恢复；
   不从旧 `replyCard`、Tool 文本或 Assistant JSON 生成指令。
8. 新 Artifact 合同不双写旧 `replyCard`。旧地图历史不增加兼容渲染分支。

## 错误分类

| 类别 | 示例 | 对外结果 |
| --- | --- | --- |
| Transport | HTTP、TLS、连接断开 | Turn failed，显示可诊断错误 |
| Stream | Idle timeout、缺 `[DONE]` | Turn failed，不保持 Working |
| Protocol | JSON 损坏、Tool call 不完整 | Turn failed |
| Provider | 4xx、429、5xx、供应商错误事件 | 保留清洗后的 Provider 错误 |
| Unsupported extension | 未启用的 `reasoning_content`/`phase` | 忽略扩展，保留标准字段 |

错误信息不得包含 API Key、Authorization header、完整敏感 URL、Profile 路径或
原始 app-server request ID。

## 验证矩阵

### `codex-api`

- 纯文本 Chat completion 产生 phase None 的 Message。
- 纯 Tool call completion 只产生 FunctionCall。
- 文本后 Tool call 保持 Message → FunctionCall 顺序。
- Tool call 后文本按首次出现顺序稳定保存。
- 同一 chunk 的文本和多个 Tool call 使用稳定 tie-break。
- 多 chunk Tool name/arguments 正确累计。
- `finish_reason` 不产生 phase。
- Usage reasoning token 只进入 TokenUsage。
- 显式 Provider phase 能力正确映射。
- 显式 Provider reasoning 能力产生独立 Reasoning Item。
- 未启用扩展时不产生 Reasoning。
- 包含完整或跨 delta 的 `::codex-inline-vis` 时，最终 Message 文本逐字保持一致。
- 损坏 JSON、不完整 Tool call、提前断流和 idle timeout 均明确失败。

### Core/app-server

- 真实 mock `/v1/chat/completions` 的 Tool 往返通过。
- added/delta/done 使用同一 ID，completed 不产生重复正文。
- live notification 与 `thread/turns/list(itemsView=full)` 的 kind、phase、顺序一致。
- Message、Reasoning 和 Tool 在历史中保持独立；Artifact 不变成新 Runtime Item。
- 中断、重试和 Thread resume 不复制 Message。

### Web

- phase None 的完成 Message 展示为正文，不进入 Reasoning。
- Commentary、FinalAnswer 和 Reasoning 使用三个明确路径。
- `create_map_card` 完成时不显示地图。
- Assistant 的“文字 → 指令 → 文字”渲染为同一消息内的“文字 → 地图 → 文字”。
- 不完整流式指令不闪现为正文；完成、切换 Thread 和刷新后位置一致。
- 切换 Thread、恢复历史和审批后继续执行不改变 Item 分类。

### Real smoke

使用第三方 Chat Provider 完成：

1. 文本 preamble。
2. `map_utils` Tool call。
3. `create_map_card` 返回 Artifact envelope 和 embed code，但不显示地图。
4. Tool 后 Assistant 在同一 Message 的两段正文之间写入 embed code。
5. 页面在该 Message 内渲染地图。
6. 刷新并重新读取历史。

实时与恢复后的 Item 数、ID、kind、phase 和顺序必须一致；不得把 preamble
显示为 Reasoning，不得产生重复 Message，也不得把地图恢复为 Tool 附件。

## 验收标准

1. 标准 Chat `content` 不再通过 Tool call 存在性推断 phase。
2. Message、Reasoning 和 Tool 的类型边界在 live/history 中一致。
3. Item 顺序由首次出现决定，不在流结束时重排。
4. phase、reasoning 等 Provider 扩展只有显式能力开启后生效。
5. 错误和断流进入明确终态，不遗留 Working/InProgress。
6. 不新增 Web 侧语义猜测或历史兼容启发式。
7. Chat/Responses 都逐字保留官方 Inline Visualization 指令，且不包含地图特例。
8. Tool 完成不自动渲染；只有 Agent Message 引用决定 Artifact 位置。
9. patch map、能力基线、开发计划、测试和真实 Provider smoke 同步更新。

## 参考

- OpenAI Chat Completions API：
  <https://platform.openai.com/docs/api-reference/chat/create>
- OpenAI Reasoning、Reasoning Item 与 Message phase：
  <https://developers.openai.com/api/docs/guides/reasoning>
- Codex App Server Item 合同：
  <https://learn.chatgpt.com/docs/app-server#items>
- Codex Inline Visualization 上游 `file` 实现：
  <https://github.com/openai/codex/blob/main/codex-rs/tui/src/inline_visualization.rs>
- Apps SDK 数据与 UI 分离：
  <https://developers.openai.com/apps-sdk/build/chatgpt-ui#separate-data-processing-from-ui-rendering>
