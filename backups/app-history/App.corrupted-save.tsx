import {
  Bot,
  ChevronDown,
  ChevronRight,
  Circle,
  FileCode2,
  Folder,
  FolderOpen,
  GitBranch,
  History,
  Pause,
  Play,
  Plus,
  Search,
  Settings,
  Square,
  Terminal,
  User,
  Wrench,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";
import { chatWithModel, getModels, type AIModel } from "./services/ai";


import {
  createProject,
  getProjects,
  listProjectFiles,
  readProjectFile,
  saveProject,
  type ProjectFile,
  type StoredProject,
} from "./services/projectStore";




interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export default function App() {
  const [models, setModels] = useState<AIModel[]>([]);
  const [selectedModel, setSelectedModel] = useState("qwen/qwen3.5-9b");

  const [projects, setProjects] = useState<StoredProject[]>([]);
  const [selectedProject, setSelectedProject] =
    useState<StoredProject | null>(null);

  const [projectFiles, setProjectFiles] = useState<ProjectFile[]>([]);

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [modelsLoading, setModelsLoading] = useState(true);
  const [projectLoading, setProjectLoading] = useState(true);

  const chatEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    loadModels();
    loadProjects();
  }, []);

  useEffect(() => {
    chatEndRef.current?.scrollIntoView({
      behavior: "smooth",
    });
  }, [messages, loading]);

  async function loadModels() {
    setModelsLoading(true);

    try {
      const availableModels = await getModels();

      setModels(availableModels);

      if (
        availableModels.length > 0 &&
        !availableModels.some(
          (model) => model.id === selectedModel,
        )
      ) {
        setSelectedModel(availableModels[0].id);
      }
    } catch (err) {
      console.error(err);
      setError("LM Studio is not available.");
    } finally {
      setModelsLoading(false);
    }
  }

  async function loadProjects() {
    setProjectLoading(true);

    try {
      const storedProjects = await getProjects();

      setProjects(storedProjects);

      if (storedProjects.length > 0) {
        await selectProject(storedProjects[0]);
      }
    } catch (err) {
      console.error(err);
      setError(
        err instanceof Error
          ? err.message
          : "Failed to load projects.",
      );
    } finally {
      setProjectLoading(false);
    }
  }

  async function selectProject(project: StoredProject) {
    setSelectedProject(project);
    setMessages(project.messages ?? []);
    setProjectFiles([]);

    if (project.folder_path) {
      try {
        const files = await listProjectFiles(project);
        setProjectFiles(files);
      } catch (err) {
        console.error("Failed to scan project:", err);

        setError(
          err instanceof Error
            ? err.message
            : "Failed to scan project folder.",
        );
      }
    }
  }

  async function createNewProject() {
    setError("");

    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Project Folder",
      });

      if (!selected || Array.isArray(selected)) {
        return;
      }

      const folderPath = selected;

      const folderName =
        folderPath.split("/").filter(Boolean).pop() ??
        "New Project";

      const now = new Date().toISOString();

      const project: StoredProject = {
        id: crypto.randomUUID(),
        name: folderName,
        created_at: now,
        updated_at: now,
        folder_path: folderPath,
        messages: [],
      };

      const savedProject = await createProject(
        project.id,
        project.name,
        project.created_at,
      );

      const attachedProject: StoredProject = {
        ...savedProject,
        folder_path: folderPath,
        updated_at: now,
        messages: [],
      };

      await saveProject(attachedProject);

      setProjects((previous) => [
        ...previous,
        attachedProject,
      ]);

      await selectProject(attachedProject);
    } catch (err) {
      console.error("Failed to create project:", err);

      setError(
        err instanceof Error
          ? err.message
          : "Failed to create project.",
      );
    }
  }


