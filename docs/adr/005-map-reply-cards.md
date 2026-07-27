# ADR-005: 地图可视化 Artifact 与回复内嵌引用

**状态**: 已接受；已实施

**日期**: 2026-07-24

**关联**: ADR-004、`docs/chat-responses-translation-spec.md`、
`docs/chat-responses-translation-plan.md`

## 背景

迁移前，`map_utils.create_map_card` 一完成，Platform 就把该 MCP Tool Item 的
`structuredContent` 投影为 `replyCard`，浏览器随即在 Tool 所在位置渲染地图。
这种实现把“生成地图 Artifact”和“把地图放进回复”绑定成同一个副作用，导致：

- 地图只能出现在 Tool Item 旁边，不能出现在一条 Assistant Message 的任意位置；
- Tool 调用次数和回复的视觉编排互相耦合；
- 模型无法先生成地图，再决定是否引用、引用几次以及在什么文字之间引用；
- 卡片成为 Web 专用 Tool 附件，不能复用 Codex 已有的内嵌可视化语义。

官方 Codex 已支持由 Assistant 在 Markdown 中写入独立的内嵌可视化指令：

```text
::codex-inline-vis{file="chart.html"}
```

官方 TUI 在实时流、最终消息和历史恢复中识别该指令，并将本地可视化降级为可信
链接。该设计把 Artifact 生成与 Assistant 编排分开，符合本项目需要的长期语义。

## 决策

地图卡片不再是 MCP Tool Item 的自动展示附件。它改为一个可被 Assistant
Message 引用的、Thread 作用域内的 Inline Visualization Artifact。

完整流程分为两个阶段：

1. `create_map_card` 验证地图配置并生成类型化 Artifact，返回一个可复制的
   `::codex-inline-vis{artifact="..."}` 短代码；Tool 完成只登记 Artifact，不渲染地图。
2. 模型把短代码写入 Assistant Message 的目标位置。消息展示层按原始文本顺序把
   Markdown 与可视化组件组合成同一条回复。

Chat Completions 和 Responses API 都只负责传递 Assistant 文本。转译器必须原样
保留指令，不解析地图、不创建卡片 Item，也不因指令改变 Message phase。

## 与官方设计的对齐

本方案复用而不是平行发明以下官方语义：

- 保留官方 `::codex-inline-vis{file="..."}` 对真实 HTML 文件的原始语义，并在同一
  独立行编排语法中增加 `artifact="..."` 类型化 Artifact 引用；
- 复用 Codex Agent Message 作为回复编排的事实来源；
- 复用 MCP `structuredContent`、`outputSchema` 和 ResourceLink 作为 Tool 与模型间
  的结构化合同；
- 复用 Apps SDK“数据 Tool 与渲染 Tool 分离”的原则：地理工具产出数据，
  `create_map_card` 只构造最终可视化 Artifact；
- 复用 Codex Thread/Turn 历史；不创建 Web 私有的第二套消息历史或排序规则。

`artifact` 属性是 Web presentation 的窄扩展，不改变 Codex Runtime transport 或
app-server Item。它不是 map 专用 Markdown：具体组件由 renderer registry 决定。
不会新增 HTML 注释、伪 Tool Item、Assistant 位置启发式或 Chat/Responses 私有字段。

Apps SDK 的 `outputTemplate` 仍适合定义组件资源和 Tool 数据/UI 分离，但标准
挂载位置属于 render Tool call。单独采用它仍会把组件位置绑定在 Tool Item 上，
不能满足“在一条回复的任意文字之间出现”。因此本方案复用其数据/组件合同，并用
Codex 官方 Inline Visualization 指令承担 Assistant 编排；两者不是互斥协议。

## 所有权

| 层 | 所有权 |
| --- | --- |
| Codex Runtime | Thread/Turn、Tool 生命周期、Agent Message 文本与顺序 |
| `codex-api` | Chat/Responses 的文本、Tool、Reasoning、phase 和流生命周期转译 |
| `map_utils` | 地理数据、`map.v3` 验证、Artifact 描述和内嵌短代码生成 |
| Platform Server | Artifact 注册、Thread/Run/组织授权、MCP Resource 解析和浏览器安全 DTO |
| Web presentation | 官方指令语法解析、消息内分段、Artifact renderer 分派 |

