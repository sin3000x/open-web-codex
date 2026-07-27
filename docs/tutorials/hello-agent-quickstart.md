# 第一篇：从一个 Python 函数到第一个 Agent Tool

## 这篇要解决什么

假设你只写过普通 Python，现在希望 Agent 能读取业务数据、调用内部系统或执行一段
可靠计算。你首先需要理解的不是一堆名词，而是这条开发路径：

```text
先写清业务目标
  → 判断哪些事让 Agent 决定，哪些事必须由代码执行
  → 把普通 Python 能力公开成 Tool
  → 告诉 Agent 什么时候、按什么规则使用
  → 在真实任务中观察调用并验证结果
  → 根据新需求修改正确的那一层
```

本篇使用“向指定的人问好”作为最小练习。模型本来就会问好，所以它不是值得上线的
业务 Tool；选择它只是为了让输入、输出和错误都能一眼看懂。学会链路后，第二、三篇
会把同样的方法用于订单数据和仓网规划。

仓库已经提供一份可以运行的最小示例。你不会从空文件开始抄代码，而是按以下顺序：

1. 先运行普通 Python，知道最终要交给 Agent 的能力是什么；
2. 再沿着调用链理解每个文件为什么存在；
3. 让真实 Agent 调用它；
4. 最后亲手增加“正式语气”，完成一次从需求到验证的改造。

完成后，你应该能独立判断：

- 改计算或业务规则时，为什么改 `core.py`；
- 改 Agent 可传的参数或可调用的操作时，为什么改 Tool；
- 改使用时机和工作步骤时，为什么改 Skill；
- 为什么这些变化都不是“重新训练模型”；
- 什么情况下才需要另一个 Agent 或子任务。

返回[教程总入口](../multi-agent-development-tutorial.md)。

## 开始前：确认你能完成到哪一步

前半篇只需要：

- Python 3.11 或更高版本；
- 能从网络安装 Python 包；
- 终端位于 `open-web-codex/` 仓库根目录。

最终让真实 Agent 调用 Tool，还需要已经运行的 open-web-codex 平台、可用的模型
Provider，以及真实 Codex 模式。**Provider** 是向 Codex 提供模型的服务；Fake
Runtime 只生成用于界面联调的模拟事件，不能证明 Skill 或 MCP Tool 被真实调用。
如果平台还没准备好，可以先完成前五步，再按
[本地运行手册](../mvp-runbook.md)准备平台。

先确认 Python 和当前目录：

```bash
python3 --version
git rev-parse --show-toplevel
```

第一条应显示 `Python 3.11` 或更高版本；第二条路径应以 `open-web-codex` 结尾。

然后准备这个示例自己的隔离环境：

```bash
tools/hello-agent/bin/setup-env
```

预期最后一行类似：

```text
Hello Agent environment is ready: .../tool-envs/hello-agent
```

这个脚本会：

1. 检查 Python 版本；
2. 在 `.local/open-web-codex/tool-envs/hello-agent` 创建虚拟环境；
3. 安装 `mcp`、`pydantic` 和示例维护测试所需的 `pytest`；
4. 把 `tools/hello-agent` 安装为可编辑的本地 Python 包。

它不会修改系统 Python。后续命令都明确使用这个虚拟环境。

## 第一步：先定义目标和边界

用户目标是：

> 向一个明确的人问好。

先把任务拆成 Agent 的工作和代码的工作：

| 谁负责 | 本例负责什么 | 真实业务中的对应例子 |
| --- | --- | --- |
| Agent | 理解用户是否在请求问候，缺少姓名时追问 | 理解“查订单是否超时”的意图 |
| Python | 校验姓名并返回固定结构的结果 | 查询订单、计算费用、写入系统 |

为什么不让模型直接生成结果？

本例中当然可以直接生成。这里故意用一件简单的事学习连接机制。真实项目中，以下能力
通常应该写成 Tool：

- 必须读取模型不知道的实时或私有数据；
- 必须写数据库、发消息、创建工单等产生真实副作用；
- 必须按照固定公式、权限或合规规则执行；
- 结果需要结构化、可测试、可审计。

“总结这段文字”“提出几个方案”通常适合由 Agent 完成；“查库存”“算运费”“提交
退款”通常需要 Tool。先划清这条边界，是开发 Agent 的第一步。

## 第二步：把业务规则写成普通 Python