```tsx
async function buildProjectContext(
  text: string,
): Promise<string> {
  if (!selectedProject || projectFiles.length === 0) {
    return "";
  }

  const query = text.toLowerCase();

  /*
   * =========================================================
   * SMART PROJECT CONTEXT ENGINE
   * =========================================================
   *
   * Goal:
   * Select only the project files that are most useful for
   * answering the user's question.
   *
   * Priority:
   * 1. Explicitly mentioned files
   * 2. Files matching the user's keywords
   * 3. Files related to the detected intent
   * 4. Important project entry/configuration files
   * 5. Source files
   */

  const stopWords = new Set([
    "the",
    "and",
    "that",
    "this",
    "with",
    "from",
    "what",
    "where",
    "when",
    "which",
    "how",
    "why",
    "does",
    "do",
    "is",
    "are",
    "can",
    "could",
    "would",
    "should",
    "please",
    "tell",
    "show",
    "give",
    "make",
    "fix",
    "add",
    "change",
    "remove",
    "update",
    "create",
    "project",
    "code",
    "file",
    "files",
    "thing",
    "something",
    "about",
    "for",
    "into",
    "inside",
    "using",
    "use",
    "want",
    "need",
    "doesnt",
    "don't",
    "not",
  ]);

  const queryWords = query
    .replace(/[^a-z0-9_./-]/g, " ")
    .split(/\s+/)
    .filter(
      (word) =>
        word.length >= 2 &&
        !stopWords.has(word),
    );

  /*
   * =========================================================
   * INTENT DETECTION
   * =========================================================
   */

  const intents = {
    build: [
      "build",
      "compile",
      "compilation",
      "typescript",
      "tsc",
      "vite",
      "gradle",
      "cargo",
      "npm",
      "yarn",
      "pnpm",
      "error",
      "errors",
      "failed",
      "failure",
      "broken",
      "crash",
      "warning",
    ],

    ui: [
      "ui",
      "interface",
      "frontend",
      "screen",
      "page",
      "component",
      "button",
      "layout",
      "design",
      "style",
      "css",
      "tailwind",
      "react",
    ],

    backend: [
      "backend",
      "server",
      "api",
      "endpoint",
      "route",
      "database",
      "db",
      "sql",
      "rust",
      "fastapi",
      "express",
      "node",
    ],

    auth: [
      "login",
      "logout",
      "authentication",
      "authorization",
      "auth",
      "password",
      "token",
      "jwt",
      "session",
      "signup",
      "register",
      "account",
    ],

    project: [
      "architecture",
      "structure",
      "overview",
      "project",
      "work",
      "working",
      "flow",
      "system",
      "how",
    ],

    testing: [
      "test",
      "testing",
      "jest",
      "vitest",
      "pytest",
      "spec",
      "unit",
      "integration",
    ],

    config: [
      "config",
      "configuration",
      "environment",
      "env",
      "dependency",
      "dependencies",
      "package",
      "settings",
    ],

    documentation: [
      "readme",
      "documentation",
      "docs",
      "explain",
      "document",
    ],

    security: [
      "security",
      "secure",
      "vulnerability",
      "permission",
      "permissions",
      "secret",
      "secrets",
      "credential",
      "credentials",
    ],
  };

  /*
   * =========================================================
   * INTENT SCORES
   * =========================================================
   */

  const detectedIntents = new Set<string>();

  for (const [intent, keywords] of Object.entries(intents)) {
    const matched = keywords.some((keyword) =>
      query.includes(keyword),
    );

    if (matched) {
      detectedIntents.add(intent);
    }
  }

  /*
   * =========================================================
   * FILE SCORING
   * =========================================================
   */

  const importantFiles = new Set([
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "cargo.toml",
    "cargo.lock",
    "tauri.conf.json",
    "tauri.conf.json5",
    "vite.config.ts",
    "vite.config.js",
    "vite.config.mts",
    "vite.config.mjs",
    "tsconfig.json",
    "tsconfig.app.json",
    "tsconfig.node.json",
    "readme.md",
    "main.rs",
    "lib.rs",
    "main.ts",
    "main.tsx",
    "index.ts",
    "index.tsx",
    "app.tsx",
    "app.vue",
    "dockerfile",
    "docker-compose.yml",
    "docker-compose.yaml",
  ]);

  const sourceExtensions = new Set([
    "ts",
    "tsx",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "rs",
    "py",
    "java",
    "kt",
    "kts",
    "go",
    "cpp",
    "c",
    "h",
    "hpp",
    "cs",
    "swift",
    "vue",
    "svelte",
  ]);

  const configExtensions = new Set([
    "json",
    "json5",
    "toml",
    "yaml",
    "yml",
    "ini",
    "env",
  ]);

  const documentationExtensions = new Set([
    "md",
    "mdx",
    "txt",
  ]);

  const scoredFiles = projectFiles
    .filter((file) => !file.is_directory)
    .map((file) => {
      const fileName = file.name.toLowerCase();
      const filePath = file.path.toLowerCase();

      const fileNameWithoutExtension =
        fileName.includes(".")
          ? fileName.substring(
              0,
              fileName.lastIndexOf("."),
            )
          : fileName;

      const extension =
        fileName.split(".").pop() ?? "";

      let score = 0;

      /*
       * -------------------------------------------------------
       * 1. EXACT FILE/PATH MENTION
       * -------------------------------------------------------
       */

      if (query.includes(filePath)) {
        score += 1500;
      }

      if (query.includes(fileName)) {
        score += 1300;
      }

      if (
        fileNameWithoutExtension.length > 1 &&
        query.includes(fileNameWithoutExtension)
      ) {
        score += 900;
      }

      /*
       * -------------------------------------------------------
       * 2. QUERY WORD MATCHING
       * -------------------------------------------------------
       */

      for (const word of queryWords) {
        if (fileName === word) {
          score += 600;
        }

        if (fileNameWithoutExtension === word) {
          score += 500;
        }

        if (fileName.includes(word)) {
          score += 220;
        }

        if (filePath.includes(word)) {
          score += 140;
        }

        if (
          fileNameWithoutExtension.includes(word)
        ) {
          score += 160;
        }
      }

      /*
       * -------------------------------------------------------
       * 3. IMPORTANT PROJECT FILES
       * -------------------------------------------------------
       */

      if (importantFiles.has(fileName)) {
        score += 30;
      }

      /*
       * -------------------------------------------------------
       * 4. INTENT-BASED FILE MATCHING
       * -------------------------------------------------------
       */

      const intentRules: Record<
        string,
        string[]
      > = {
        build: [
          "package",
          "tsconfig",
          "vite",
          "cargo",
          "gradle",
          "webpack",
          "babel",
          "rollup",
          "build",
          "config",
        ],

        ui: [
          "component",
          "page",
          "screen",
          "layout",
          "view",
          "ui",
          "style",
          "css",
          "tsx",
          "jsx",
          "vue",
          "svelte",
        ],

        backend: [
          "server",
          "api",
          "route",
          "controller",
          "service",
          "backend",
          "db",
          "database",
          "model",
          "repository",
          "rust",
        ],

        auth: [
          "auth",
          "login",
          "logout",
          "register",
          "signup",
          "session",
          "user",
          "token",
          "jwt",
          "password",
        ],

        testing: [
          "test",
          "spec",
          "__tests__",
        ],

        config: [
          "config",
          "settings",
          "env",
          "package",
          "tsconfig",
        ],

        documentation: [
          "readme",
          "docs",
          "documentation",
        ],

        security: [
          "auth",
          "security",
          "permission",
          "token",
          "secret",
          "credential",
        ],

        project: [
          "main",
          "index",
          "app",
          "package",
          "readme",
          "config",
        ],
      };

      for (const intent of detectedIntents) {
        const keywords =
          intentRules[intent] ?? [];

        for (const keyword of keywords) {
          if (fileName.includes(keyword)) {
            score += 100;
          }

          if (filePath.includes(keyword)) {
            score += 50;
          }
        }
      }

      /*
       * -------------------------------------------------------
       * 5. SOURCE CODE PREFERENCE
       * -------------------------------------------------------
       */

      if (sourceExtensions.has(extension)) {
        score += 15;
      }

      /*
       * -------------------------------------------------------
       * 6. CONFIG / DOCUMENTATION PREFERENCE
       * -------------------------------------------------------
       */

      if (configExtensions.has(extension)) {
        score += 20;
      }

      if (documentationExtensions.has(extension)) {
        score += 15;
      }

      /*
       * -------------------------------------------------------
       * 7. SPECIAL INTENT BOOSTS
       * -------------------------------------------------------
       */

      if (
        detectedIntents.has("documentation") &&
        fileName === "readme.md"
      ) {
        score += 300;
      }

      if (
        detectedIntents.has("build") &&
        importantFiles.has(fileName)
      ) {
        score += 180;
      }

      if (
        detectedIntents.has("project") &&
        (
          fileName === "package.json" ||
          fileName === "readme.md" ||
          fileName === "main.tsx" ||
          fileName === "main.ts" ||
          fileName === "main.rs" ||
          fileName === "lib.rs"
        )
      ) {
        score += 150;
      }

      return {
        file,
        score,
      };
    })
    .filter((item) => item.score > 0)
    .sort((a, b) => {
      if (b.score !== a.score) {
        return b.score - a.score;
      }

      return a.file.path.localeCompare(
        b.file.path,
      );
    });

  /*
   * =========================================================
   * FILE SELECTION
   * =========================================================
   */

  const explicitFiles = scoredFiles.filter(
    (item) => item.score >= 900,
  );

  const selected = [
    ...explicitFiles,
    ...scoredFiles.filter(
      (item) => item.score < 900,
    ),
  ]
    .filter(
      (item, index, array) =>
        array.findIndex(
          (other) =>
            other.file.path === item.file.path,
        ) === index,
    )
    .slice(0, 10);

  /*
   * If the question is broad and nothing matched,
   * provide a small architectural snapshot.
   */

  if (selected.length === 0) {
    const fallbackNames = new Set([
      "package.json",
      "readme.md",
      "main.tsx",
      "main.ts",
      "app.tsx",
      "main.rs",
      "lib.rs",
      "cargo.toml",
    ]);

    const fallbackFiles = projectFiles
      .filter(
        (file) =>
          !file.is_directory &&
          fallbackNames.has(
            file.name.toLowerCase(),
          ),
      )
      .slice(0, 6);

    for (const file of fallbackFiles) {
      selected.push({
        file,
        score: 10,
      });
    }
  }

  if (selected.length === 0) {
    return "";
  }

  /*
   * =========================================================
   * READ SELECTED FILES
   * =========================================================
   */

  const fileContexts: string[] = [];

  let totalContextCharacters = 0;

  const MAX_FILE_CHARS = 100_000;
  const MAX_TOTAL_CHARS = 350_000;

  for (const item of selected) {
    try {
      const content = await readProjectFile(
        selectedProject,
        item.file.path,
      );

      let limitedContent = content;

      if (limitedContent.length > MAX_FILE_CHARS) {
        limitedContent =
          limitedContent.slice(
            0,
            MAX_FILE_CHARS,
          ) +
          "\n\n[File truncated for context size]";
      }

      if (
        totalContextCharacters +
          limitedContent.length >
        MAX_TOTAL_CHARS
      ) {
        break;
      }

      fileContexts.push(
        `FILE: ${item.file.path}\n\n${limitedContent}`,
      );

      totalContextCharacters +=
        limitedContent.length;
    } catch (err) {
      console.error(
        `Failed to read ${item.file.path}:`,
        err,
      );
    }
  }

  if (fileContexts.length === 0) {
    return "";
  }

  /*
   * =========================================================
   * FINAL AI CONTEXT
   * =========================================================
   */

  const detectedIntentText =
    detectedIntents.size > 0
      ? Array.from(detectedIntents).join(", ")
      : "general";

  return `