内嵌地图不要求修改 Codex Core 或新增 app-server Item。若官方 app-server 后续提供
类型化 Inline Visualization 内容项，应直接收敛到官方类型并删除 Web 侧等价解析，
而不是长期维护平行协议。

## 规范流程

```text
User
  -> map_utils data tools
  -> MCP Resource / data_ref
  -> create_map_card
  -> Inline Visualization Artifact + embed code
  -> MCP Tool completed（只登记，不展示卡片）
  -> Assistant Message:
       Markdown before
       ::codex-inline-vis{artifact="map-...."}
       Markdown after
  -> Web:
       Markdown segment
       Map renderer
       Markdown segment
```

一次 Assistant Message 可以引用零个、一个或多个 Artifact；同一个已完成 Artifact
可以在所属 Run/Thread 的当前或后续 Turn 被再次引用。没有被 Assistant 引用的
Artifact 不展示。

## MCP 输出合同

地理数据 Tool 继续通过标准 MCP Resource 返回大型 GeoJSON。完整 `data_ref` 使用
原始 MCP server ID 和 `resource_link.uri`，不把 GeoJSON 复制进 Assistant 文本或
地图描述。

`create_map_card` 只定义一个 `map.v3` 合同。`sources` 是平台管理的 GeoJSON
source：内联数据使用标准 `source.data`，Resource 使用互斥的 Open Web
`source.data_ref`，其他标准 GeoJSON source option 保留。`layers` 直接使用官方
Mapbox Style Specification Layer JSON，并由官方 validator 校验；Tool、Platform 和
Web 不再各自维护 layer type、paint/layout、filter 或 expression 白名单。官方未知
属性诊断作为 warning 返回，已知语法错误仍然失败。标准 camera 字段位于顶层；
文本 hover 和 legend 都是 `extensions` 下的可选 Open Web 行为。

`create_map_card` 的目标输出是通用 Artifact envelope，而不是 Tool 附带的
`replyCard`：

```json
{
  "type": "open-web-artifact",
  "kind": "inline-visualization.v1",
  "artifact": {
    "ref": "map-7d67b30d",
    "renderer": {
      "kind": "map.v3",
      "payload": {
        "title": "北京到上海路线",
        "intent": "route",
        "status": "ready",
        "sources": {
          "route-data": {
            "type": "geojson",
            "data": {
              "type": "mcp_resource",
              "server": "map_utils",
              "uri": "maps-data://geojson/map-data-8a4c...",
              "format": "geojson"
            }
          }
        },
        "layers": [
          {
            "id": "route",
            "type": "line",
            "source": "route-data",
            "paint": {
              "line-color": "#2563eb",
              "line-opacity": 0.9,
              "line-width": 5
            }
          }
        ]
      }
    }
  },
  "embed": {
    "syntax": "codex-inline-vis.artifact.v1",
    "code": "::codex-inline-vis{artifact=\"map-7d67b30d\"}"
  }
}
```

合同规则：

- Tool 必须声明 `outputSchema`，并在返回前验证 envelope、`map.v3` 和字段间不变量。
- `artifact.ref` 必须由 Tool 生成、不可预测且不具有路径语义；只允许有界的
  ASCII 字母、数字、点、下划线和连字符，不允许引号或路径分隔符。
- `embed.code` 必须由 Tool 根据 `artifact.ref` 生成；模型只复制，不自行拼接。
- Platform 通过 envelope 类型和 renderer registry 识别 Artifact，不根据
  `serverName == map_utils` 或 `toolName == create_map_card` 硬编码分支。
- `renderer.kind` 是版本化 renderer 能力 ID。`map.v3` 只是第一个实现；图表、
  表单等类型化组件使用同一 envelope 和 `artifact` 指令。真实 HTML 文件仍使用
  官方 `file` 指令，不包装成类型化 Artifact。
