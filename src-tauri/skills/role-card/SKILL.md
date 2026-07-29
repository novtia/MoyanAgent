---
id: role-card
name: 角色卡设计
description: 引导 Agent 用 RoleState 创建或更新角色状态板：字段约定、属性结构、外貌与性别必填规则。
author: Lumen
version: 1.2.0
tags: [创作, 角色, RoleState]
when_to_use: 用户要求设计角色、生成角色卡、或更新某人在状态板上的结构化字段时使用。
---

# 角色卡设计

在 Agent 模式下引用本技能后，你必须通过 `RoleState` 工具把自然语言需求落成**结构化角色卡**。不要只写散文设定；状态板才是权威。

## 工作流

1. 先 `RoleState` `get`，看板上已有哪些角色。
2. 新角色 → `create`（稳定小写 ascii `id`，如 `rin`）。
3. 已有角色 → `update`，只用 `set` / `unset` 改**变更字段**，禁止整板重发。

## 必填与推荐字段

- `id`：稳定小写 ascii
- `name`、`gender`（`male` | `female`，**create 必填**）
- `appearance`：外貌概述，≤100 汉字；含体型与性别相关体征
- `location` / `mood` / `outfit`：短文本
- `attributes`：0–100 整数（好感、信任…）→ 雷达图
- `meters`：`{ value, max }`（体力、理智…）→ 进度条
- `tags`：短标签数组
- `nsfw`：按 RoleState 工具说明维护（英文 key）
- 跑团可选：`persona`、`goals`、`speech_style`、`control`（`ai` | `user`）、`memory_path`、`model`（该角色征询用的模型 id，空则用默认）

## 增量原则

只写变化；不要复述未改字段。角色永久离场才 `delete`。

## 输出

工具调用完成后，用一两句中文向用户确认：创建了谁 / 更新了哪些字段。
