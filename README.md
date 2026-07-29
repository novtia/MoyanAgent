# Lumen

本地优先的 AI 工作台。基于 **Tauri 2 + React + TypeScript**（前端）与 **Rust**（后端：SQLite、文件系统、多供应商 LLM 客户端）。

会话、媒体、密钥与配置均保存在本机；请求由 Rust 后端发出，不经过 WebView。

## 能力概览

- **多会话对话**：历史持久化；流式回复；消息编辑 / 重发 / 引用；附件拖拽、粘贴与选择；`@` 引用项目文件。
- **Ask / Agent / Plan**：绑定项目后可切换模式——问答与联网、可读写工作区的完整 Agent、只读探索与规划。
- **Agent 工具链**：读写文件、Shell、Grep、Todo、AskUser、WebSearch / WebFetch、文档创建与编辑、角色状态等；支持自定义 Agent 与流程串联。
- **项目工作区**：本地文件夹绑定、项目级提示词与模型参数、右侧文档阅读器（文件树、查找替换、diff 确认）。
- **图像与视频**：文生图 / 参考图 remix、本地裁剪与遮罩编辑；具备 video 能力的模型可文生视频、首帧 / 首尾帧与多模态参考。
- **多供应商 LLM**：添加 OpenAI 兼容端点（OpenRouter、OpenAI、Anthropic 等）；按能力标记模型；会话内切换模型与思考强度。
- **搜索**：`Ctrl+K` 跨会话全文检索（含正文、思考与工具输出）；会话内精读查找；可配置本地或 API 联网搜索。
- **备份与用量**：模块化 ZIP 备份 / 恢复；Token 与费用趋势统计。
- **外观**：多主题、强调色、字体与布局密度；中 / 英界面。

侧边栏中的 Skills / Plugins / Automations 目前为占位，尚未启用。

## 仓库结构

```
Lumen/
├── src/                         React + TypeScript 前端
│   ├── api/tauri.ts             Tauri invoke 封装
│   ├── store/                   Zustand 状态
│   ├── components/              侧边栏、会话、Composer、设置、阅读器等
│   └── styles/                  设计 token + 模块化 CSS
└── src-tauri/                   Rust 后端
    ├── migrations/              SQLite schema
    └── src/
        ├── app/                 Tauri commands、生成流、会话 API
        ├── ai/                  Agent、工具、多供应商客户端
        └── data/                SQLite、路径、备份与传输
```

## 数据目录

应用数据位于 `<app_data>/atelier`（历史目录名，产品现为 Lumen）：

```
atelier/
├── atelier.db
└── sessions/<session_id>/
    ├── in/      上传的参考媒体
    ├── out/     生成结果
    ├── edit/    本地编辑结果
    └── thumb/   缩略图
```

## 开发

依赖：**Rust**（1.77+）、**Node.js 18+**、**npm**，以及 [Tauri 2 系统前置条件](https://v2.tauri.app/start/prerequisites/)（Windows 通常已自带 WebView2）。

```bash
npm install
npm run tauri:dev
```

首次 `cargo build` 较慢（编译 SQLite、reqwest、image 等），之后为增量编译。

## 打包

```bash
npm run tauri:build
```

产物在 `src-tauri/target/release/bundle/`。发布前请替换 `src-tauri/icons/`（或执行 `npx tauri icon path/to/source.png` 重新生成）。

## 首次使用

1. 打开侧边栏 **设置**。
2. 在「模型服务」中添加并启用至少一个供应商与模型。
3. （可选）配置联网搜索来源。
4. **新建会话**，按需绑定项目，开始对话。

生图 / 生视频取决于所选模型的能力标记；OpenAI 兼容的 `chat/completions` 或厂商专用端点由后端按供应商适配。