The user is working inside the attached project.

You have access to REAL files from the user's project.

Detected question intent:
${detectedIntentText}

Important rules:
- These files come directly from the user's project.
- Treat the provided file contents as the source of truth.
- Do not invent file contents.
- Do not claim you cannot access the user's project files.
- Analyze the actual code provided.
- When discussing code, always mention the actual file path.
- Distinguish clearly between facts found in the files and your suggestions.
- If the provided files are insufficient, identify the specific additional file that would be useful.
- Do not assume that a file exists unless it appears in the project context.

Selected project files:

${fileContexts.join(
  "\n\n==============================\n\n",
)}
`;
}
```




  for (const item of selected) {
    try {
      const content = await readProjectFile(
        selectedProject,
        item.file.path,
      );

      /*
       * Prevent one very large source file from
       * dominating the model context.
       */

      const limitedContent =
        content.length > 120_000
          ? content.slice(0, 120_000) +
            "\n\n[File truncated for context size]"
          : content;

      fileContexts.push(
        `FILE: ${item.file.path}\n\n${limitedContent}`,
      );
    } catch (err) {
      console.error(
        `Failed to read ${item.file.path}:`,
        err,
      );
    }
  }

  if (fileContexts.length === 0) {
    return "";
  }

  return `
The user is working inside the attached project.

Use the following REAL project files as authoritative context.

Important rules:
- These files come directly from the user's project.
- Do not invent their contents.
- When discussing code, mention the actual file path.
- Prefer the provided project files over assumptions.
- If the provided files are insufficient to answer confidently, say which additional file would be useful.

Selected project files:

${fileContexts.join(
  "\n\n==============================\n\n",
)}
`;
}