打开 [`core.py`](../../tools/hello-agent/hello_agent/core.py)：

```python
from pydantic import BaseModel

MAX_NAME_CHARACTERS = 40


class Greeting(BaseModel):
    name: str
    message: str


def build_greeting(name: str) -> Greeting:
    clean_name = name.strip()
    if not clean_name:
        raise ValueError("name must not be empty")
    if len(clean_name) > MAX_NAME_CHARACTERS:
        raise ValueError("name must contain at most 40 characters")
    return Greeting(name=clean_name, message=f"你好，{clean_name}！")
```

这段代码还不是 Agent，也不是 Tool，只是普通 Python。

### `Greeting` 是什么

`Greeting` 是本例自己定义的返回数据结构，不是 Codex 或 MCP 规定的类名。你可以按
业务需要把它换成 `OrderStatus`、`ShippingQuote` 或其他名称。

它继承 Pydantic 的 `BaseModel`，表示结果必须有两个字符串字段：

```json
{
  "name": "小林",
  "message": "你好，小林！"
}
```

这里不用普通 `dict`，是因为明确的类型既能检查数据，也能在下一步帮助 MCP 生成
Tool 的输出 Schema。可以先把 `BaseModel` 理解成“带数据校验能力的 dataclass”。

设计自己的结果结构时，至少做到：

- 字段名表达业务含义；
- 字段类型明确；
- 不返回密码、内部堆栈或无关的大对象；
- 调用者不需要解析一段随意变化的自然语言才能使用结果。

### `build_greeting` 是什么

`build_greeting` 是实现规则的内部函数。`build_` 只是本例的命名习惯，不是框架要求。
它负责：

1. 删除姓名首尾空格；
2. 拒绝空姓名；
3. 拒绝超过 40 个字符的姓名；
4. 返回符合 `Greeting` 结构的结果。

先脱离 Agent 直接运行它：

```bash
.local/open-web-codex/tool-envs/hello-agent/bin/python -c \
  'from hello_agent.core import build_greeting; print(build_greeting(" 小林 ").model_dump())'
```

预期输出：

```text
{'name': '小林', 'message': '你好，小林！'}
```

这一步不是在“验证普通代码也能运行”这么简单。它先固定了三件事：

- Agent 将来可以传什么输入：姓名；
- 真正执行什么规则：清理、校验、生成；
- Agent 将来会收到什么结果：`name` 和 `message`。

如果这里算错，接上 Agent 只会把错误包装得更像正确答案。因此开发顺序应当始终是：
先让业务函数独立正确，再接协议和模型。

## 第三步：把允许调用的函数公开成 Tool

普通 Python 环境里可能有成百上千个函数。Agent 不应该任意执行它们；开发者必须明确
公开哪些操作、参数和结果是允许的。

打开 [`server.py`](../../tools/hello-agent/hello_agent/server.py)：

```python
from mcp.server.fastmcp import FastMCP

from .core import Greeting, build_greeting

mcp = FastMCP("Hello")


@mcp.tool()
def say_hello(name: str) -> Greeting:
    """Return one deterministic structured greeting for a named person."""

    return build_greeting(name)
```

这里发生了两件事：

1. `FastMCP("Hello")` 创建一个 MCP Server；
2. `@mcp.tool()` 把 `say_hello` 注册到这个 Server 的可调用 Tool 列表。

`say_hello` 本身仍然是 Python 函数。加上装饰器后，FastMCP 会读取它的函数名、说明、
`name: str` 参数和 `Greeting` 返回类型，并生成机器可读的输入与输出 Schema。
结构化结果来自 `Greeting` 返回类型，不需要额外的 JSON 开关。

**Schema** 在这里就是“有哪些字段、每个字段是什么类型、是否必填”的正式说明。

Codex 看到的调用大致是：

```text
Tool：say_hello
输入：name，字符串，必填
输出：Greeting，包含 name 和 message
```

所以它知道应该发送：

```json
{"name": "小林"}
```

### 为什么保留两层函数

```text
say_hello       Agent 能看到的边界
    ↓
build_greeting  真正实现规则的普通 Python
```

MCP 并不强制拆成两层。小型一次性 Tool 可以直接把规则写进 `say_hello`。本例拆成
`core.py` 和 `server.py`，是为了让变化各归其位：

