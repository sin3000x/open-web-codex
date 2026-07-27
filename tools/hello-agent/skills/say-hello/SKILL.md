---
name: say-hello
description: Use the hello MCP server to generate one deterministic structured greeting for a named person. Use when a user asks to greet, welcome, or say hello to one person.
---

# Say Hello

1. Require one explicit person's name. Ask for it when missing; never guess.
2. Call `hello.say_hello` exactly once with that name.
3. Return the structured `name` and `message` from the Tool.
4. If the Tool rejects the input or is unavailable, report the failure instead of composing a
   substitute greeting.
