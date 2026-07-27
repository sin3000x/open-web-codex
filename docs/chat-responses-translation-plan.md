# Chat Completions / Responses 转译实施方案

## 目标

把第三方 Chat Completions Provider 的输出可靠地转为 Codex Responses-shaped
Item/Event，同时满足：

- 不把普通 Chat `content` 误判为 Reasoning 或 `commentary`。
- 保留 Message、Reasoning 和 Tool 的类型与顺序。
- 让地图成为 Assistant Message 内可按位置引用的 Inline Visualization Artifact，
  而不是 MCP Tool 一完成就展示的 `replyCard`。
- 实时、Thread 切换、刷新恢复和审批后继续执行使用同一权威合同。
- Provider/模型扩展保持隔离，未知 Provider 默认走安全的标准 Chat 语义。
- 流错误、Tool call 不完整、超时和断网进入明确终态。
- Chat transport 继续集中在 `codex-api`，不把 Provider 语义扩散到 Core 或 Web。

字段和生命周期的规范定义见
`docs/chat-responses-translation-spec.md`。

## 当前事实与缺口

当前 `codex-api/src/sse/chat.rs` 在整个 Chat 响应结束时使用以下规则：

```text
没有 Tool call -> final_answer
存在 Tool call -> commentary
```

该规则不是 Chat Completions 协议字段，而是本地启发式。它造成四个直接问题：

1. 与 Tool call 同时返回的用户可见正文被折叠进执行过程。
2. `commentary` 容易在 UI 上被误认为 Reasoning，尽管二者是不同 Item。
3. 文本和 Tool call 分开累计，在 `[DONE]` 时按实现顺序发出，可能丢失首次出现
   顺序。
4. 当前 Chat delta DTO 不承载任何显式 Provider reasoning/phase 扩展；既不能
   可靠区分，也没有 Provider-scoped 开关。

现有 app-server 和 Web 合同已经具备本次改造需要的基础：

- Agent Message 的 `phase` 已是可选字段。
- Reasoning 已是独立 Thread Item。
- Platform 只投影已知 phase。
- Web 已分别渲染 Agent Message、Reasoning、Tool、Approval 和 Command；Agent
  Message 内部可按原始文本顺序组合 Markdown 与 Inline Visualization Artifact。

地图展示耦合已经移除：`create_map_card` 只返回 Artifact envelope 和 embed code，
Tool completion 不展示卡片。官方 Codex 在 TUI 中实现
`::codex-inline-vis{file="..."}` 的实时与历史解析；Web 沿用其 Assistant 文本编排
与独立行规则，并以 `artifact="..."` 引用类型化组件。因此内嵌地图没有新增
app-server Item，也没有在 Chat transport 中创建地图特例。

## 边界

| 层 | 本次职责 | 不属于本次职责 |
| --- | --- | --- |
| `codex-api` | Chat 请求/响应 DTO、流累计、Item kind/phase、Tool identity、顺序和错误 | Thread/Turn 状态、UI、Provider Secret |
| Provider crates | 声明 Provider-scoped Chat 扩展能力 | 解析 SSE、展示 Reasoning |
| Core | 选择 `WireApi::Chat` 并消费统一 `ResponseEvent` | 根据 Tool/文本猜 phase |
| app-server protocol | 保留现有 Agent Message phase 和独立 Reasoning | 为 Chat 新建平行协议 |
| `map_utils` | 生成版本化 Artifact envelope 和官方 embed code | 决定回复位置 |
| Platform Server | 注册/授权 Artifact，投影安全 renderer DTO | 修正 Runtime phase、自动展示 Tool Artifact |
| WebApp/TUI | 按 Item kind/phase 展示；严格解析官方 inline-vis 指令 | 从位置、关键词或 Tool 数量推断语义 |

## 非目标

- 不把 Responses API 的全部能力模拟到 Chat。
- 不暴露原始 chain of thought。
- 不重写既有 rollout 或数据库历史。
- 不用关键词识别“思考”“正文”或“最终回答”。
- 不把地图 JSON 编码进 Assistant Markdown；Markdown 只保留官方短引用指令。
- 不新增 map 专用 Markdown、Chat 字段或 app-server Item。
- 不改变 MCP Tool、Approval、Command 或 Artifact 所有权。
- 不在本次改造中迁移所有 Provider 到 Responses API。

## 实施原则

