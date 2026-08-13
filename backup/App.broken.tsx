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

import {
  useEffect,
  useState,
  type ReactNode,
} from "react";

import { open } from "@tauri-apps/plugin-dialog";

import {
  chatWithModel,
  getModels,
  type AIModel,
} from "./services/ai";

import {
  createProject,
  getProjects,
  listProjectFiles,
  readProjectFile,
  saveProject,
  type ProjectFile,
  type StoredProject,
} from "./services/projectStore";

import "./App.css";

interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

interface FileTreeNode {
  name: string;
  path: string;
  isDirectory: boolean;
  children: FileTreeNode[];
}

const fallbackProjects = [
  "MyApp",
  "Website",
  "PhishGuard",
  "DataAnalyzer",
];

const recent = [
  "Fix auth bug",
  "Improve speed",
  "Database migration",
];

const tasks = [
  "Fix auth bug",
  "Add unit tests",
];

function buildFileTree(
  files: ProjectFile[],
): FileTreeNode[] {
  const roots: FileTreeNode[] = [];

  for (const file of files) {
    const parts = file.path
      .split("/")
      .filter(Boolean);

    let current = roots;
    let currentPath = "";

    parts.forEach((part, index) => {
      currentPath = currentPath
        ? `${currentPath}/${part}`
        : part;

      const isLast =
        index === parts.length - 1;

      let node = current.find(
        (item) => item.name === part,
      );

      if (!node) {
        node = {
          name: part,
          path: currentPath,
          isDirectory: isLast
            ? file.is_directory
            : true,
          children: [],
        };

        current.push(node);
      }

      current = node.children;
    });
  }

  function sortNodes(
    nodes: FileTreeNode[],
  ) {
    nodes.sort((a, b) => {
      if (
        a.isDirectory !==
        b.isDirectory
      ) {
        return a.isDirectory ? -1 : 1;
      }

      return a.name.localeCompare(
        b.name,
      );
    });

    for (const node of nodes) {
      sortNodes(node.children);
    }
  }

  sortNodes(roots);

  return roots;
}

function getFileExtension(
  path: string,
): string {
  const name =
    path.split("/").pop() ?? "";

  const index = name.lastIndexOf(".");

  return index >= 0
    ? name
        .slice(index + 1)
        .toLowerCase()
    : "";
}


type ArchitectureRole =
  | "frontend"
  | "native-backend"
  | "ai-integration"
  | "python-support"
  | "configuration"
  | "test"
  | "documentation"
  | "dependency"
  | "build-artifact"
  | "environment"
  | "application-code"
  | "unknown";

interface ArchitectureInfo {
  role: ArchitectureRole;
  description: string;
  priority: number;
}

const IGNORED_DIRECTORY_NAMES = new Set([
  "node_modules",
  "target",
  "dist",
  "build",
  ".git",
  ".hg",
  ".svn",
  ".venv",
  "venv",
  "env",
  ".env",
  "__pycache__",
  ".pytest_cache",
  ".mypy_cache",
  ".ruff_cache",
  "coverage",
  ".next",
  ".nuxt",
  ".cache",
]);

const GENERATED_FILE_NAMES = new Set([
  "package-lock.json",
  "npm-shrinkwrap.json",
  "yarn.lock",
  "pnpm-lock.yaml",
]);

