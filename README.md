# Local AI

A local AI-powered developer workspace built with Tauri, React, TypeScript, Rust, and LM Studio.

Local AI is designed to help developers understand, analyze, and modify their projects using locally running AI models.

## Features

- Local AI chat powered by LM Studio
- Streaming AI responses
- Local model selection
- Persistent projects
- Persistent chat history
- Attach real project folders
- Recursive project file scanning
- Project file tree
- Project Intelligence
- Relevant-file selection
- Real project-file reading
- AI project analysis
- AI-generated file edits
- AI-generated new files
- Approval before applying changes
- Safe project-file writing
- Path traversal protection
- Undo for existing-file edits
- Simulation preview for proposed changes
- Integrated developer workspace UI

## Architecture

```text
Local AI
│
├── frontend/
│   ├── React
│   ├── TypeScript
│   ├── Vite
│   └── LM Studio integration
│
└── src-tauri/
    ├── Rust
    └── Tauri native backend