| 变化 | 通常改哪里 | 为什么 |
| --- | --- | --- |
| 问候文字、姓名长度等业务规则 | `core.py` | 不需要碰 MCP |
| Agent 可以传哪些参数 | `server.py`，通常也同步改 `core.py` | Tool Schema 发生变化 |
| 改用别的通信框架 | `server.py` | 业务规则仍可复用 |
| 其他 Python 程序也要生成问候 | 直接调用 `build_greeting` | 不必启动 MCP |

这就是“修改问候规则时可以直接测试 Python；更换 Agent 或连接方式时不重写业务
函数”的具体含义。

## 第四步：启动 Server，并让 Codex 找到它

`@mcp.tool()` 只是在 Python 程序内部登记 Tool，还没有启动进程，也没有建立连接。
这和 Web 框架中“注册路由”与“启动 Web Server”是两件事相同。

[`server.py`](../../tools/hello-agent/hello_agent/server.py) 的后半段是：

```python
def main() -> None:
    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
```

`mcp.run(transport="stdio")` 启动 MCP Server。

**MCP** 是 Model Context Protocol 的缩写。对新手来说，可以先把它理解成 Agent 和
外部能力之间共同遵守的消息格式：Agent 用统一格式询问“有哪些 Tool”、发起调用并
接收结果，Tool 实现不需要知道模型内部怎样工作。

本例的 `stdio` 表示：

```text
Codex 启动本地 Python 进程
  ↔ 通过进程的标准输入、标准输出交换 MCP 消息
```

它不需要开放网络端口。MCP 协议消息占用标准输出，所以 Server 不应随意向标准输出
打印调试内容；日志应写到标准错误或专用日志。

Codex 还需要知道用什么命令启动这个进程。下面只展示
[`.mcp.json`](../../tools/hello-agent/.mcp.json) 的核心字段，实际文件还设置了启动
超时、Tool 超时、审批模式和允许传入的环境变量：

```json
{
  "mcpServers": {
    "hello": {
      "command": "./bin/hello-agent-launcher",
      "args": [],
      "cwd": "."
    }
  }
}
```

- `hello` 是 Codex 中使用的 MCP Server ID；
- `command` 是启动命令；
- `cwd` 表示从 Plugin 根目录解析相对路径。

[`hello-agent-launcher`](../../tools/hello-agent/bin/hello-agent-launcher) 会检查准备好的
虚拟环境，然后运行：

```text
python -m hello_agent.server
```

因此依赖由 `setup-env` 提前安装，不会在对话过程中临时下载。

到这里，完整的代码调用路径是：

```text
Codex
  → 根据 .mcp.json 启动 hello Server
  → 通过 MCP 调用 hello.say_hello
  → say_hello 调用 build_greeting
  → 返回 Greeting
```

### 这些文件是不是都必须

“必须”要看运行范围：

| 文件 | 在什么范围需要 | 作用 |
| --- | --- | --- |
| `hello_agent/server.py` | 作为 MCP Server 运行时需要 | 创建 Server、注册并启动 Tool |
| `.mcp.json` | 当前 Codex Plugin 接入需要 | 声明如何启动 Server |
| `.codex-plugin/plugin.json` | 当前仓库的源码能力发现需要 | 把 MCP 和 Skill 组成可发现的能力包 |
| `bin/hello-agent-launcher` | 本项目的隔离环境方案需要 | 用正确虚拟环境启动 Server |
| `hello_agent/core.py` | 推荐，不是 MCP 强制 | 分离业务规则与协议边界 |
| `skills/say-hello/SKILL.md` | 简单 Tool 可选 | 说明何时、怎样调用 |
| `tests/` | 运行时不需要 | 修改后防止规则和配置被破坏 |

如果你只写一个独立 FastMCP 程序，文件结构可以不同；如果希望它在当前
open-web-codex 源码开发路径中被发现，就要满足这里的 Plugin 声明。

## 第五步：用 Skill 说明“什么时候、怎样用”

Tool 说明它“能做什么”，但复杂任务还需要工作方法。例如查订单时要先校验订单号，
退款时要先确认权限，仓网规划时要先准备数据再计算方案。

这种提供给 Agent 的可复用工作方法叫 **Skill**。

本例的 [`SKILL.md`](../../tools/hello-agent/skills/say-hello/SKILL.md) 规定：