function getArchitectureInfo(
  filePath: string,
): ArchitectureInfo {
  const normalized = filePath
    .replace(/\\/g, "/")
    .toLowerCase();

  const parts = normalized
    .split("/")
    .filter(Boolean);

  const fileName =
    parts[parts.length - 1] ?? "";

  const extension =
    getFileExtension(normalized);

  /*
   * Infrastructure / generated content
   * must be classified before application code.
   */
  if (
    parts.some((part) =>
      IGNORED_DIRECTORY_NAMES.has(part),
    )
  ) {
    if (
      parts.includes(".venv") ||
      parts.includes("venv") ||
      parts.includes("env")
    ) {
      return {
        role: "environment",
        description:
          "Python virtual environment or local runtime environment; not application logic.",
        priority: -100,
      };
    }

    if (
      parts.includes("node_modules")
    ) {
      return {
        role: "dependency",
        description:
          "Installed JavaScript/Node dependency; not application source code.",
        priority: -100,
      };
    }

    if (
      parts.includes("target") ||
      parts.includes("dist") ||
      parts.includes("build")
    ) {
      return {
        role: "build-artifact",
        description:
          "Generated build output; not application source code.",
        priority: -100,
      };
    }

    return {
      role: "dependency",
      description:
        "Generated, cached, or dependency content; not primary application logic.",
      priority: -100,
    };
  }

  if (
    GENERATED_FILE_NAMES.has(fileName)
  ) {
    return {
      role: "dependency",
      description:
        "Dependency lockfile; describes installed dependencies but is not application logic.",
      priority: -20,
    };
  }

  /*
   * React / TypeScript frontend.
   */
  if (
    normalized.startsWith("frontend/") ||
    normalized.includes("/frontend/")
  ) {
    if (
      normalized.includes(
        "/services/ai.",
      ) ||
      fileName === "ai.ts" ||
      fileName === "ai.tsx"
    ) {
      return {
        role: "ai-integration",
        description:
          "Frontend AI integration and communication with the local LM Studio service.",
        priority: 30,
      };
    }

    if (
      extension === "tsx" ||
      extension === "ts" ||
      extension === "jsx" ||
      extension === "js" ||
      extension === "css" ||
      extension === "scss" ||
      extension === "html"
    ) {
      return {
        role: "frontend",
        description:
          "React/TypeScript/Vite frontend application code.",
        priority: 20,
      };
    }

    if (
      fileName === "package.json" ||
      fileName.includes("vite.config") ||
      fileName.startsWith("tsconfig")
    ) {
      return {
        role: "configuration",
        description:
          "Frontend build or TypeScript configuration.",
        priority: 10,
      };
    }
  }

  /*
   * Rust/Tauri native backend.
   */
  if (
    normalized.startsWith("src-tauri/") ||
    normalized.includes("/src-tauri/")
  ) {
    if (
      extension === "rs"
    ) {
      return {
        role: "native-backend",
        description:
          "Rust/Tauri native backend and application logic.",
        priority: 40,
      };
    }

    if (
      fileName === "cargo.toml" ||
      fileName === "cargo.lock"
    ) {
      return {
        role: "configuration",
        description:
          "Rust/Tauri project configuration or dependency metadata.",
        priority: 10,
      };
    }
  }

  /*
   * Python backend/support area.
   *
   * Important:
   * This intentionally does NOT classify backend/.venv
   * as backend application logic because .venv was
   * excluded above.
   */
  if (
    normalized.startsWith("backend/") ||
    normalized.includes("/backend/")
  ) {
    if (
      extension === "py"
    ) {
      return {
        role: "python-support",
        description:
          "Python code in the separate backend/support area.",
        priority: 15,
      };
    }

    if (
      fileName === "requirements.txt" ||
      fileName === "pyproject.toml"
    ) {
      return {
        role: "configuration",
        description:
          "Python dependency or project configuration.",
        priority: 8,
      };
    }
  }

  /*
   * Tests.
   */
  if (
    normalized.includes("/test/") ||
    normalized.includes("/tests/") ||
    normalized.includes(".test.") ||
    normalized.includes(".spec.")
  ) {
    return {
      role: "test",
      description:
        "Automated test code.",
      priority: 5,
    };
  }

  /*
   * Documentation.
   */
  if (
    extension === "md" ||
    extension === "mdx" ||
    fileName === "readme" ||
    fileName.startsWith("readme.")
  ) {
    return {
      role: "documentation",
      description:
        "Project documentation.",
      priority: 3,
    };
  }

  /*
   * General application source.
   */
  const sourceExtensions = [
    "ts",
    "tsx",
    "js",
    "jsx",
    "py",
    "rs",
    "java",
    "kt",
    "c",
    "cpp",
    "h",
    "hpp",
    "go",
    "swift",
    "dart",
  ];

  if (
    sourceExtensions.includes(
      extension,
    )
  ) {
    return {
      role: "application-code",
      description:
        "Application source code.",
      priority: 5,
    };
  }

  /*
   * Known project configuration.
   */
  const importantConfigurationFiles = [
    "package.json",
    "cargo.toml",
    "vite.config",
    "tsconfig",
    "requirements.txt",
    "pyproject.toml",
  ];

  if (
    importantConfigurationFiles.some(
      (name) =>
        fileName.includes(name),
    )
  ) {
    return {
      role: "configuration",
      description:
        "Project configuration.",
      priority: 5,
    };
  }

  return {
    role: "unknown",
    description:
      "Unclassified project file.",
    priority: 0,
  };
}