1. 先修正标准 Chat 默认语义，再增加可选 Provider 扩展。
2. 每个阶段都可独立提交、验证和回滚。
3. 不先改 Web 掩盖 Runtime 错误。
4. 不新增生成协议字段，除非 Provider 能力配置确实要求。
5. 对高冲突 Codex 文件先接受上游结构，再重放最小 retained seam。
6. Canonical 文档只记录当前状态；某阶段完成后再把对应事实标为完成。
7. Inline Visualization 复用官方指令；地图 Artifact 合同留在 MCP/Platform/Web，
   不扩大 Provider Chat retained seam。

## 工作分解

### 阶段 0：上游与差异门禁

**Owner：** Codex customization workflow

在修改 `codex/` 前：

1. 运行 `scripts/codex-upstream-status.sh`。
2. 运行 `scripts/codex-customization-status.sh`。
3. 检查 official main 是否已经提供 Chat phase/reasoning 转译。
4. 将本次变化归入现有 `provider-chat-transport` retained seam，不新建宽泛 seam。
5. 如果上游已有等价实现，采用上游结构并把本地部分标为 `upstreamed`。

**退出条件：**

- 已记录当前 upstream 基线。
- patch map 明确本次变化仍属于 Chat transport。
- 没有修改尚未同步的高冲突文件后再反向处理上游结构。

### 阶段 1：用测试固定目标合同

**Owner：** `codex-api`

先增加失败测试，覆盖：

1. 标准纯文本输出 → Message，phase None。
2. 文本 + Tool call → Message phase None + FunctionCall。
3. `finish_reason = tool_calls` 不改变 Message phase。
4. 文本首次出现早于 Tool → 历史顺序保持 Message → Tool。
5. Tool 首次出现早于文本 → 顺序保持 Tool → Message。
6. 多 Tool call 的 index 分片、乱序 delta 和 arguments 累计。
7. added/delta/done 使用同一 Item ID。
8. `[DONE]` 前断流、损坏 JSON、缺失 Tool ID/name 明确失败。
9. Usage 中 reasoning token 只进入 TokenUsage。

测试应以完整事件序列 deep equality 为主，避免只断言单个 phase。

**涉及文件：**

- `codex/codex-rs/codex-api/src/sse/chat.rs`
- 新的 sibling test module，避免继续扩大现有 500+ 行实现文件
- Core Chat mock integration tests

**退出条件：**

- 新测试准确复现当前误分类和重排。
- 测试未通过前不改 Web。

### 阶段 2：标准 Chat 语义修复

**Owner：** `codex-api`

最小行为改动：

1. 删除 `tool_calls.is_empty()` 驱动的 phase 推断。
2. 标准 `delta.content` 完成后生成 `phase = None` 的 Message。
3. `finish_reason` 只保留为终止/诊断事实，不参与 phase 分类。
4. 保持纯 Tool call 响应不生成空 Message。
5. 保持 Usage、Server model header 和 request ID 行为不变。

该阶段即可修复“地图介绍正文被当成思考折叠”的核心问题。

**不在本阶段做：**

- `reasoning_content`。
- Provider phase 扩展。
- app-server Schema 变化。
- 浏览器补偿逻辑。

**退出条件：**

- 标准 Chat 测试全部通过。
- 现有 Chat Tool/MCP/namespace 测试通过。
- phase None 的 Message 在 app-server live/history 中均保留为 None。

### 阶段 3：有序 Chat 流累计器

**Owner：** `codex-api`

将当前分离的 `assistant_text`/`tool_calls` 累计改为有序 Item 累计器。

建议新增内部结构：

```text
ChatStreamAccumulator
  response_id
  ordered_slots[]
  message_state?
  tool_calls_by_index
  token_usage

ChatOutputSlot
  Message
  ToolCall(index)
  Reasoning       # 阶段 4 启用
```

行为：

1. 首次遇到某类逻辑 Item 时登记 slot。
2. Tool delta 仍按 index 合并，但 slot 不重排。
3. 同一 chunk 同时出现文本和 Tool 时，Message 先登记，Tool 按 index 登记。
4. `[DONE]` 时先校验全部 Item，再按 slot 顺序发送 done。
5. 任一校验失败时不发送 response completed。

`sse/chat.rs` 当前超过 500 行。新累计状态应进入独立模块，例如：

```text
codex-api/src/sse/chat.rs
codex-api/src/sse/chat_state.rs
codex-api/src/sse/chat_state_tests.rs
```

不为遵守行数规则而机械移动无关代码；只提取本次拥有的 DTO、累计状态和测试。