- `content` 只向模型说明 Artifact 已准备好，并要求把 `embed.code` 原样放到目标
  回复位置。它不是渲染输入。
- `map.v3.layers` 是标准 Mapbox Layer JSON。Web 只替换本地 layer/source ID，
  其余字段原样交给 `map.addLayer`。
- hover 只引用 GeoJSON property，Web 使用 `textContent` 构造弹层，不接受
  Tool 提供的 HTML。远程图标只允许 HTTPS 栅格格式，合同拒绝 SVG、非 HTTPS URL、
  未知字段和互相冲突的内建/自定义样式。
- Web renderer 固定使用 Mercator 投影。卡片正文只呈现 summary/fallback 和可选
  `extensions.legend`，不呈现 source/layer 计数或 camera 调试值。

## Artifact 注册与授权

现有 `reply_artifacts` 继续保存 MCP Resource 缓存；新增通用 Inline
Visualization Artifact registry 保存可被消息引用的类型化组件。两者通过经过验证
的 Resource 引用关联，不把数据缓存和可视化身份混成一张记录。

Inline Visualization registry 当前保存：

- organization、Run、Thread，以及 producer Turn 和 Tool Item provenance；
- Thread 内安全、不透明的 `artifact.ref`；
- renderer kind、版本化 payload 和状态；
- 创建与更新时间。

MCP Resource 的原始 server/URI、MIME、大小、缓存内容和 SHA-256 仍由独立的
`reply_artifacts` Resource cache 保存。Inline Artifact 注册时把合法 Resource 引用
解析为授权 URL 后再持久化 renderer payload，两种身份不混入同一条记录。

解析规则：

- 指令只能引用同一 Run、同一 Thread 中已完成的 Artifact；后续 Turn 可以复用；
- 拒绝跨组织、跨 Run、跨 Thread、前向、自引用、重名和未完成 Artifact；
- 浏览器只接收授权后的 Artifact ID 和安全 renderer DTO，不接收 MCP Resource URI、
  Profile 路径、Runtime request ID 或凭据；
- Resource 继续通过官方 `mcpServer/resource/read` 延迟读取；
- Artifact 注册失败不能吞掉原 MCP Tool completed 事件，Turn 必须进入明确终态。

Thread 作用域的 `artifact.ref` 是 Assistant 可复制的展示句柄；GeoJSON 的规范数据身份仍是
MCP server + Resource URI，不再创建第二套地图数据身份。

## Assistant Message 编排

展示层只在 Agent Message 中识别官方指令，并遵循官方 TUI 的语法边界：

- 指令必须独占一行；
- fenced 或 indented code block 中的字面量不解析；
- 不完整的流式指令先缓冲，不能把半段短代码闪到页面上；
- 普通 Markdown 保持原顺序；
- Reasoning、Tool output、Command output 和用户消息中的相同文本不解析；
- 无法解析或无权访问的 Artifact 显示明确的 unavailable/error 占位，不回退为
  旧 `replyCard` 或正文 JSON。

Web 将一条 Agent Message 确定性地分解为：

```text
MessageSegment = Markdown(text) | InlineVisualization(reference, artifactId)
```

这只是同一 Agent Message 的 presentation，不是新的 Runtime Item。消息气泡、
复制、选择、实时流和历史恢复都以同一分段结果为准。

## Chat / Responses 转译

内嵌可视化与 wire API 正交：

| 输入 | 规范行为 |
| --- | --- |
| Chat `delta.content` | 原样追加到 Agent Message；指令保持文本 |
| Responses `output_text.delta` | 原样追加到 Agent Message；指令保持文本 |
| Chat/Responses Tool call | 仍是独立 Tool Item |
| Tool result Artifact envelope | 登记 Artifact，不创建可视卡片 |
| Reasoning | 独立 Reasoning Item，绝不解析指令 |
| Message phase | 由 wire/Provider 明确信息决定；指令不改变 phase |

转译器不得扫描、生成、移动或删除内嵌指令。这样 Chat 与 Responses 共用完全相同的
Assistant Message 编排语义，地图不会成为 Chat transport 的特例。

## 实时与历史