async function sendMessage() {
  const text = input.trim();

  if (!text || loading || !selectedProject) {
    return;
  }

  setError("");

  const userMessage: ChatMessage = {
    role: "user",
    content: text,
  };

  const conversation = [...messages, userMessage];

  setMessages(conversation);
  setInput("");
  setLoading(true);

  try {
    const projectContext =
      await buildProjectContext(text);

    const aiMessages = [
      ...(projectContext
        ? [
            {
              role: "system" as const,
              content: `
You are Local AI, a local coding assistant.

You are working inside the user's attached project.

When real project files are provided below:
- Treat them as the source of truth.
- Do not claim you cannot access the user's files.
- Analyze the actual contents.
- When referring to code, mention the file path.
- Do not invent file contents.

PROJECT CONTEXT:

${projectContext}
`,
            },
          ]
        : []),

      ...conversation.map((message) => ({
        role: message.role,
        content: message.content,
      })),
    ];

    const response = await chatWithModel(
      selectedModel,
      aiMessages,
    );

    const updatedMessages: ChatMessage[] = [
      ...conversation,
      {
        role: "assistant",
        content: response,
      },
    ];

    setMessages(updatedMessages);

    const updatedProject: StoredProject = {
      ...selectedProject,
      messages: updatedMessages,
      updated_at: new Date().toISOString(),
    };

    await saveProject(updatedProject);

    setSelectedProject(updatedProject);

    setProjects((previous) =>
      previous.map((project) =>
        project.id === updatedProject.id
          ? updatedProject
          : project,
      ),
    );
  } catch (err) {
    console.error(err);

    setError(
      err instanceof Error
        ? err.message
        : "Failed to communicate with LM Studio.",
    );
  } finally {
    setLoading(false);
  }
}


  function handleKeyDown(
    event: React.KeyboardEvent<HTMLTextAreaElement>,
  ) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      sendMessage();
    }
  }

  const modelName =
    models.find(
      (model) => model.id === selectedModel,
    )?.id ?? selectedModel;

  const projectName =
    selectedProject?.name ?? "No Project";

  return (
    <div className="local-ai">
      {/* TOP BAR */}
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">◈</span>
          <strong>LOCAL AI</strong>
        </div>

        <button className="project-select">
          {projectName}
          <ChevronDown size={14} />
        </button>

        <nav className="modes">
          <button className="mode active">Chat</button>
          <button className="mode">Agent</button>
          <button className="mode">Builder</button>
          <button className="mode">Research</button>
        </nav>

        <div className="top-right">
          <select
            className="model"
            value={selectedModel}
            onChange={(event) =>
              setSelectedModel(event.target.value)
            }
            disabled={modelsLoading}
          >
            {models.length === 0 ? (
              <option value={selectedModel}>
                {selectedModel}
              </option>
            ) : (
              models.map((model) => (
                <option
                  key={model.id}
                  value={model.id}
                >
                  {model.id}
                </option>
              ))
            )}
          </select>

          <button className="top-icon">
            <Settings size={16} />
          </button>

          <button className="top-icon">
            <Circle size={16} />
          </button>

          <button className="user-icon">
            <User size={14} />
          </button>
        </div>
      </header>

      {/* WORKSPACE */}
      <div className="workspace">
        {/* LEFT SIDEBAR */}
        <aside className="left-sidebar">
          <button
            className="new-project"
            onClick={createNewProject}
            disabled={projectLoading}
          >
            <Plus size={15} />
            New Project
          </button>

          <Section title="PROJECTS">
            {projects.map((project) => (
              <div
                key={project.id}
                className={`sidebar-row ${
                  selectedProject?.id === project.id
                    ? "selected"
                    : ""
                }`}
                onClick={() =>
                  selectProject(project)
                }
                style={{ cursor: "pointer" }}
              >
                <Circle
                  size={7}
                  fill={
                    selectedProject?.id === project.id
                      ? "#9b5cff"
                      : "transparent"
                  }
                  color={
                    selectedProject?.id === project.id
                      ? "#9b5cff"
                      : "#555b68"
                  }
                />

                {project.name}
              </div>
            ))}

            <div
              className="sidebar-row muted"
              onClick={createNewProject}
              style={{ cursor: "pointer" }}
            >
              <Plus size={13} />
              Add Project
            </div>
          </Section>

          <Section title="RECENT">
            {projects
              .flatMap((project) =>
                project.messages.length > 0
                  ? [
                      {
                        project,
                        message:
                          project.messages[
                            project.messages.length - 2
                          ],
                      },
                    ]
                  : [],
              )
              .slice(0, 5)
              .map((item, index) => (
                <div
                  className="sidebar-row"
                  key={`${item.project.id}-${index}`}
                >
                  <History size={13} />
                  {item.message?.content?.slice(
                    0,
                    30,
                  ) || item.project.name}
                </div>
              ))}
          </Section>

          <Section title="TASKS">
            <div className="sidebar-row">
              <Circle size={8} />
              Project files
            </div>

            <div className="sidebar-row">
              <Circle size={8} />
              AI analysis
            </div>
          </Section>

          <Section title="TOOLS">
            <div className="tools">
              <Tool
                icon={<Folder size={15} />}
                label="Files"
              />

              <Tool
                icon={<Terminal size={15} />}
                label="Terminal"
              />

              <Tool
                icon={<Search size={15} />}
                label="Search"
              />

              <Tool
                icon={<GitBranch size={15} />}
                label="Git"
              />
            </div>
          </Section>

          <div className="system-status">
            <div className="section-title">
              SYSTEM STATUS
            </div>

            <Status label="CPU" value="Local" />
            <Status
              label="RAM"
              value="16GB"
            />
            <Status
              label="MODEL"
              value="LM Studio"
            />
          </div>
        </aside>

        {/* CENTER */}
        <main className="center">
          <div className="task-header">
            <div>
              <div className="task-title">
                <h1>
                  {projectName}
                </h1>

                <span className="progress">
                  <i />
                  {loading
                    ? "Thinking"
                    : "Ready"}
                </span>
              </div>

              <p className="task-subtitle">
                {modelName}
              </p>
            </div>

            <div className="task-buttons">
              <button
                className="stop"
                onClick={() =>
                  setLoading(false)
                }
                disabled={!loading}
              >
                <Square
                  size={11}
                  fill="currentColor"
                />
                Stop
              </button>

              <button>
                <Pause size={13} />
                Pause
              </button>
            </div>
          </div>

          {/* CHAT */}
          <div className="chat">
            {messages.length === 0 && (
              <div className="empty-chat">
                <Bot size={30} />

                <h2>Local AI</h2>

                <p>
                  Ask anything about your project.
                </p>

                <small>
                  {selectedProject?.folder_path
                    ? `Attached: ${selectedProject.folder_path}`
                    : "No project folder attached"}
                </small>
              </div>
            )}

            {messages.map(
              (message, index) => (
                <Message
                  key={`${message.role}-${index}`}
                  user={
                    message.role === "user"
                  }
                  text={message.content}
                />
              ),
            )}

            {loading && (
              <div className="message ai">
                <Bot size={18} />

                <div>
                  <div className="message-name">
                    AI
                  </div>

                  <p className="thinking">
                    Thinking...
                  </p>
                </div>
              </div>
            )}

            {error && (
              <div className="ai-error">
                <strong>
                  Error
                </strong>

                <span>
                  {error}
                </span>
              </div>
            )}

            <div ref={chatEndRef} />
          </div>

          {/* COMPOSER */}
          <div className="composer">
            <textarea
              value={input}
              onChange={(event) =>
                setInput(event.target.value)
              }
              onKeyDown={handleKeyDown}
              placeholder={
                selectedProject
                  ? "Ask anything about your project..."
                  : "Create a project first..."
              }
              disabled={
                loading ||
                !selectedProject
              }
              rows={1}
            />

            <button
              title="Tools"
              type="button"
            >
              <Wrench size={14} />
            </button>

            <button
              className="send"
              onClick={sendMessage}
              disabled={
                !input.trim() ||
                loading ||
                !selectedProject
              }
              title="Send"
              type="button"
            >
              <Play
                size={13}
                fill="currentColor"
              />
            </button>
          </div>
        </main>

        {/* RIGHT PANEL */}
        <aside className="right-sidebar">
          <div className="right-tabs">
            <button className="active">
              FILES
            </button>
            <button>SEARCH</button>
            <button>GIT</button>
          </div>

          <div className="file-tree">
            <div className="path">
              <FolderOpen size={14} />

              {selectedProject?.folder_path ??
                "No project folder"}
            </div>

            {projectFiles.length === 0 ? (
              <div
                style={{
                  padding: "15px",
                  color: "#777",
                  fontSize: "12px",
                }}
              >
                {selectedProject
                  ? "No files found."
                  : "Create a project and select a folder."}
              </div>
            ) : (
              <ProjectFileTree
                files={projectFiles}
              />
            )}
          </div>

          <Info title="PROJECT OVERVIEW">
            <InfoRow
              label="Project"
              value={projectName}
            />

            <InfoRow
              label="Files"
              value={String(
                projectFiles.filter(
                  (file) =>
                    !file.is_directory,
                ).length,
              )}
            />

            <InfoRow
              label="Folders"
              value={String(
                projectFiles.filter(
                  (file) =>
                    file.is_directory,
                ).length,
              )}
            />

            <InfoRow
              label="AI Model"
              value={modelName}
            />
          </Info>

          <Info title="PROJECT">
            <InfoRow
              label="Folder"
              value={
                selectedProject?.folder_path
                  ? "Attached"
                  : "Not attached"
              }
            />

            <InfoRow
              label="Messages"
              value={String(
                messages.length,
              )}
            />
          </Info>

          <div className="timeline">
            <div className="section-title">
              ACTIVITY TIMELINE
            </div>

            <Timeline
              time="Now"
              text="Project opened"
            />

            <Timeline
              time="Now"
              text={
                projectFiles.length > 0
                  ? "Project files scanned"
                  : "Waiting for project"
              }
            />

            <Timeline
              time="Now"
              text="AI ready"
            />
          </div>
        </aside>
      </div>

      {/* BOTTOM PANEL */}
      <section className="bottom">
        <div className="bottom-tabs">
          <button className="active">
            TASKS
          </button>

          <button>CHANGES</button>
          <button>SIMULATION</button>
          <button>TERMINAL</button>
          <button>LOGS</button>
        </div>

        <div className="bottom-content">
          <div>
            <div className="section-title">
              PROJECT
            </div>

            <p>
              {selectedProject?.name ??
                "No project selected"}
            </p>

            <small>
              {projectFiles.length} items
              loaded
            </small>
          </div>

          <div>
            <div className="section-title">
              PROJECT FOLDER
            </div>

            <p>
              {selectedProject?.folder_path ??
                "No folder attached"}
            </p>
          </div>

          <div>
            <div className="section-title">
              AI STATUS
            </div>

            <code>
              {loading
                ? "Generating..."
                : "Ready"}
            </code>
          </div>

          <div className="expected">
            <div className="section-title">
              NEXT
            </div>

            <p>
              AI file understanding will
              be connected next.
            </p>
          </div>
        </div>
      </section>
    </div>
  );
}

