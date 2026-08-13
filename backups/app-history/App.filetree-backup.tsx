import {
  Bot,
  ChevronDown,
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
  useRef,
  useState,
  type ReactNode,
} from "react";

import "./App.css";
import { open } from "@tauri-apps/plugin-dialog";

import {
  getModels,
  streamChatWithModel,
  type AIModel,
} from "./services/ai";

import {
  getProjects,
  createProject,
  saveProject,
  deleteProject,
  listProjectFiles,
  type StoredProject,
  type ProjectFile,
} from "./services/projectStore";

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

function buildFileTree(
  files: ProjectFile[],
): FileTreeNode[] {
  const root: FileTreeNode[] = [];

  for (const file of files) {
    const parts = file.path
      .split("/")
      .filter(Boolean);

    let current = root;
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

  sortNodes(root);

  return root;
}

export default function App() {
  const [models, setModels] = useState<AIModel[]>([]);
  const [selectedModel, setSelectedModel] =
    useState("qwen/qwen3.5-9b");

  const [projects, setProjects] =
    useState<StoredProject[]>([]);

  const [selectedProjectId, setSelectedProjectId] =
    useState<string | null>(null);

  const [messages, setMessages] =
    useState<ChatMessage[]>([]);

  const [projectFiles, setProjectFiles] =
    useState<ProjectFile[]>([]);

  const [filesLoading, setFilesLoading] =
    useState(false);

  const [expandedFolders, setExpandedFolders] =
    useState<Set<string>>(new Set());

  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [modelsLoading, setModelsLoading] =
    useState(true);

  const chatEndRef =
    useRef<HTMLDivElement | null>(null);

  const abortControllerRef =
    useRef<AbortController | null>(null);

  useEffect(() => {
    loadModels();
    loadStoredProjects();
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
          (model) =>
            model.id === selectedModel,
        )
      ) {
        setSelectedModel(
          availableModels[0].id,
        );
      }

      setError("");
    } catch (err) {
      console.error(
        "Failed to load LM Studio models:",
        err,
      );

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
      setExpandedFolders(new Set());
      return;
    }

    setFilesLoading(true);

    try {
      const files =
        await listProjectFiles(project);

      setProjectFiles(files);

      setExpandedFolders(new Set());

      setError("");
    } catch (err) {
      console.error(
        "Failed to load project files:",
        err,
      );

      setProjectFiles([]);
      setExpandedFolders(new Set());

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

      if (storedProjects.length > 0) {
        const firstProject =
          storedProjects[0];

        setSelectedProjectId(
          firstProject.id,
        );

        setMessages(
          firstProject.messages.map(
            (message) => ({
              role: message.role,
              content: message.content,
            }),
          ),
        );

        await loadProjectFiles(
          firstProject,
        );
      } else {
        setProjectFiles([]);
      }
    } catch (err) {
      console.error(
        "Failed to load projects:",
        err,
      );

      setProjects([]);
      setSelectedProjectId(null);
      setMessages([]);
      setProjectFiles([]);
    }
  }

  async function handleCreateProject() {
    const name = window.prompt(
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

      setMessages([]);
      setProjectFiles([]);
      setExpandedFolders(new Set());
      setError("");
    } catch (err) {
      console.error(
        "Failed to create project:",
        err,
      );

      setError(
        err instanceof Error
          ? err.message
          : "Failed to create project.",
      );
    }
  }

  function handleSelectProject(
    project: StoredProject,
  ) {
    setSelectedProjectId(project.id);

    setMessages(
      project.messages.map((message) => ({
        role: message.role,
        content: message.content,
      })),
    );

    void loadProjectFiles(project);

    setError("");
  }

  async function handleOpenProjectFolder() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Project Folder",
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
          .pop() ||
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
          folder_path: folderPath,
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

      setMessages([]);
      setError("");

      await loadProjectFiles(
        projectWithPath,
      );
    } catch (err) {
      console.error(
        "Failed to open project folder:",
        err,
      );

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
      await deleteProject(project.id);

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
        if (remaining.length > 0) {
          handleSelectProject(
            remaining[0],
          );
        } else {
          setSelectedProjectId(null);
          setMessages([]);
          setProjectFiles([]);
          setExpandedFolders(
            new Set(),
          );
        }
      }
    } catch (err) {
      console.error(
        "Failed to delete project:",
        err,
      );

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
    setExpandedFolders((current) => {
      const next = new Set(current);

      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }

      return next;
    });
  }

  function handleFileClick(
    file: FileTreeNode,
  ) {
    if (file.isDirectory) {
      toggleFolder(file.path);
      return;
    }

    console.log(
      "Selected file:",
      file.path,
    );
  }

  async function sendMessage() {
    const text = input.trim();

    if (!text || loading) {
      return;
    }

    setError("");

    const userMessage: ChatMessage = {
      role: "user",
      content: text,
    };

    const conversation = [
      ...messages,
      userMessage,
    ];

    setMessages([
      ...conversation,
      {
        role: "assistant",
        content: "",
      },
    ]);

    setInput("");
    setLoading(true);

    const controller =
      new AbortController();

    abortControllerRef.current =
      controller;

    let assistantResponse = "";

    try {
      await streamChatWithModel(
        selectedModel,
        conversation.map(
          (message) => ({
            role: message.role,
            content: message.content,
          }),
        ),
        (chunk) => {
          assistantResponse += chunk;

          setMessages([
            ...conversation,
            {
              role: "assistant",
              content:
                assistantResponse,
            },
          ]);
        },
        controller.signal,
      );

      const finalMessages: ChatMessage[] =
        assistantResponse
          ? [
              ...conversation,
              {
                role: "assistant",
                content:
                  assistantResponse,
              },
            ]
          : conversation;

      setMessages(finalMessages);

      if (selectedProjectId) {
        const existingProject =
          projects.find(
            (project) =>
              project.id ===
              selectedProjectId,
          );

        if (existingProject) {
          const updatedProject: StoredProject =
            {
              ...existingProject,
              messages: finalMessages,
              updated_at:
                new Date().toISOString(),
            };

          await saveProject(
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
      }
    } catch (err) {
      if (
        err instanceof DOMException &&
        err.name === "AbortError"
      ) {
        console.log(
          "Generation stopped by user.",
        );

        if (assistantResponse) {
          setMessages([
            ...conversation,
            {
              role: "assistant",
              content:
                assistantResponse,
            },
          ]);
        } else {
          setMessages(
            conversation,
          );
        }

        return;
      }

      if (
        err instanceof Error &&
        err.name === "AbortError"
      ) {
        console.log(
          "Generation stopped by user.",
        );

        return;
      }

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
      abortControllerRef.current =
        null;

      setLoading(false);
    }
  }

  function stopGeneration() {
    if (
      abortControllerRef.current
    ) {
      abortControllerRef.current.abort();

      abortControllerRef.current =
        null;
    }
  }

  function handleKeyDown(
    event: React.KeyboardEvent,
  ) {
    if (
      event.key === "Enter" &&
      !event.shiftKey
    ) {
      event.preventDefault();
      sendMessage();
    }
  }

  const modelName =
    models.find(
      (model) =>
        model.id === selectedModel,
    )?.id ?? selectedModel;

  const selectedProject =
    projects.find(
      (project) =>
        project.id ===
        selectedProjectId,
    );

  const currentProjectName =
    selectedProject?.name ??
    "MyApp";

  const fileTree =
    buildFileTree(projectFiles);

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
    <div className="app">
      <header className="topbar">
        <div className="brand">
          ◈
          <div>
            LOCAL AI
          </div>
        </div>

        <button className="project-select">
          {currentProjectName}
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
            value={selectedModel}
            onChange={(event) =>
              setSelectedModel(
                event.target.value,
              )
            }
            disabled={modelsLoading}
          >
            {models.length === 0 ? (
              <option
                value={selectedModel}
              >
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

      <div className="workspace">
        <aside className="left-sidebar">
          <button
            className="new-project"
            onClick={
              handleCreateProject
            }
          >
            <Plus size={15} />
            New Project
          </button>

          <Section title="PROJECTS">
            {projects.length > 0
              ? projects.map(
                  (project) => (
                    <div
                      key={project.id}
                      className={`sidebar-row ${
                        project.id ===
                        selectedProjectId
                          ? "selected"
                          : ""
                      }`}
                      onClick={() =>
                        handleSelectProject(
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
                        {project.name}
                      </span>
                    </div>
                  ),
                )
              : fallbackProjects.map(
                  (project, index) => (
                    <div
                      key={project}
                      className={`sidebar-row ${
                        index === 0
                          ? "selected"
                          : ""
                      }`}
                    >
                      <Circle
                        size={7}
                        fill={
                          index === 0
                            ? "#9b5cff"
                            : "transparent"
                        }
                        color={
                          index === 0
                            ? "#9b5cff"
                            : "#555b68"
                        }
                      />

                      <span>
                        {project}
                      </span>
                    </div>
                  ),
                )}

            {selectedProject && (
              <button
                className="sidebar-row muted"
                onClick={() =>
                  handleDeleteProject(
                    selectedProject,
                  )
                }
              >
                Delete current project
              </button>
            )}

            <div
              className="sidebar-row muted"
              onClick={
                handleCreateProject
              }
            >
              <Plus size={13} />
              Add Project
            </div>
          </Section>

          <div
            className="sidebar-row muted"
            onClick={
              handleOpenProjectFolder
            }
          >
            <FolderOpen size={13} />
            Open Project Folder
          </div>

          <Section title="RECENT">
            {recent.map((item) => (
              <div
                className="sidebar-row"
                key={item}
              >
                <History size={13} />
                {item}
              </div>
            ))}
          </Section>

          <Section title="TASKS">
            {tasks.map((item) => (
              <div
                className="sidebar-row"
                key={item}
              >
                <Circle size={8} />
                {item}
              </div>
            ))}
          </Section>

          <Section title="TOOLS">
            <div className="tools">
              <Tool
                icon={
                  <Folder size={15} />
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
                  <Search size={15} />
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
              value="23%"
            />

            <Status
              label="RAM"
              value="8.1 / 16GB"
            />

            <Status
              label="VRAM"
              value="4.5GB"
            />
          </div>
        </aside>

        <main className="center">
          <div className="task-header">
            <div>
              <div className="task-title">
                <h1>
                  {currentProjectName}
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
                onClick={
                  stopGeneration
                }
                disabled={!loading}
              >
                <Square
                  size={11}
                  fill="currentColor"
                />

                Stop
              </button>

              <button
                disabled={!loading}
              >
                <Pause size={13} />
                Pause
              </button>
            </div>
          </div>

          <div className="chat">
            {messages.length === 0 && (
              <div className="empty-chat">
                <Bot size={30} />

                <h2>
                  Local AI
                </h2>

                <p>
                  Ask anything about
                  your project.
                </p>

                <small>
                  Running locally
                  through LM Studio
                </small>
              </div>
            )}

            {messages.map(
              (message, index) => (
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
                  Connection error
                </strong>

                <span>
                  {error}
                </span>
              </div>
            )}

            <div
              ref={chatEndRef}
            />
          </div>

          <div className="composer">
            <textarea
              value={input}
              onChange={(event) =>
                setInput(
                  event.target.value,
                )
              }
              onKeyDown={
                handleKeyDown
              }
              placeholder="Ask anything about your project..."
              disabled={loading}
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
              onClick={
                sendMessage
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
              <FolderOpen size={14} />

              {selectedProject?.folder_path ??
                `~/Projects/${currentProjectName}`}
            </div>

            {filesLoading ? (
              <div className="file-loading">
                Scanning project...
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
              value={String(fileCount)}
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
              value={String(fileCount)}
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
              label="Last Commit"
              value="--"
            />
          </Info>

          <div className="timeline">
            <div className="section-title">
              ACTIVITY TIMELINE
            </div>

            <Timeline
              time="11:20"
              text="Task started"
            />

            <Timeline
              time="11:21"
              text="Analyzed project"
            />

            <Timeline
              time="11:24"
              text="Proposed changes"
            />

            <Timeline
              time="11:25"
              text="Simulation ready"
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
              SIMULATION SUMMARY
            </div>

            <p>
              2 files modified
            </p>

            <p>
              3 lines changed
            </p>

            <p>
              0 files deleted
            </p>

            <small>
              Est. time: 2 seconds
            </small>

            <div className="risk">
              <span>●</span>
              Risk level: Low
            </div>
          </div>

          <div>
            <div className="section-title">
              FILES TO BE CHANGED
              (2)
            </div>

            <p>
              <FileCode2 size={12} />
              AuthService.kt
              <small>
                (1 change)
              </small>
            </p>

            <p>
              <FileCode2 size={12} />
              TokenProvider.kt
              <small>
                (2 changes)
              </small>
            </p>
          </div>

          <div>
            <div className="section-title">
              COMMANDS TO EXECUTE
              (2)
            </div>

            <code>
              ./gradlew test
            </code>

            <code>
              ./gradlew build
            </code>
          </div>

          <div className="expected">
            <div className="section-title">
              EXPECTED OUTCOME
            </div>

            <p>
              Authentication flow
              will be more accurate.
              Tests should pass
              after the fix.
            </p>

            <button className="apply-changes">
              Apply Changes
            </button>
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
  depth?: number;
}) {
  return (
    <div>
      {nodes.map((node) => {
        const expanded =
          expandedFolders.has(
            node.path,
          );

        return (
          <div key={node.path}>
            <div
              className={`file-entry ${
                node.isDirectory
                  ? "directory"
                  : "file"
              }`}
              style={{
                paddingLeft:
                  8 + depth * 16,
              }}
              onClick={() =>
                onFileClick(node)
              }
            >
              {node.isDirectory ? (
                <>
                  <span className="tree-arrow">
                    {expanded
                      ? "▾"
                      : "▸"}
                  </span>

                  {expanded ? (
                    <FolderOpen
                      size={13}
                    />
                  ) : (
                    <Folder size={13} />
                  )}
                </>
              ) : (
                <FileCode2 size={13} />
              )}

              <span
                title={node.path}
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
      {label}
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
      {user ? (
        <User size={18} />
      ) : (
        <Bot size={18} />
      )}

      <div className="message-body">
        <div className="message-name">
          {user ? "You" : "AI"}
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
    <div className="timeline-item">
      <span className="timeline-time">
        {time}
      </span>

      <span className="timeline-dot">
        ●
      </span>

      <span>{text}</span>
    </div>
  );
}

