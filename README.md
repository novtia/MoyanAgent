# Atelier · gpt-image-2

Local-first desktop application for OpenRouter-compatible image generation and
editing chat. Built on **Tauri 2 + React + TypeScript** (frontend) and **Rust**
(backend with SQLite, file-system image store, local image editor, OpenRouter
client).

## Features

- Multi-session chat with persistent history (SQLite).
- Generate images from text prompts via OpenRouter `chat/completions`
  (`image_config` aspect ratio + size).
- Image editing: drop in reference images and ask the model to remix.
- Local image tools (no API call):
  - Crop (free / 1:1 / 4:3 / 3:4 / 16:9 / 9:16)
  - Transform (rotate 90/180/270, flip H/V, resize)
  - Mask (paint to remove regions; outputs PNG with transparency)
- Image preview overlay with wheel-zoom and pan.
- API key, endpoint and model are stored locally in SQLite — never leave the
  machine.
- Drag & drop / paste / file picker for attachments (PNG / JPEG / WebP, ≤ 50
  MB, ≤ 8 per message).

## Layout

```
gpt-image2/
├── src/                    React + TS frontend
│   ├── api/tauri.ts        invoke wrappers
│   ├── store/              Zustand stores (settings, session)
│   ├── components/         UI: Sidebar, Chat, Composer, Editor, Preview
│   └── styles/             design tokens + modular CSS (globals.css → modules/*)
└── src-tauri/              Rust backend
    ├── migrations/         SQLite schema
    └── src/
        ├── lib.rs          AppState + Tauri commands
        ├── db.rs           r2d2 + rusqlite pool
        ├── settings.rs     settings table
        ├── session.rs      sessions / messages / message_images
        ├── images.rs       attachment + thumbnail + output storage
        ├── editor.rs       image-crate based local edits
        ├── openrouter.rs   reqwest chat/completions client
        └── paths.rs        app data layout helpers
```

## Data lives in `<app_data>/atelier`

```
atelier/
├── atelier.db
└── sessions/<session_id>/
    ├── in/<id>.<ext>      uploaded reference images
    ├── out/<id>.<ext>     generated images
    ├── edit/<id>.<ext>    local-edit results
    └── thumb/<id>.webp    thumbnails for the strip
```

## Develop

Prereqs: **Rust toolchain** (1.77+), **Node.js 18+**, **npm**, plus the
[Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) (on
Windows: WebView2 — usually already installed).

```bash
npm install
npm run tauri:dev
```

The first `cargo build` will be slow (compiles SQLite + reqwest + image
toolchain). Subsequent runs are incremental.

## Build a desktop bundle

```bash
npm run tauri:build
```

Bundle outputs land in `src-tauri/target/release/bundle/`. The bundled icons
under `src-tauri/icons/` are minimal placeholders — replace them with your own
artwork (or run `npx tauri icon path/to/source.png` to regenerate) before
shipping.

## First-run setup inside the app

1. Open the **Settings** panel (left sidebar).
2. Paste your OpenRouter API key. Endpoint and model are pre-filled with
   `https://openrouter.ai/api/v1/chat/completions` and
   `openai/gpt-5.4-image-2`.
3. Pick the default aspect ratio and image size.
4. Click **新建会话** to start a session, type a prompt, hit Enter.

## Notes on the OpenRouter contract

Requests are POSTed by the Rust backend, not the WebView. The body is

```json
{
  "model": "<model>",
  "modalities": ["image", "text"],
  "messages": [{ "role": "user", "content": "<prompt or array>" }],
  "image_config": { "aspect_ratio": "...", "image_size": "..." }
}
```

with `content` upgraded to a `[ {type:"text"}, {type:"image_url"} ... ]` array
when there are reference attachments. Response parsing tries
`message.images[].image_url.url`, then `message.content[]`, then any inline
`data:image/<fmt>;base64,...` URL in the assistant text.