function ProjectFileTree({
  files,
}: {
  files: ProjectFile[];
}) {
  const tree = buildTree(files);

  return (
    <div>
      {tree.map((node) => (
        <FileTreeNode
          key={node.path}
          node={node}
          depth={0}
        />
      ))}
    </div>
  );
}

interface FileTreeNode {
  name: string;
  path: string;
  is_directory: boolean;
  children: FileTreeNode[];
}

function buildTree(
  files: ProjectFile[],
): FileTreeNode[] {
  const roots: FileTreeNode[] = [];

  for (const file of files) {
    const parts = file.path
      .split("/")
      .filter(Boolean);

    let current = roots;

    parts.forEach((part, index) => {
      let existing = current.find(
        (node) => node.name === part,
      );

      if (!existing) {
        existing = {
          name: part,
          path: parts
            .slice(0, index + 1)
            .join("/"),
          is_directory:
            index < parts.length - 1 ||
            file.is_directory,
          children: [],
        };

        current.push(existing);
      }

      current = existing.children;
    });
  }

  return roots;
}

function FileTreeNode({
  node,
  depth,
}: {
  node: FileTreeNode;
  depth: number;
}) {
  const [expanded, setExpanded] =
    useState(false);

  return (
    <div>
      <div
        className="tree-row"
        style={{
          paddingLeft: `${depth * 14}px`,
          cursor: node.is_directory
            ? "pointer"
            : "default",
        }}
        onClick={() => {
          if (node.is_directory) {
            setExpanded(!expanded);
          }
        }}
      >
        {node.is_directory ? (
          <ChevronRight
            size={11}
            style={{
              transform: expanded
                ? "rotate(90deg)"
                : "rotate(0deg)",
            }}
          />
        ) : (
          <span className="tree-indent" />
        )}

        {node.is_directory ? (
          expanded ? (
            <FolderOpen
              size={13}
              className="folder"
            />
          ) : (
            <Folder
              size={13}
              className="folder"
            />
          )
        ) : (
          <FileCode2 size={13} />
        )}

        <span>{node.name}</span>
      </div>

      {expanded &&
        node.children.map((child) => (
          <FileTreeNode
            key={child.path}
            node={child}
            depth={depth + 1}
          />
        ))}
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="sidebar-section">
      <div className="section-title">
        {title}
      </div>

      {children}
    </section>
  );
}

function Tool({
  icon,
  label,
}: {
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <div className="tool">
      {icon}
      <span>{label}</span>
    </div>
  );
}

function Status({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="status-row">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function Message({
  text,
  user = false,
}: {
  text: string;
  user?: boolean;
}) {
  return (
    <div
      className={`message ${
        user ? "user" : "ai"
      }`}
    >
      <div className="message-avatar">
        {user ? (
          <User size={15} />
        ) : (
          <Bot size={16} />
        )}
      </div>

      <div className="message-body">
        <div className="message-name">
          {user ? "You" : "AI"}
        </div>

        <p>{text}</p>
      </div>
    </div>
  );
}

function Info({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="info-card">
      <div className="section-title">
        {title}
      </div>

      {children}
    </div>
  );
}

function InfoRow({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="info-row">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function Timeline({
  time,
  text,
}: {
  time: string;
  text: string;
}) {
  return (
    <div className="timeline-row">
      <span>{time}</span>
      <strong>●</strong>
      <p>{text}</p>
    </div>
  );
}