**退出条件：**

- 实时 Item added/done 与历史 Item 顺序一致。
- 文本和多个 Tool call 的顺序不依赖 `[DONE]` 处理顺序。
- 中断流不会产生“completed”假成功。

### 阶段 4：Provider-scoped phase/reasoning 扩展

**Owner：** Provider metadata + `codex-api`

在标准语义稳定后，增加显式 Provider 能力：

```text
ChatOutputSemantics
  phase_field: none | phase
  reasoning_field: none | reasoning_content
```

默认：

```text
phase_field = none
reasoning_field = none
```

实施内容：

1. Provider/模型元数据声明能力；缓存 key 必须包含 Provider identity。
2. Core 只把解析后的能力传给 Chat endpoint，不解析字段。
3. Chat delta DTO 可以接受扩展字段，但只有能力开启时才使用。
4. `reasoning_content` 产生独立 Reasoning Item。
5. `phase` 只接受 `commentary`/`final_answer`；未知值退化为 None。
6. Provider 切换后新 Turn 必须使用新 Provider 的语义能力。

如果能力需要进入配置 Schema或 app-server Provider API：

- 修改 Rust protocol/config 源。
- 重新生成 Schema 和 TypeScript。
- 更新 offline fixtures。
- 运行真实 app-server smoke。

浏览器 Provider 表单是否暴露这两个高级选项应单独评审。已知内置 Provider 可由
Provider facts 给出；未知 Custom Provider 默认不开启，避免用户误配造成正文泄漏或
错误分类。

**退出条件：**

- 启用与未启用扩展的测试完全隔离。
- DeepSeek/兼容 Provider 的 reasoning 正确进入 Reasoning Item。
- 不支持扩展的 Provider 不受影响。

### 阶段 5：app-server 与消息分类一致性

**Owner：** app-server、Platform 和 Web 的现有 typed contract 消费者

本阶段只完成 Chat/Responses Message 分类，不实现地图 Artifact：

#### app-server

- `item/started`、delta、`item/completed` ID 一致。
- `thread/turns/list(itemsView=full)` 保留 phase None。
- Reasoning 与 Agent Message 不合并。
- Live 与 history 的 Item 顺序一致。

#### Platform

- 只投影 Runtime 给出的已知 phase。
- phase None 不补写。
- Reasoning 不转换为 Agent Message。

#### Web

- phase None 的 Agent Message 是用户可见正文。
- Commentary 与 Reasoning 保持不同组件。
- live、切换 Thread、刷新和审批恢复使用同一分类。
- 不增加 last-message、Tool-count 或关键词推断。

**退出条件：**

- 同一 Turn 的实时与恢复结构一致。
- 正文不会进入 Reasoning 折叠。
- Reasoning 不会恢复成正文。
- 不产生重复 Agent Message。

### 阶段 6：Inline Visualization Artifact 与 Assistant 编排

**Owner：** `tools/maps-mcp`、Platform Artifact registry、Web presentation

**状态：** 已完成。真实 DeepSeek/Mapbox 浏览器用例已验证同一 Agent Message 的
“文字—地图—文字”、Thread 切换和刷新恢复；Responses Provider 的真实浏览器矩阵
保留在阶段 7。

本阶段保留官方 `::codex-inline-vis{file="..."}` 的真实 HTML 语义，并在相同的
Assistant 独立行编排语法中实现 `::codex-inline-vis{artifact="..."}` 类型化引用；
不修改 `codex-api`、Core 或 app-server Item 类型。

#### 6.1 固定通用合同

先用 Schema 和测试向量固定：

- `open-web-artifact` / `inline-visualization.v1` envelope；
- 安全、不可预测、Thread 作用域且无路径语义的 Artifact ref；
- `embed.syntax = codex-inline-vis.artifact.v1` 与由 Tool 生成的完整 `embed.code`；
- renderer registry；第一个 renderer 是 `map.v3`；
- 官方语法边界：独立行、代码块不解析、不完整 delta 缓冲。

合同不得包含 `map_utils` 或 `create_map_card` 名称判断。Tool 身份只用于审计；
Artifact 类型、renderer capability 和授权记录决定处理方式。

#### 6.2 改造 `create_map_card`

- 保留现有 viewport、source/layer、样式和 MCP Resource 验证。
- 返回通用 Artifact envelope、`map.v3` renderer payload 和可复制 embed code。
- `content` 明确要求模型把 embed code 原样放入目标 Assistant 回复位置。
- Tool 完成不再意味着“显示卡片”。
- 不双写旧 `open-web-card` Tool 附件和新 envelope。
- 更新 `map-utils` Skill：调用数据 Tool → 调用 `create_map_card` → 把返回短代码
  放入最终回复；不得复制 JSON。