- Tool completed 后先持久化 Artifact，再广播可引用状态。
- Assistant delta 使用同一 message ID 累加；指令完成后在原位置替换为组件。
- `item/completed` 不重新排序或复制消息。
- `thread/turns/list(itemsView=full)` 返回原始 Agent Message 文本；历史恢复使用同一
  指令解析器和 Artifact registry。
- 切换 Thread、刷新、审批后继续和中断恢复不得改变 Message/Reasoning/Tool/
  Visualization 的分类或相对顺序。

## 不保留旧卡片兼容路径

迁移采用一次性合同切换：

- 删除“`create_map_card` Tool completed 即投影/渲染 `replyCard`”路径；
- 不双写旧 `open-web-card` Tool 附件和新 Artifact envelope；
- 不扫描旧 Tool 文本、Assistant JSON、关键词或位置来恢复地图；
- 不为既有数据库历史添加 `replyCard -> inline directive` 重写；
- 旧历史保留其原始 Codex Tool/Message 事实，但不会通过兼容分支重新生成地图。

唯一允许的兼容工作是跟随官方 Codex app-server/rollout 合同所必需的适配；不得把
项目旧地图实现混入 `legacy_response_tool_history` 等官方兼容 seam。

## 非目标

- 不把完整卡片 JSON放进 Assistant Markdown。
- 不让 Chat/Responses 转译器理解 `map.v3`。
- 不由 Platform 自动决定卡片位置。
- 不把 MCP Tool Item 伪装成 Assistant Message。
- 不在浏览器中信任或直接执行 Tool 返回的任意 HTML/JavaScript。
- 不以 Apps SDK `outputTemplate` 的自动 Tool 旁挂载替代 Assistant 内编排；未来若
  支持 MCP App 组件，它也必须先注册为 Artifact，再由同一指令引用。

## 已实施路径

1. 固定 Artifact envelope、官方 `file` 语义、扩展 `artifact` 语义和
   live/history 测试向量。
2. `create_map_card` 只生成 Artifact 与短代码。
3. Platform 的 Tool 结果处理使用通用 Artifact 注册。
4. Assistant Message 使用 Markdown/Visualization 有序分段。
5. 旧 `replyCard` DTO、Tool 附件渲染和 map-specific event projection 分支已删除。
6. DeepSeek Chat 的真实浏览器用例已验证“文字—地图—文字”、Thread 切换和刷新恢复。
   Chat phase/order 修复与 Responses Provider 的真实浏览器矩阵仍按
   `docs/chat-responses-translation-plan.md` 独立实施。

## 验收标准

- 调用 `create_map_card` 但不在 Assistant Message 引用时，页面不显示地图。
- 短代码放在两段文字之间时，地图在同一条回复的两段文字之间展示。
- 一条消息可放多张地图；同一 Artifact 可再次引用。
- Tool/审批/Reasoning 仍留在执行组，地图不计为 Tool 行或独立 Assistant Message。
- Chat 与 Responses 对同一输出得到相同的 Message 分段与历史。
- 实时流、Turn 完成、Thread 切换和刷新后的结构与顺序一致。
- 不存在旧 `replyCard` 自动渲染、双写、文本 JSON 或位置推断分支。
- Platform 代码不按 `map_utils`/`create_map_card` 名称识别通用 Artifact。
- 跨组织、跨 Run、跨 Thread、前向和未授权引用全部拒绝。
- Mapbox/Google Key、MCP URI、本地路径和 Runtime request ID 不进入浏览器消息。

## 参考

- OpenAI Codex inline visualization source:
  <https://github.com/openai/codex/blob/main/codex-rs/tui/src/inline_visualization.rs>
- OpenAI Visualizations:
  <https://learn.chatgpt.com/docs/visualizations>
- OpenAI Apps SDK, separate data processing from UI rendering:
  <https://developers.openai.com/apps-sdk/build/chatgpt-ui#separate-data-processing-from-ui-rendering>
- OpenAI Apps SDK inline card guidance:
  <https://developers.openai.com/apps-sdk/concepts/ui-guidelines#inline-card>