function scoreFileRelevance(
  file: ProjectFile,
  query: string,
): number {
  if (file.is_directory) {
    return -1;
  }

  const architecture =
    getArchitectureInfo(
      file.path,
    );

  /*
   * Never use ignored/generated/environment
   * files as normal AI context.
   */
  if (
    architecture.priority <= -100
  ) {
    return -100;
  }

  const path =
    file.path.toLowerCase();

  const words = query
    .toLowerCase()
    .split(/[^a-z0-9_./-]+/)
    .filter(
      (word) =>
        word.length >= 2,
    );

  let score =
    architecture.priority;

  /*
   * Query/path relevance.
   */
  for (const word of words) {
    if (path.includes(word)) {
      score += 10;
    }
  }

  /*
   * Architecture-aware keywords.
   */
  const architectureKeywords: Record<
    ArchitectureRole,
    string[]
  > = {
    frontend: [
      "frontend",
      "react",
      "component",
      "ui",
      "tsx",
      "typescript",
    ],

    "native-backend": [
      "backend",
      "rust",
      "tauri",
      "native",
      "command",
      "invoke",
    ],

    "ai-integration": [
      "ai",
      "model",
      "lm",
      "lm studio",
      "llm",
      "chat",
      "stream",
    ],

    "python-support": [
      "python",
      "backend",
      "api",
      "fastapi",
    ],

    configuration: [
      "config",
      "configuration",
      "dependencies",
      "build",
      "package",
    ],

    test: [
      "test",
      "testing",
      "spec",
    ],

    documentation: [
      "docs",
      "documentation",
      "readme",
    ],

    dependency: [],
    "build-artifact": [],
    environment: [],

    "application-code": [
      "code",
      "logic",
      "application",
      "source",
    ],

    unknown: [],
  };

  const keywords =
    architectureKeywords[
      architecture.role
    ];

  for (const word of words) {
    if (
      keywords.some(
        (keyword) =>
          keyword.includes(word) ||
          word.includes(keyword),
      )
    ) {
      score += 8;
    }
  }

  /*
   * Important project files.
   */
  const importantFiles = [
    "readme",
    "package.json",
    "cargo.toml",
    "vite.config",
    "tsconfig",
    "requirements.txt",
    "pyproject.toml",
    "main.",
    "app.",
    "index.",
    "lib.rs",
  ];

  for (const name of importantFiles) {
    if (path.includes(name)) {
      score += 3;
    }
  }

  /*
   * Source files get a small baseline boost.
   */
  const sourceExtensions = [
    "ts",
    "tsx",
    "js",
    "jsx",
    "py",
    "rs",
    "java",
    "kt",
    "c",
    "cpp",
    "h",
    "hpp",
    "go",
    "swift",
    "dart",
    "css",
    "scss",
    "html",
    "json",
    "yaml",
    "yml",
    "toml",
  ];

  if (
    sourceExtensions.includes(
      getFileExtension(file.path),
    )
  ) {
    score += 2;
  }

  return score;
}

  async function loadModels() {
    setModelsLoading(true);

    try {
      const availableModels =
        await getModels();

      setModels(availableModels);

      if (
        availableModels.length > 0 &&
        !availableModels.some(
          (model) =>
            model.id ===
            selectedModel,
        )
      ) {
        setSelectedModel(
          availableModels[0].id,
        );
      }

      setError("");
    } catch (err) {
      console.error(err);

      setError(
        "LM Studio is not available.",
      );
    } finally {
      setModelsLoading(false);
    }
  }

  async function loadProjectFiles(
    project: StoredProject,
  ) {
    if (!project.folder_path) {
      setProjectFiles([]);
      setExpandedFolders(
        new Set(),
      );
      return;
    }

    setFilesLoading(true);

    try {
      const files =
        await listProjectFiles(
          project,
        );

      setProjectFiles(files);
      setExpandedFolders(
        new Set(),
      );
      setSelectedFile(null);
      setError("");
    } catch (err) {
      console.error(err);

      setProjectFiles([]);

      setError(
        err instanceof Error
          ? err.message
          : "Failed to scan project files.",
      );
    } finally {
      setFilesLoading(false);
    }
  }

  async function loadStoredProjects() {
    try {
      const storedProjects =
        await getProjects();

      setProjects(storedProjects);

      if (
        storedProjects.length === 0
      ) {
        setSelectedProject(null);
        setSelectedProjectId(null);
        setMessages([]);
        return;
      }

      const first =
        storedProjects[0];

      setSelectedProjectId(
        first.id,
      );

      setSelectedProject(first);

      setMessages(
        first.messages.map(
          (message) => ({
            role: message.role,
            content: message.content,
          }),
        ),
      );

      await loadProjectFiles(
        first,
      );
    } catch (err) {
      console.error(err);

      setProjects([]);
      setSelectedProject(null);
      setSelectedProjectId(null);
      setMessages([]);
      setProjectFiles([]);
    }
  }

  async function handleCreateProject() {
    const name =
      window.prompt(
        "Enter project name:",
      );

    if (!name?.trim()) {
      return;
    }

    try {
      const now =
        new Date().toISOString();

      const project =
        await createProject(
          crypto.randomUUID(),
          name.trim(),
          now,
        );

      setProjects((current) => [
        ...current,
        project,
      ]);

      setSelectedProjectId(
        project.id,
      );

      setSelectedProject(project);
      setMessages([]);
      setProjectFiles([]);
      setExpandedFolders(
        new Set(),
      );
      setSelectedFile(null);
      setError("");
    } catch (err) {
      console.error(err);

      setError(
        err instanceof Error
          ? err.message
          : "Failed to create project.",
      );
    }
  }

  async function handleSelectProject(
    project: StoredProject,
  ) {
    setSelectedProjectId(
      project.id,
    );

    setSelectedProject(project);

    setMessages(
      project.messages.map(
        (message) => ({
          role: message.role,
          content: message.content,
        }),
      ),
    );

    setError("");

    await loadProjectFiles(
      project,
    );
  }

  async function handleOpenProjectFolder() {
    try {
      const selected =
        await open({
          directory: true,
          multiple: false,
          title:
            "Select Project Folder",
        });

      if (
        !selected ||
        Array.isArray(selected)
      ) {
        return;
      }

      const folderPath = selected;

      const folderName =
        folderPath
          .split("/")
          .filter(Boolean)
          .pop() ??
        "Project";

      const now =
        new Date().toISOString();

      const project =
        await createProject(
          crypto.randomUUID(),
          folderName,
          now,
        );

      const projectWithPath: StoredProject =
        {
          ...project,
          folder_path:
            folderPath,
        };

      await saveProject(
        projectWithPath,
      );

      setProjects((current) => [
        ...current,
        projectWithPath,
      ]);

      setSelectedProjectId(
        projectWithPath.id,
      );

      setSelectedProject(
        projectWithPath,
      );

      setMessages([]);
      setError("");

      await loadProjectFiles(
        projectWithPath,
      );
    } catch (err) {
      console.error(err);

      setError(
        err instanceof Error
          ? err.message
          : "Failed to open project folder.",
      );
    }
  }

  async function handleDeleteProject(
    project: StoredProject,
  ) {
    const confirmed =
      window.confirm(
        `Delete project "${project.name}"?`,
      );

    if (!confirmed) {
      return;
    }

    try {
      const { deleteProject } =
        await import(
          "./services/projectStore"
        );

      await deleteProject(
        project.id,
      );

      const remaining =
        projects.filter(
          (item) =>
            item.id !== project.id,
        );

      setProjects(remaining);

      if (
        selectedProjectId ===
        project.id
      ) {
        if (
          remaining.length > 0
        ) {
          await handleSelectProject(
            remaining[0],
          );
        } else {
          setSelectedProjectId(
            null,
          );

          setSelectedProject(
            null,
          );

          setMessages([]);
          setProjectFiles([]);
          setExpandedFolders(
            new Set(),
          );
        }
      }
    } catch (err) {
      console.error(err);

      setError(
        err instanceof Error
          ? err.message
          : "Failed to delete project.",
      );
    }
  }

  function toggleFolder(
    path: string,
  ) {
    setExpandedFolders(
      (current) => {
        const next = new Set(
          current,
        );

        if (next.has(path)) {
          next.delete(path);
        } else {
          next.add(path);
        }

        return next;
      },
    );
  }

  async function handleFileClick(
    file: FileTreeNode,
  ) {
    if (file.isDirectory) {
      toggleFolder(file.path);
      return;
    }

    setSelectedFile(file.path);

    console.log(
      "Selected project file:",
      file.path,
    );
  }