#### 6.3 Platform Artifact registry

- 从通用 envelope 注册 Artifact，不直接投影 `replyCard`。
- 复用现有 organization/Run/Thread ownership、producer provenance 和 MCP Resource read。
- `(Run, artifact.ref)` 唯一，并记录 Thread 归属与 producer Turn/Item；Resource
  只允许更早完成的 producer。Assistant 可在同一 Run/Thread 的后续 Turn 引用
  已完成 Artifact，拒绝前向、自引用、跨 Run/Thread、重名和未完成引用。
- 只向浏览器返回安全 Artifact ID 和 renderer DTO。
- Artifact 失败不能吞掉 Tool completed 或让 Turn 停在 InProgress。

#### 6.4 Web 消息内编排

新增一个官方语法的确定性 parser，把单条 Agent Message 分成：

```text
Markdown(text) | InlineVisualization(reference, artifactId)
```

要求：

- parser 语义与上游 TUI 测试向量一致；
- 只解析 Agent Message，不解析 Reasoning、Tool、Command、用户消息或代码块；
- 流式半段指令不显示短代码，也不导致整条消息闪烁；
- Artifact renderer 作为同一消息的 block segment 展示；
- live 和 history 使用同一 parser、resolver 和 renderer；
- 删除 Tool Item 后自动插入 `ReplyCard` 的 MessageList 分支。

#### 6.5 删除旧路径

- 删除 `replyCard` Tool attachment DTO、投影、历史恢复和前端渲染分支；
- 删除按 `map_utils`/`create_map_card` 识别卡片的 Server 常量和条件；
- 不增加旧历史迁移、双读、文本 JSON 或位置兜底；
- 旧历史保留原始 Tool/Message，但不通过兼容代码重新生成地图。

**退出条件：**

- 调用 Tool 但不引用时不显示地图。
- 一条 Assistant Message 的“文字 → 指令 → 文字”显示为“文字 → 地图 → 文字”。
- 多卡片、重复引用、Thread 切换和刷新顺序一致。
- Web/Server 没有 Tool-name/map-server 硬编码的通用 Artifact 分支。
- `codex/` 没有为地图增加本地改动。

### 阶段 7：Chat、Responses 与真实 Provider E2E

**Owner：** Cross-project validation

固定以下序列：

```text
User Message
Assistant Message phase=None（可选 preamble）
map_utils data Tool
create_map_card Tool -> Artifact registered, no visual output
Assistant Message:
  text before
  ::codex-inline-vis{artifact="map-...."}
  text after
```

E2E 步骤：

1. 分别使用 Responses Provider 和第三方 Chat Provider 创建新 Thread。
2. 发送“在一段文字中间显示地图卡片”。
3. 记录 live Item 的 ID、kind、phase、原始 Message 文本和 Artifact 注册状态。
4. 确认 Tool 完成到指令出现之前没有地图。
5. 等待 Turn 完成并检查地图位于同一 Assistant Message 内。
6. 切换到其他 Thread 再返回。
7. 刷新页面并读取 authoritative history。
8. 比较 live/completed/history 三份结构。
9. 覆盖拒绝配置、无效 Key、MCP Resource 失败、无效 Artifact、流中断和无效
   Tool call，确认 Turn 与 Artifact 都有明确终态。

Secret 只通过现有测试环境变量和 Provider Secret 注入，测试日志不得输出 Key。

**退出条件：**

- Chat 与 Responses 的可视化位置、Message 文本和历史结构一致。
- 真实 Provider + MCP + Inline Visualization 通过。
- 刷新前后 Item 数量、类型和顺序相同。
- 负向用例进入明确终态。

### 阶段 8：文档与同步收口

每完成一个阶段后更新：

- `docs/capability-baseline.md`：只写已验证事实。
- `docs/development-plan.md`：更新完成状态和剩余项。
- `docs/custom-codex-patch-map.md`：说明 Inline Visualization 完全复用 upstream，
  不扩大 Chat retained seam。
- `docs/architecture.md`：完成实现后把旧 Tool attachment 流程替换为当前 Artifact
  流程。
- `docs/adr/005-map-reply-cards.md`：保持最终合同与实现一致。
- `docs/chat-responses-translation-spec.md`：规范发生变化时先更新规范再改实现。