```text
用户要求向一个明确的人问好时：
1. 必须取得姓名，缺少时先询问；
2. 调用 hello.say_hello；
3. 使用 Tool 返回的 name 和 message；
4. Tool 失败时不能自己编造成功结果。
```

对一步就能完成的 `say_hello`，Tool 自己的说明已经足够清楚，Skill 不是技术上的
必需项。这里保留它，是为了演示两层职责：

```text
Tool：可以执行什么
Skill：何时执行、按什么顺序执行、失败时怎么办
```

最后，[`plugin.json`](../../tools/hello-agent/.codex-plugin/plugin.json) 指向
`.mcp.json` 和 `skills/`。**Plugin** 只是把相关能力打包给 Codex 发现；它不是一个
Agent。

## 第六步：让真实 Agent 调用 Tool

### 先分清 Task、Thread 和 Agent

open-web-codex 界面把一项持续工作称为 **Task（任务）**；它底层对应 Codex 的
**Thread**。这里可以把两者理解成同一件事：

> 一次任务的持续上下文，保存用户消息、Agent 回答和 Tool 调用记录。

**Agent** 是在这个上下文中理解目标、选择下一步并使用能力的执行者。你不会在本例
中创建一个 `Agent` Python 类。这里的“开发 Agent”是为 Codex Agent 设计目标、
Tool、Skill、输入输出和验证方式。

### 准备真实运行条件

如果平台尚未运行，先完成[本地运行手册](../mvp-runbook.md)。开始本步前确认：

- 使用 `./scripts/start-all.sh` 启动的是真实模式，不是 `--fake`；
- 浏览器中已经配置并选中可用的 Provider 和模型；
- 已选择一个授权工作区；
- `tools/hello-agent/bin/setup-env` 已成功运行。

工作区首页的模型菜单如果显示 “Connect this workspace to load available models”，说明
平台或 Provider 尚未准备好；此时先处理平台配置，不要把“没有模型”误判为 Tool
代码错误。

能力列表在新任务创建时交给 Codex，所以准备或修改 Plugin 后必须新建任务；已经打开
的旧任务不会自动获得新能力。

在工作区首页的输入框发送第一条消息，创建一个新任务。任务打开后：

1. 展开侧栏的 **MCP Servers**；
2. 确认 `hello` 状态为 `ready`；
3. 如果是 `error`，先查看页面错误，再检查是否运行过 `setup-env`。

### 发起一次明确调用

发送：

```text
请使用 $say-hello 向小林问好。
```

输入 `$say-hello` 时可以从自动补全中选择 Skill。若页面请求 Tool 审批，确认本次
调用。

完成后展开对话中的 **tool calls**，应看到：

```text
hello.say_hello
```

输入应为：

```json
{"name": "小林"}
```

结果应包含：

```json
{
  "name": "小林",
  "message": "你好，小林！"
}
```

如果只看到模型写出“你好，小林！”，却没有 `hello.say_hello` 调用，说明模型会问好，
但不能证明 Python Tool 已接通。

再发送：

```text
请使用 $say-hello 问好。
```

因为没有姓名，正确行为是先询问，而不是猜一个名字。这个失败路径证明 Skill 的输入
规则确实生效。

普通的“向小林问好”不一定触发 Tool，因为模型本身就能完成这件事；因此本篇用显式
Skill 验证连接。真实业务 Tool 应用“需要实时数据、确定计算或副作用”来证明调用价值，
而不是强迫 Agent 为任何一句话都调用工具。

## 第七步：亲手把能力改成“支持正式语气”

现在完成一次真正的 Agent 能力开发。新需求是：

> 用户明确要求“正式欢迎”时，返回“`姓名，欢迎加入项目。`”；其他情况仍返回原来的
> 友好问候。

先判断要改哪些层：

| 新需求影响 | 应改哪里 |
| --- | --- |
| 新增正式问候规则 | `core.py` |
| Agent 需要传入 `tone` 参数 | `server.py` 的 Tool 接口 |
| Agent 要知道何时选正式语气 | `SKILL.md` |
| 旧行为不能被破坏 | 直接检查，并更新测试 |

这一步会修改你的工作区。先改
[`core.py`](../../tools/hello-agent/hello_agent/core.py)：