async function buildProjectContext(
  query: string,
): Promise<string> {
  if (
    !selectedProject ||
    projectFiles.length === 0
  ) {
    return "";
  }

  const architectureEntries =
    projectFiles
      .filter(
        (file) =>
          !file.is_directory,
      )
      .map((file) => ({
        file,
        architecture:
          getArchitectureInfo(
            file.path,
          ),
        score:
          scoreFileRelevance(
            file,
            query,
          ),
      }))
      /*
       * Completely exclude dependency,
       * generated and environment content.
       */
      .filter(
        (item) =>
          item.architecture.priority >
          -100,
      );

  /*
   * Select the most relevant application files.
   */
  const candidates =
    architectureEntries
      .sort(
        (a, b) =>
          b.score - a.score,
      );

  const selected =
    candidates
      .slice(0, 8)
      .filter(
        (item) =>
          item.score > 0 ||
          candidates.length <= 8,
      );

  const parts: string[] = [];

  /*
   * Add an architecture map before actual
   * source files so the model understands
   * the project structure.
   */
  const architectureMap =
    architectureEntries
      .filter(
        (item) =>
          item.architecture.role !==
          "unknown",
      )
      .sort(
        (a, b) =>
          b.architecture.priority -
          a.architecture.priority,
      )
      .slice(0, 40)
      .map(
        (item) =>
          `- ${item.file.path} → ${item.architecture.role}: ${item.architecture.description}`,
      );

  parts.push(
    `PROJECT ARCHITECTURE:

This Local AI application is a Tauri desktop application.

Core architecture:
- frontend/ → React + TypeScript + Vite frontend.
- src-tauri/ → Rust + Tauri native backend and desktop integration.
- frontend/src/services/ai.ts → local AI/LM Studio communication.
- LM Studio → local model/API service used by the application.
- backend/ → separate/supporting Python area; do not automatically assume it is the application's core backend.
- backend/.venv/ → Python virtual environment and dependencies, NOT application logic.
- node_modules/ → JavaScript dependencies, NOT application logic.
- target/ → Rust build artifacts, NOT application logic.
- dist/ → frontend build artifacts, NOT application logic.

Architecture classification:
${architectureMap.join("\n")}

Use this architecture when answering questions about where functionality lives. Prefer actual application source files over dependencies, virtual environments, caches, or generated build output.`,
  );

  for (const item of selected) {
    try {
      const content =
        await readProjectFile(
          selectedProject,
          item.file.path,
        );

      const limitedContent =
        content.length > 12000
          ? `${content.slice(
              0,
              12000,
            )}\n\n[File truncated]`
          : content;

      parts.push(
        `FILE: ${item.file.path}
ROLE: ${item.architecture.role}
DESCRIPTION: ${item.architecture.description}

${limitedContent}`,
      );
    } catch (err) {
      console.warn(
        `Could not read ${item.file.path}`,
        err,
      );
    }
  }

  if (parts.length === 1) {
    return "";
  }

  return parts.join(
    "\n\n==============================\n\n",
  );
}

  async function sendMessage() {
    const text =
      input.trim();

    if (
      !text ||
      loading
    ) {
      return;
    }

    setError("");

    const userMessage: ChatMessage =
      {
        role: "user",
        content: text,
      };

    const conversation = [
      ...messages,
      userMessage,
    ];

    setMessages(conversation);
    setInput("");
    setLoading(true);

    try {
      const projectContext =
        await buildProjectContext(
          text,
        );

      const aiMessages = [
        ...(projectContext
          ? [
              {
                role: "system" as const,
                content: `You are Local AI, a local coding assistant.

You are helping the user understand and work with their real project.

PROJECT STRUCTURE:
${projectFiles
  .filter(
    (file) =>
      !file.is_directory,
  )
  .map(
    (file) =>
      file.path,
  )
  .join("\n")}

RELEVANT PROJECT FILE CONTENT:
${projectContext}

Rules:
- Use the provided project files as the source of truth.
- Do not invent file contents.
- If the supplied files are insufficient, say so.
- Mention file paths when discussing project code.
- Do not claim that you changed files.
- You are currently in analysis/read-only mode.`,
              },
            ]
          : []),
        ...conversation.map(
          (message) => ({
            role: message.role,
            content:
              message.content,
          }),
        ),
      ];

      const response =
        await chatWithModel(
          selectedModel,
          aiMessages,
        );

      const updatedMessages:
        ChatMessage[] = [
          ...conversation,
          {
            role: "assistant",
            content: response,
          },
        ];

      setMessages(
        updatedMessages,
      );

      if (selectedProject) {
        const updatedProject:
          StoredProject = {
            ...selectedProject,
            messages:
              updatedMessages,
            updated_at:
              new Date().toISOString(),
          };

        await saveProject(
          updatedProject,
        );

        setSelectedProject(
          updatedProject,
        );

        setProjects((current) =>
          current.map(
            (project) =>
              project.id ===
              updatedProject.id
                ? updatedProject
                : project,
          ),
        );
      }
    } catch (err) {
      console.error(err);

      setError(
        err instanceof Error
          ? err.message
          : "Failed to communicate with LM Studio.",
      );

      setMessages(
        conversation,
      );
    } finally {
      setLoading(false);
    }
  }

  function handleKeyDown(
    event: React.KeyboardEvent<HTMLTextAreaElement>,
  ) {
    if (
      event.key === "Enter" &&
      !event.shiftKey
    ) {
      event.preventDefault();

      void sendMessage();
    }
  }

  const modelName =
    models.find(
      (model) =>
        model.id ===
        selectedModel,
    )?.id ?? selectedModel;

  const projectName =
    selectedProject?.name ??
    "No Project";

  const fileTree =
    buildFileTree(
      projectFiles,
    );

  const fileCount =
    projectFiles.filter(
      (file) =>
        !file.is_directory,
    ).length;

  const folderCount =
    projectFiles.filter(
      (file) =>
        file.is_directory,
    ).length;

  return (
    <div className="local-ai">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">
            ◈
          </span>

          <strong>
            LOCAL AI
          </strong>
        </div>

        <button className="project-select">
          {projectName}

          <ChevronDown size={14} />
        </button>

        <nav className="modes">
          <button className="mode active">
            Chat
          </button>

          <button className="mode">
            Agent
          </button>

          <button className="mode">
            Builder
          </button>

          <button className="mode">
            Research
          </button>
        </nav>

        <div className="top-right">
          <select
            className="model"
            value={
              selectedModel
            }
            onChange={(event) =>
              setSelectedModel(
                event.target.value,
              )
            }
            disabled={
              modelsLoading
            }
          >
            {models.length ===
            0 ? (
              <option
                value={
                  selectedModel
                }
              >
                {selectedModel}
              </option>
            ) : (
              models.map(
                (model) => (
                  <option
                    key={
                      model.id
                    }
                    value={
                      model.id
                    }
                  >
                    {model.id}
                  </option>
                ),
              )
            )}
          </select>

          <button className="top-icon">
            <Settings
              size={16}
            />
          </button>

          <button className="top-icon">
            <Circle
              size={16}
            />
          </button>

          <button className="user-icon">
            <User size={14} />
          </button>
        </div>
      </header>

      <div className="workspace">
        <aside className="left-sidebar">
          <button
            className="new-project"
            onClick={() =>
              void handleCreateProject()
            }
          >
            <Plus size={15} />
            New Project
          </button>

          <Section title="PROJECTS">
            {projects.length >
            0
              ? projects.map(
                  (project) => (
                    <div
                      key={
                        project.id
                      }
                      className={`sidebar-row ${
                        project.id ===
                        selectedProjectId
                          ? "selected"
                          : ""
                      }`}
                      onClick={() =>
                        void handleSelectProject(
                          project,
                        )
                      }
                    >
                      <Circle
                        size={7}
                        fill={
                          project.id ===
                          selectedProjectId
                            ? "#9b5cff"
                            : "transparent"
                        }
                        color={
                          project.id ===
                          selectedProjectId
                            ? "#9b5cff"
                            : "#555b68"
                        }
                      />

                      <span>
                        {
                          project.name
                        }
                      </span>
                    </div>
                  ),
                )
              : fallbackProjects.map(
                  (
                    project,
                    index,
                  ) => (
                    <div
                      key={
                        project
                      }
                      className={`sidebar-row ${
                        index ===
                        0
                          ? "selected"
                          : ""
                      }`}
                    >
                      <Circle
                        size={7}
                        fill={
                          index ===
                          0
                            ? "#9b5cff"
                            : "transparent"
                        }
                        color={
                          index ===
                          0
                            ? "#9b5cff"
                            : "#555b68"
                        }
                      />

                      <span>
                        {
                          project
                        }
                      </span>
                    </div>
                  ),
                )}

            {selectedProject && (
              <button
                className="sidebar-row muted"
                onClick={() =>
                  void handleDeleteProject(
                    selectedProject,
                  )
                }
              >
                Delete current
                project
              </button>
            )}

            <div
              className="sidebar-row muted"
              onClick={() =>
                void handleCreateProject()
              }
            >
              <Plus size={13} />
              Add Project
            </div>
          </Section>

          <div
            className="sidebar-row muted"
            onClick={() =>
              void handleOpenProjectFolder()
            }
          >
            <FolderOpen
              size={13}
            />
            Open Project Folder
          </div>

          <Section title="RECENT">
            {recent.map(
              (item) => (
                <div
                  className="sidebar-row"
                  key={item}
                >
                  <History
                    size={13}
                  />
                  {item}
                </div>
              ),
            )}
          </Section>

          <Section title="TASKS">
            {tasks.map(
              (item) => (
                <div
                  className="sidebar-row"
                  key={item}
                >
                  <Circle
                    size={8}
                  />
                  {item}
                </div>
              ),
            )}
          </Section>

          <Section title="TOOLS">
            <div className="tools">
              <Tool
                icon={
                  <Folder
                    size={15}
                  />
                }
                label="Files"
              />

              <Tool
                icon={
                  <Terminal
                    size={15}
                  />
                }
                label="Terminal"
              />

              <Tool
                icon={
                  <Search
                    size={15}
                  />
                }
                label="Search"
              />

              <Tool
                icon={
                  <GitBranch
                    size={15}
                  />
                }
                label="Git"
              />
            </div>
          </Section>

          <div className="system-status">
            <div className="section-title">
              SYSTEM STATUS
            </div>

            <Status
              label="CPU"
              value="--"
            />

            <Status
              label="RAM"
              value="--"
            />

            <Status
              label="VRAM"
              value="--"
            />
          </div>
        </aside>

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
                disabled={
                  !loading
                }
                onClick={() => {
                  setLoading(
                    false,
                  );
                }}
              >
                <Square
                  size={11}
                  fill="currentColor"
                />
                Stop
              </button>

              <button
                disabled={
                  !loading
                }
              >
                <Pause
                  size={13}
                />
                Pause
              </button>
            </div>
          </div>

          <div className="chat">
            {messages.length ===
              0 && (
              <div className="empty-chat">
                <Bot
                  size={30}
                />

                <h2>
                  Local AI
                </h2>

                <p>
                  Ask anything
                  about your
                  project.
                </p>

                <small>
                  Running locally
                  through LM
                  Studio
                </small>
              </div>
            )}

            {messages.map(
              (
                message,
                index,
              ) => (
                <Message
                  key={`${message.role}-${index}`}
                  user={
                    message.role ===
                    "user"
                  }
                  text={
                    message.content
                  }
                />
              ),
            )}

            {loading && (
              <div className="message ai">
                <Bot
                  size={18}
                />

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
                  Connection error
                </strong>

                <span>
                  {error}
                </span>
              </div>
            )}
          </div>

          <div className="composer">
            <textarea
              value={input}
              onChange={(event) =>
                setInput(
                  event.target
                    .value,
                )
              }
              onKeyDown={
                handleKeyDown
              }
              placeholder="Ask anything about your project..."
              disabled={
                loading
              }
              rows={1}
            />

            <button
              title="Tools"
              type="button"
            >
              <Wrench
                size={14}
              />
            </button>

            <button
              className="send"
              onClick={() =>
                void sendMessage()
              }
              disabled={
                !input.trim() ||
                loading
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

        <aside className="right-sidebar">
          <div className="right-tabs">
            <button className="active">
              FILES
            </button>

            <button>
              SEARCH
            </button>

            <button>
              GIT
            </button>
          </div>

          <div className="file-tree">
            <div className="path">
              <FolderOpen
                size={14}
              />

              {selectedProject?.folder_path ??
                `~/Projects/${projectName}`}
            </div>

            {filesLoading ? (
              <div className="file-loading">
                Scanning
                project...
              </div>
            ) : projectFiles.length ===
              0 ? (
              <div className="file-empty">
                {selectedProject?.folder_path
                  ? "No files found."
                  : "Open a project folder to view files."}
              </div>
            ) : (
              <FileTree
                nodes={fileTree}
                expandedFolders={
                  expandedFolders
                }
                onToggle={
                  toggleFolder
                }
                onFileClick={
                  handleFileClick
                }
                selectedFile={
                  selectedFile
                }
              />
            )}
          </div>

          <Info title="PROJECT OVERVIEW">
            <InfoRow
              label="Language"
              value="Auto"
            />

            <InfoRow
              label="Framework"
              value="Auto"
            />

            <InfoRow
              label="Files"
              value={String(
                fileCount,
              )}
            />

            <InfoRow
              label="Folders"
              value={String(
                folderCount,
              )}
            />
          </Info>

          <Info title="PROJECT STATS">
            <InfoRow
              label="Files"
              value={String(
                fileCount,
              )}
            />

            <InfoRow
              label="Folders"
              value={String(
                folderCount,
              )}
            />

            <InfoRow
              label="Total Items"
              value={String(
                projectFiles.length,
              )}
            />

            <InfoRow
              label="Selected"
              value={
                selectedFile ??
                "--"
              }
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
                projectFiles.length >
                0
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

      <section className="bottom">
        <div className="bottom-tabs">
          <button className="active">
            TASKS
          </button>

          <button>
            CHANGES
          </button>

          <button>
            SIMULATION
          </button>

          <button>
            TERMINAL
          </button>

          <button>
            LOGS
          </button>
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
              {
                projectFiles.length
              }{" "}
              items loaded
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
              AI can now inspect
              relevant project
              files when answering
              questions.
            </p>
          </div>
        </div>
      </section>
    </div>
  );
}

