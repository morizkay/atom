# Atom GPUI + Pi Coding Agent

**Atom**, refactored from Electron/JavaScript into a native GPU-accelerated **Rust + GPUI** desktop code editor with the **Pi Coding Agent** built directly in.

## Architecture

This repository is a Cargo workspace split into four focused crates:

| Crate | Purpose |
| --- | --- |
| `crates/atom-theme` | Authentic Atom **One Dark** (`#282c34` / `#21252b`) and **One Light** UI + syntax color palettes for GPUI. |
| `crates/atom-editor` | Rope-backed (`ropey`) text buffer, cursor navigation, undo/redo history, file I/O, and fast syntax tokenizer (Rust, TypeScript, JavaScript, Python, JSON, TOML, Markdown, Shell, C/C++, Go). |
| `crates/pi-agent` | Built-in **Pi Coding Agent**: native Rust 4-tool core (`read`, `write`, `edit`, `bash`), under-1k-token system prompt, tree-structured JSONL sessions (`/tree`), multi-provider streaming LLM client (Anthropic, OpenAI, OpenRouter, Ollama), and an optional `pi --mode rpc` JSONL subprocess bridge. |
| `crates/atom-app` | GPUI desktop binary (`atom`) assembling the Tree View, multi-tab syntax-highlighted editor, built-in Pi Coding Agent dock, Command Palette, and Status Bar. |

## Multi-Project Nested Top Tabs (JetBrains-Style)

Atom GPUI supports working across multiple projects inside a single window:

- **Outer Top Tab Bar**: Each open project gets a top-level project tab displaying its name, Git branch, and live Pi agent status (`π ●` running, `π ↻` interrupted/resumable, `π` idle).
- **Project-Scoped Sidebar**: The left file tree sidebar is scoped to the active project and switches automatically as you switch project tabs.
- **Nested File Tab Bar**: Each project maintains its own inner file tabs, cursor positions, and open buffers.
- **Concurrent Durable Pi Agents**: Each project runs its own isolated durable Pi agent under `<project>/.pi/` — background project tabs keep streaming and auto-reloading modified files while you work in another project.
- Open multiple projects at launch (`cargo run --bin atom -- /path/to/projA /path/to/projB`) or click **+ Project** (`Ctrl+Shift+O` / `Cmd+Shift+O`).

## Built-in Durable Pi Coding Agent

Pi is integrated directly into the right-hand dock (`Ctrl+Shift+A` / `Cmd+Shift+A`) and supports two interchangeable backends:

1. **Native Rust Durable Core (default, zero Node.js dependency)**
   - Implements Pi's 4 core tools: `read`, `write`, `edit`, `bash`
   - **Durable atomic checkpointing**: Persists every turn and tool execution atomically to `<project>/.pi/state.json` and `<project>/.pi/sessions/<id>.jsonl`
   - **Crash recovery & `/resume`**: If a run is aborted, fails transiently (with automatic exponential-backoff retries), or Atom exits mid-run, reopening restores the session and offers one-click **Resume** (`/resume`)
   - **Session browser (`/sessions`) & tree branching (`/tree`)**: Browse and reload any prior persisted session or branch from any historical turn
   - Automatically reloads open editor tabs when Pi modifies a file
   - Streams responses from **Anthropic** (`ANTHROPIC_API_KEY`), **OpenAI** (`OPENAI_API_KEY`), **OpenRouter** (`OPENROUTER_API_KEY`), or local **Ollama** (`http://localhost:11434/v1`)
   - Reads credentials from environment variables or `~/.pi/agent/auth.json`

2. **External Pi CLI RPC Bridge (`pi --mode rpc`)**
   - Click the backend chip (`Native Rust` → `Pi CLI (RPC)`) or run `/backend`
   - Spawns `@earendil-works/pi-coding-agent` in `--mode rpc` using strict LF-only JSONL framing so you can use your full Pi TypeScript extensions, skills, and packages directly inside Atom.

### Pi Slash Commands

- `/resume` — Resume an interrupted or failed durable run from the last checkpoint
- `/sessions` — Browse & load persisted `.pi/sessions/` history
- `/tree` — Toggle the session tree drawer and click any prior node to branch from it
- `/new` — Start a new durable session tree
- `/model <id>` — Switch the active model ID
- `/provider` — Cycle between Anthropic, OpenAI, OpenRouter, and Ollama
- `/backend` — Switch between Native Rust and external `pi --mode rpc`
- `/clear` — Clear the chat display
- `/help` — Show available commands

## Keyboard Shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+Shift+O` / `Cmd+Shift+O` | Open Project in New Top Tab |
| `Ctrl+Alt+Right` / `Ctrl+Alt+Left` | Next / Previous Project Tab |
| `Ctrl+Shift+P` / `Cmd+Shift+P` | Toggle Command Palette |
| `Ctrl+Shift+A` / `Cmd+Shift+A` | Toggle Pi Coding Agent Panel |
| `Ctrl+\` / `Cmd+\` | Toggle Project Sidebar (Tree View) |
| `Ctrl+S` / `Cmd+S` | Save Active File |
| `Ctrl+N` / `Cmd+N` | New File Tab |
| `Ctrl+W` / `Cmd+W` | Close Active File Tab |
| `Ctrl+Z` / `Cmd+Z` | Undo |
| `Ctrl+Shift+Z` / `Ctrl+Y` | Redo |
| `Ctrl+Shift+K` | Delete Line |
| `Esc` | Close Overlay / Return Focus to Editor |

## Building & Running

### Linux Dependencies (Ubuntu/Debian)

```bash
sudo apt-get install -y \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
  libxcb1-dev libxcb-shape0-dev libxcb-xfixes0-dev libxcb-render0-dev \
  libasound2-dev libfontconfig1-dev libfreetype-dev pkg-config cmake
```

### Run Atom GPUI

```bash
# Open current directory
cargo run --bin atom

# Open a specific project directory or file
cargo run --bin atom -- /path/to/project
```

### Run Tests

```bash
cargo test --workspace
```

## License

MIT — see [LICENSE.md](LICENSE.md).