```python
from typing import Literal

from pydantic import BaseModel

MAX_NAME_CHARACTERS = 40
GreetingTone = Literal["friendly", "formal"]


class Greeting(BaseModel):
    name: str
    message: str


def build_greeting(
    name: str,
    tone: GreetingTone = "friendly",
) -> Greeting:
    clean_name = name.strip()
    if not clean_name:
        raise ValueError("name must not be empty")
    if len(clean_name) > MAX_NAME_CHARACTERS:
        raise ValueError("name must contain at most 40 characters")
    if tone not in ("friendly", "formal"):
        raise ValueError("tone must be friendly or formal")

    message = (
        f"{clean_name}，欢迎加入项目。"
        if tone == "formal"
        else f"你好，{clean_name}！"
    )
    return Greeting(name=clean_name, message=message)
```

`Literal["friendly", "formal"]` 表示这个参数只允许两个明确值。普通 Python 函数仍
保留显式检查；FastMCP 则用这个类型把可选值写进 Tool Schema。

再把 [`server.py`](../../tools/hello-agent/hello_agent/server.py) 中的导入和 Tool
改为：

```python
from .core import Greeting, GreetingTone, build_greeting


@mcp.tool()
def say_hello(
    name: str,
    tone: GreetingTone = "friendly",
) -> Greeting:
    """Return a friendly or formal structured greeting for one named person."""

    return build_greeting(name, tone)
```

最后在 [`SKILL.md`](../../tools/hello-agent/skills/say-hello/SKILL.md) 中加入规则：

```text
用户明确要求“正式”或“商务”语气时传 tone="formal"；
其他情况传 tone="friendly"。
```

先不启动 Agent，直接检查新规则：

```bash
.local/open-web-codex/tool-envs/hello-agent/bin/python -c \
  'from hello_agent.core import build_greeting; print(build_greeting("小林", "formal").model_dump())'
```

预期：

```text
{'name': '小林', 'message': '小林，欢迎加入项目。'}
```

现有测试会继续保护默认的友好语气。再在
[`test_core.py`](../../tools/hello-agent/tests/test_core.py) 中增加一个正式语气用例：

```python
def test_builds_formal_message() -> None:
    greeting = build_greeting("小林", "formal")

    assert greeting == Greeting(name="小林", message="小林，欢迎加入项目。")
```

测试不是接入 Agent 的先决条件；但当你决定保留一次改动时，它能防止以后修改破坏
友好语气或正式语气。

运行：

```bash
.local/open-web-codex/tool-envs/hello-agent/bin/python \
  -m pytest tools/hello-agent/tests -q
```

如果测试失败，先根据失败信息更新预期，再继续。不要为了得到绿色结果删除原有规则
检查。

然后新建任务，确认侧栏 `hello` 为 `ready`，发送：

```text
请使用 $say-hello 正式欢迎小林加入项目。
```

调用记录应包含：

```json
{
  "name": "小林",
  "tone": "formal"
}
```

这次改造完成了一个可复用闭环：

```text
新业务目标
  → 修改确定性规则
  → 修改 Tool 输入合同
  → 修改 Agent 的使用方法
  → 先验证 Python
  → 新任务加载新能力
  → 检查真实调用轨迹
```

## 以后想实现别的目标，应该改哪里

| 你想改变什么 | 优先修改 | 例子 |
| --- | --- | --- |
| 计算、校验或数据处理规则 | `core.py` 等普通业务代码 | 运费公式、库存阈值 |
| Agent 能执行的新操作 | 新增或修改 `@mcp.tool()` | `get_order`、`create_ticket` |
| Tool 的输入或输出字段 | Python 类型和 Tool 签名 | 新增 `currency`、`status` |
| 何时调用、调用顺序、失败处理 | `SKILL.md` | 退款前先查订单再验权限 |
| 一次任务的具体目标 | 用户请求或上层任务的委派 | “只分析华东近 30 天订单” |
| 长期可复用的专业职责 | Skill + 一组相关 Tool + 明确交付合同 | 订单查询、退款审核 |
| 独立上下文、权限或失败重试 | 新的子任务 / 子 Thread | 数据准备与网络规划分开 |

不要因为新增一个函数就创建一个 Agent，也不要因为想换一句提示词就新建 MCP Server。
只有当一项职责需要独立目标、上下文、权限、交付物或失败状态时，才考虑拆成另一个
Agent。第二篇会从真实仓网场景解释这个判断。

## 从零开发自己的 Agent 能力

把问候示例换成订单、客服或数据分析场景时，可以直接复用下面的顺序：