function FileTree({
  nodes,
  expandedFolders,
  onToggle,
  onFileClick,
  selectedFile,
  depth = 0,
}: {
  nodes: FileTreeNode[];
  expandedFolders: Set<string>;
  onToggle: (
    path: string,
  ) => void;
  onFileClick: (
    file: FileTreeNode,
  ) => void;
  selectedFile: string | null;
  depth?: number;
}) {
  return (
    <div>
      {nodes.map((node) => {
        const expanded =
          expandedFolders.has(
            node.path,
          );

        const selected =
          selectedFile ===
          node.path;

        return (
          <div
            key={node.path}
          >
            <div
              className={`tree-row ${
                selected
                  ? "selected"
                  : ""
              }`}
              style={{
                paddingLeft:
                  `${8 + depth * 16}px`,
                cursor:
                  "pointer",
              }}
              onClick={() =>
                onFileClick(
                  node,
                )
              }
            >
              {node.isDirectory ? (
                expanded ? (
                  <ChevronDown
                    size={11}
                  />
                ) : (
                  <ChevronRight
                    size={11}
                  />
                )
              ) : (
                <span className="tree-indent" />
              )}

              {node.isDirectory ? (
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
                <FileCode2
                  size={13}
                />
              )}

              <span
                title={
                  node.path
                }
              >
                {node.name}
              </span>
            </div>

            {node.isDirectory &&
              expanded &&
              node.children.length >
                0 && (
                <FileTree
                  nodes={
                    node.children
                  }
                  expandedFolders={
                    expandedFolders
                  }
                  onToggle={
                    onToggle
                  }
                  onFileClick={
                    onFileClick
                  }
                  selectedFile={
                    selectedFile
                  }
                  depth={
                    depth + 1
                  }
                />
              )}
          </div>
        );
      })}
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="sidebar-section">
      <div className="section-title">
        {title}
      </div>

      {children}
    </div>
  );
}

function Tool({
  icon,
  label,
}: {
  icon: ReactNode;
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
        user
          ? "user"
          : "ai"
      }`}
    >
      {user ? (
        <User size={18} />
      ) : (
        <Bot size={18} />
      )}

      <div className="message-body">
        <div className="message-name">
          {user
            ? "You"
            : "AI"}
        </div>

        <p>
          {text}
        </p>
      </div>
    </div>
  );
}

function Info({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="info-section">
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
      <strong
        title={value}
      >
        {value}
      </strong>
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
    <div className="timeline-item">
      <span className="timeline-time">
        {time}
      </span>

      <span className="timeline-dot">
        ●
      </span>

      <span>
        {text}
      </span>
    </div>
  );
}

