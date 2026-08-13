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
import "./App.css";
import { chatWithModel, getModels, type AIModel } from "./services/ai";

const projects = ["MyApp", "Website", "PhishGuard", "DataAnalyzer"];
const recent = ["Fix auth bug", "Improve speed", "Database migration"];
const tasks = ["Fix auth bug", "Add unit tests"];

interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export default function App() {
  const [models, setModels] = useState<AIModel[]>([]);
  const [selectedModel, setSelectedModel] = useState("qwen/qwen3.5-9b");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [modelsLoading, setModelsLoading] = useState(true);

  const chatEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    loadModels();
  }, []);

  useEffect(() => {
    chatEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, loading]);

  async function loadModels() {
    setModelsLoading(true);
    try {
      const availableModels = await getModels();

      setModels(availableModels);

      if (
        availableModels.length > 0 &&
        !availableModels.some((model) => model.id === selectedModel)
      ) {
        setSelectedModel(availableModels[0].id);
      }
    } catch (err) {
      console.error("Failed to load LM Studio models:", err);
      setError("LM Studio is not available.");
    } finally {
      setModelsLoading(false);
    }
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

    const conversation = [...messages, userMessage];

    setMessages(conversation);
    setInput("");
    setLoading(true);

    try {
      const response = await chatWithModel(
        selectedModel,
        conversation.map((message) => ({
          role: message.role,
          content: message.content,
        })),
      );

      setMessages([
        ...conversation,
        {
          role: "assistant",
          content: response,
        },
      ]);
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
    models.find((model) => model.id === selectedModel)?.id ??
    selectedModel;

  return (
    <div className="local-ai">
      {/* TOP BAR */}
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">◈</span>
          <strong>LOCAL AI</strong>
        </div>

        <button className="project-select">
          MyApp
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
            onChange={(event) => setSelectedModel(event.target.value)}
            disabled={modelsLoading}
          >
            {models.length === 0 ? (
              <option value={selectedModel}>
                {selectedModel}
              </option>
            ) : modelsLoading ? (
              <option value="" disabled>
                Loading...
              </option>
            ) : (
              models.map((model) => (
                <option key={model.id} value={model.id}>
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
          <button className="new-project">
            <Plus size={15} />
            New Project
          </button>

          <Section title="PROJECTS">
            {projects.map((project, index) => (
              <div
                key={project}
                className={`sidebar-row ${
                  index === 0 ? "selected" : ""
                }`}
              >
                <Circle
                  size={7}
                  fill={
                    index === 0 ? "#9b5cff" : "transparent"
                  }
                  color={
                    index === 0 ? "#9b5cff" : "#555b68"
                  }
                />
                {project}
              </div>
            ))}

            <div className="sidebar-row muted">
              <Plus size={13} />
              Add Project
            </div>
          </Section>

          <Section title="RECENT">
            {recent.map((item) => (
              <div className="sidebar-row" key={item}>
                <History size={13} />
                {item}
              </div>
            ))}
          </Section>

          <Section title="TASKS">
            {tasks.map((item) => (
              <div className="sidebar-row" key={item}>
                <Circle size={8} />
                {item}
              </div>
            ))}
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

            <Status label="CPU" value="23%" />
            <Status label="RAM" value="8.1 / 16GB" />
            <Status label="VRAM" value="4.5GB" />
          </div>
        </aside>

        {/* CENTER */}
        <main className="center">
          <div className="task-header">
            <div>
              <div className="task-title">
                <h1>Local AI Workspace</h1>

                <span className="progress">
                  <i />
                  {loading ? "Thinking" : "Ready"}
                </span>
              </div>

              <p className="task-subtitle">
                {modelName}
              </p>
            </div>

            <div className="task-buttons">
              <button
                className="stop"
                onClick={() => setLoading(false)}
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
                  Running locally through LM Studio
                </small>
              </div>
            )}

            {messages.map((message, index) => (
              <Message
                key={`${message.role}-${index}`}
                user={message.role === "user"}
                text={message.content}
              />
            ))}

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
                <strong>Connection error</strong>
                <span>{error}</span>
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
              onClick={sendMessage}
              disabled={!input.trim() || loading}
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
              ~/Projects/MyApp
            </div>

            <Tree name="app" folder>
              <Tree name="src" folder>
                <Tree name="main" folder>
                  <Tree name="kotlin" folder>
                    <Tree name="com.example" folder>
                      <Tree name="AuthService.kt" />
                      <Tree name="TokenProvider.kt" />
                      <Tree name="LoginActivity.kt" />
                    </Tree>
                  </Tree>
                </Tree>
              </Tree>

              <Tree name="res" folder />
              <Tree name="AndroidManifest.xml" />
              <Tree name="build.gradle" />
            </Tree>
          </div>

          <Info title="PROJECT OVERVIEW">
            <InfoRow
              label="Language"
              value="Kotlin"
            />
            <InfoRow
              label="Framework"
              value="Android"
            />
            <InfoRow
              label="Build Tool"
              value="Gradle"
            />
            <InfoRow
              label="Tests"
              value="JUnit"
            />
          </Info>

          <Info title="PROJECT STATS">
            <InfoRow
              label="Files"
              value="212"
            />
            <InfoRow
              label="Lines"
              value="18,742"
            />
            <InfoRow
              label="Functions"
              value="1,028"
            />
            <InfoRow
              label="Classes"
              value="218"
            />
            <InfoRow
              label="Last Commit"
              value="2h ago"
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

      {/* BOTTOM PANEL */}
      <section className="bottom">
        <div className="bottom-tabs">
          <button className="active">TASKS</button>
          <button>CHANGES</button>
          <button>SIMULATION</button>
          <button>TERMINAL</button>
          <button>LOGS</button>
        </div>

        <div className="bottom-content">
          <div>
            <div className="section-title">
              SIMULATION SUMMARY
            </div>

            <p>2 files modified</p>
            <p>3 lines changed</p>
            <p>0 files deleted</p>

            <small>Est. time: 2 seconds</small>

            <div className="risk">
              <span>●</span>
              Risk level: Low
            </div>
          </div>

          <div>
            <div className="section-title">
              FILES TO BE CHANGED (2)
            </div>

            <p>
              <FileCode2 size={12} />
              AuthService.kt
              <small>(1 change)</small>
            </p>

            <p>
              <FileCode2 size={12} />
              TokenProvider.kt
              <small>(2 changes)</small>
            </p>
          </div>

          <div>
            <div className="section-title">
              COMMANDS TO EXECUTE (2)
            </div>

            <code>./gradlew test</code>
            <code>./gradlew build</code>
          </div>

          <div className="expected">
            <div className="section-title">
              EXPECTED OUTCOME
            </div>

            <p>
              Authentication flow will be more accurate.
              Tests should pass after the fix.
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

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="sidebar-section">
      <div className="section-title">{title}</div>
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
      className={`message ${user ? "user" : "ai"}`}
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

function Tree({
  name,
  folder = false,
  children,
}: {
  name: string;
  folder?: boolean;
  children?: React.ReactNode;
}) {
  return (
    <div className="tree-node">
      <div className="tree-row">
        {folder ? (
          <ChevronRight size={11} />
        ) : (
          <span className="tree-indent" />
        )}

        {folder ? (
          <Folder size={13} className="folder" />
        ) : (
          <FileCode2 size={13} />
        )}

        <span>{name}</span>
      </div>

      {children && (
        <div className="tree-children">
          {children}
        </div>
      )}
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