1. 写一条真实用户请求，并写清成功结果和正确失败；
2. 列出 Agent 负责的判断，以及代码必须执行的事实、计算或副作用；
3. 用类型定义输入输出，先实现不依赖 MCP 的普通 Python；
4. 直接运行正常、边界和失败输入；
5. 用一个小的 `@mcp.tool()` 函数公开允许调用的能力；
6. 用 `.mcp.json` 声明启动方式，并通过 Plugin 让当前项目发现；
7. 只有存在触发条件、多步顺序或失败策略时，再写 Skill；
8. 新建真实任务，检查 MCP 状态、Tool 参数、结果和失败行为；
9. 为准备长期保留的规则补测试；
10. 只有出现独立职责边界时，再设计子任务和上层协调者。

一个专业 Agent 的最小职责说明可以写成：

```text
目标：它最终要交付什么
输入：允许接收哪些数据或引用
输出：必须返回什么结构和证据
Tool：哪些事实或动作必须调用代码
停止条件：缺什么、错什么时不能继续
```

这五项先写清楚，再决定文件和框架，通常比先创建一个 `agent.py` 更接近真正的 Agent
开发。

在当前仓库中新建一个同类能力包时，最小目录通常是：

```text
tools/your-capability/
├── .codex-plugin/plugin.json   Codex 发现入口
├── .mcp.json                   MCP Server 启动声明
├── pyproject.toml              Python 版本和依赖
├── bin/
│   ├── setup-env               提前准备隔离环境
│   └── launcher                启动 Server
├── your_package/
│   ├── core.py                 普通业务规则
│   └── server.py               @mcp.tool() 与 mcp.run()
└── skills/                     有可复用工作方法时再添加
```

目录名和 Python 包名可以按领域修改，职责关系不要颠倒。

## 可选：什么时候做 MCP 协议检查

如果 Python 函数正确、页面里的 `hello` 却无法 `ready`，可以运行仓库预先提供的
协议检查：

```bash
.local/open-web-codex/tool-envs/hello-agent/bin/python \
  tools/hello-agent/tests/stdio_smoke.py
```

预期最后一行：

```text
Hello Agent stdio smoke passed
```

[`stdio_smoke.py`](../../tools/hello-agent/tests/stdio_smoke.py) 是示例作者写的最小 MCP
Client：它启动 Server、列出 Tool、调用 `say_hello` 并检查结果。它不是 MCP 自动
生成的文件，也不是第一次开发 Tool 必须创建的文件。只有在排查“业务函数正常，但
MCP 连接不通”时才需要它。

| 现象 | 先检查什么 |
| --- | --- |
| `setup-env` 报 Python 版本错误 | 安装或选择 Python 3.11+ |
| `setup-env` 在安装依赖时失败 | 网络、Python 包源和错误末尾 |
| 侧栏没有 `hello` | 是否在准备环境后新建了任务 |
| `hello` 是 `error` | 先运行 `stdio_smoke.py`，再查看页面给出的 Server 错误 |
| 回答正确但没有 Tool 调用 | 使用显式 `$say-hello`，并检查展开后的 tool calls |

## 回顾与完成标志

本篇没有训练模型，也没有创建一个 `Agent` 类。你给现有 Codex Agent 增加了一项
受控 Python 能力，并知道以后如何演进它：

```text
用户
  → 任务中的 Agent
  → 直接选择 Tool，或在需要时参考 Skill
  → hello.say_hello
  → build_greeting
  → 结构化结果
```

- [ ] 能说明为什么先定义业务目标和 Agent/代码边界；
- [ ] 能解释 `Greeting` 没有框架规定的特殊命名；
- [ ] 能解释 `build_greeting`、`say_hello` 和 `@mcp.tool()` 的关系；
- [ ] 能解释注册 Tool、启动 MCP Server、让 Codex 发现 Server 是三件事；
- [ ] 能解释 Tool 与 Skill 的不同；
- [ ] 真实任务中出现 `hello.say_hello` 调用，而不只是模型生成文字；
- [ ] 能根据变化判断应该修改业务代码、Tool、Skill 还是 Agent 职责；
- [ ] 能完成一次规则、Tool 合同、Skill 和验证同步变化的改造。

下一篇进入真实业务：

[第二篇：把仓网问题拆成数据与规划两项职责](hello-agent-team.md)