不得在实现完成前把 Proposed 行为写成 available。

## 提交拆分

建议拆为以下可审查提交：

1. `docs: define Chat and Responses translation contract`
2. `test(codex-api): cover phase-neutral ordered Chat output`
3. `fix(codex-api): stop inferring message phase from tool calls`
4. `refactor(codex-api): preserve Chat output item order`
5. `feat(provider): declare optional Chat output semantics`
6. `test(app-server): align live and restored translated items`
7. `test(web): keep unknown-phase messages separate from reasoning`
8. `feat(map-utils): return inline visualization artifacts`
9. `feat(platform): register authorized inline visualization artifacts`
10. `feat(web): compose inline visualizations inside agent messages`
11. `refactor(web): remove tool-attached reply cards`
12. `test(smoke): verify Chat and Responses inline visualization ordering`
13. `docs: record verified translation and artifact behavior`

阶段 4 没有立即需求时可以独立延期；阶段 2、3、5 和 6 仍能完成标准 Chat 修复。

## 验证命令

Codex 改动按实际范围运行：

```bash
cd codex/codex-rs
just fmt
just test -p codex-api
just test -p codex-app-server-protocol
```

如果改动进入 Core：

```bash
cd codex/codex-rs
just test -p codex-core <focused-chat-tests>
```

如果改动协议/config：

```bash
cd codex/codex-rs
just write-app-server-schema
just test -p codex-app-server-protocol
```

跨项目：

```bash
npm run typecheck
npm run test
npm run check:codex-contracts
npm run smoke:codex-app-server -- --require-manifest
scripts/smoke-third-party-map-card-mcp.sh
scripts/smoke-map-card-rendering.sh
```

地图 smoke 已断言“Tool 完成不渲染、Assistant 指令才渲染”，不包含旧
`replyCard` 断言。

提交前还需：

```bash
scripts/codex-upstream-status.sh
scripts/codex-customization-status.sh
```

完整 Codex workspace 测试只在共享/common/core/protocol 改动确实需要时运行，并按
`codex/AGENTS.md` 的完整测试批准规则执行。

## 风险与缓解

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| phase None 改变旧 UI 分组 | 中间 preamble 变成普通正文 | 这是协议正确退化；用 E2E 评审展示，不增加猜测 |
| Provider 扩展字段语义不一致 | reasoning 泄漏或正文丢失 | 默认关闭，Provider-scoped capability 开启 |
| Tool delta 乱序或缺字段 | 错调 Tool、Turn 卡住 | 完成前严格校验，失败进入 terminal error |
| changed Item done order 影响历史 | live/history 顺序变化 | 全序列测试 + Thread reload smoke |
| 上游同步冲突 | retained seam 扩大 | 独立状态模块，Core 只保留 dispatch |
| 旧历史仍有误分类 | 刷新旧 Turn 仍显示旧结果 | 不可无损推断；明确不重写历史 |
| Web 把任意正文当组件代码 | 注入、误渲染、live/history 差异 | 只解析官方独立行指令、代码块排除、严格 basename |
| Artifact 在 Tool 完成时继续自动展示 | 无法由回复编排位置 | 删除 `replyCard` 投影，不双写旧合同 |
| Web 为修复截图增加分类启发式 | 再次产生实时/历史差异 | phase 只消费 typed contract；指令 parser 不参与分类 |
| 通用 Artifact 逻辑硬编码 map Tool 名 | 后续组件继续复制分支 | envelope + renderer registry，不按 server/tool 名判断 |

## 完成定义

全部满足后，实施任务才算完成：

1. 标准 Chat 文本不再根据 Tool call 推断 phase。
2. 显式 reasoning 只生成独立 Reasoning Item。
3. Message/Tool 的首次出现顺序在 live/history 中一致。
4. 中断、超时、损坏 JSON 和不完整 Tool call 都进入终态。
5. app-server、Platform、Web 没有新增语义猜测。
6. `create_map_card` 只生成 Artifact 和 embed code，Tool 完成不显示地图。
7. 地图由同一 Agent Message 中的官方指令决定位置。
8. Chat 与 Responses 的内嵌结果、刷新、Thread 切换和审批恢复一致。
9. 旧 `replyCard` 自动渲染、双写和历史兼容分支已删除。
10. Schema/fixtures/Manifest 在受影响时已重新生成并通过 smoke。
11. patch map 和 Canonical 文档只描述已验证的最终状态。
