import {
  Activity,
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
  Zap,
} from "lucide-react";
import "./App.css";

const projects = ["MyApp", "Website", "PhishGuard", "DataAnalyzer"];
const recent = ["Fix auth bug", "Improve speed", "Database migration"];
const tasks = ["Fix auth bug", "Add unit tests"];

export default function App() {
  return (
    <div className="local-ai">
      {/* TOP BAR */}
      <header className="topbar">
        <div className="brand">
          <div className="brand-icon">
            <Zap size={15} />
          </div>
          <span>LOCAL AI</span>
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
          <button className="model">
            <Bot size={14} />
            Qwen 3.5 9B
            <ChevronDown size={12} />
          </button>

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

      {/* MAIN WORKSPACE */}
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
                className={`sidebar-row ${index === 0 ? "selected" : ""}`}
              >
                <Circle
                  size={7}
                  fill={index === 0 ? "#9b5cff" : "transparent"}
                  color={index === 0 ? "#9b5cff" : "#555b68"}
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
              <Tool icon={<Folder size={15} />} label="Files" />
              <Tool icon={<Terminal size={15} />} label="Terminal" />
              <Tool icon={<Search size={15} />} label="Search" />
              <Tool icon={<GitBranch size={15} />} label="Git" />
            </div>
          </Section>

          <div className="system-status">
            <div className="section-title">SYSTEM STATUS</div>
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
                <h1>Fix authentication bug</h1>
                <span className="progress">
                  <i />
                  In Progress
                </span>
              </div>

              <p className="task-subtitle">
                AI is working on your project
              </p>
            </div>

            <div className="task-buttons">
              <button className="stop">
                <Square size={11} fill="currentColor" />
                Stop
              </button>

              <button>
                <Pause size={13} />
                Pause
              </button>
            </div>
          </div>

          <div className="chat">
            <Message
              user
              text="The login is failing. Can you find the problem?"
            />

            <Message
              text="I'll help you fix this. I'll analyze the project and identify where the authentication flow is failing."
            />

            {/* AGENT ACTIVITY */}
            <div className="agent-activity">
              <div className="activity-header">
                <span>
                  <Activity size={14} />
                  Agent Activity
                </span>
                <strong>4 / 4</strong>
              </div>

              <AgentStep text="Analyzing project... (212 files)" />
              <AgentStep text="Scanning files..." />
              <AgentStep text="Reading AuthService.kt" />
              <AgentStep text="Identifying issue..." />
            </div>

            <Message text="I found a potential issue. The token expiration time is being calculated incorrectly in AuthService.kt." />

            {/* PROPOSED CHANGES */}
            <div className="proposed">
              <div className="proposed-header">
                <div>
                  <strong>Proposed Changes</strong>
                  <span>2 files</span>
                </div>

                <button className="show-diff">
                  Show Diff
                  <ChevronRight size={13} />
                </button>
              </div>

              <div className="changed-files">
                <span>
                  <FileCode2 size={13} />
                  AuthService.kt
                </span>

                <span>
                  <FileCode2 size={13} />
                  TokenProvider.kt
                </span>
              </div>
            </div>

            <Message user text="Show me the simulation first." />

            <Message text="Sure. I'm running the simulation now without modifying your files." />
          </div>

          {/* CENTER DIFF / TERMINAL */}
          <div className="center-lower">
            <div className="diff-header">
              <span>DIFF: AuthService.kt</span>
              <div>
                <button>Split</button>
                <button>Unified</button>
              </div>
            </div>

            <div className="diff">
              <div className="diff-old">
                <div className="diff-title red">REMOVED</div>
                <code>45 | if (expiry &gt; now)</code>
                <code>46 | throw Exception</code>
                <code>47 | {"}"}</code>
                <code>49 | return isValid</code>
              </div>

              <div className="diff-new">
                <div className="diff-title green">ADDED</div>
                <code>45 | if (expiry &lt; now)</code>
                <code>46 | throw Exception</code>
                <code>47 | {"}"}</code>
                <code>49 | return tokenIsValid</code>
              </div>
            </div>

            <div className="diff-actions">
              <button className="reject">Reject</button>
              <button className="apply">Apply</button>
            </div>

            <div className="terminal">
              <div className="terminal-title">
                <span>
                  <Terminal size={13} />
                  TERMINAL
                </span>
                <span>bash ▾</span>
              </div>

              <div className="terminal-output">
                <span>user@myapp:~/Projects/MyApp $ ./gradlew test</span>
                <span className="terminal-error">
                  &gt; Task :app:test FAILED (3 errors)
                </span>
              </div>
            </div>
          </div>

          <div className="composer">
            <span>Ask anything about your project...</span>

            <button>
              <Wrench size={14} />
            </button>

            <button className="send">
              <Play size={13} fill="currentColor" />
            </button>
          </div>
        </main>

        {/* RIGHT PANEL */}
        <aside className="right-sidebar">
          <div className="right-tabs">
            <button className="active">FILES</button>
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
            <InfoRow label="Language" value="Kotlin" />
            <InfoRow label="Framework" value="Android" />
            <InfoRow label="Build Tool" value="Gradle" />
            <InfoRow label="Tests" value="JUnit" />
          </Info>

          <Info title="PROJECT STATS">
            <InfoRow label="Files" value="212" />
            <InfoRow label="Lines" value="18,742" />
            <InfoRow label="Functions" value="1,028" />
            <InfoRow label="Classes" value="218" />
            <InfoRow label="Last Commit" value="2h ago" />
          </Info>

          <div className="timeline">
            <div className="section-title">ACTIVITY TIMELINE</div>

            <Timeline time="11:20" text="Task started" />
            <Timeline time="11:21" text="Analyzed project" />
            <Timeline time="11:24" text="Proposed changes" />
            <Timeline time="11:25" text="Simulation ready" />
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
            <div className="section-title">SIMULATION SUMMARY</div>
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
              <FileCode2 size={12} /> AuthService.kt
              <small>(1 change)</small>
            </p>

            <p>
              <FileCode2 size={12} /> TokenProvider.kt
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
            <div className="section-title">EXPECTED OUTCOME</div>

            <p>
              Authentication flow will be more accurate. Tests should pass
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
    <button className="tool">
      {icon}
      <span>{label}</span>
    </button>
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
    <div className="status">
      <span>
        <i />
        {label}
      </span>
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
    <div className={`message ${user ? "user" : "ai"}`}>
      <div className="message-avatar">
        {user ? <User size={13} /> : <Bot size={13} />}
      </div>

      <div>
        <div className="message-name">{user ? "You" : "AI"}</div>
        <p>{text}</p>
      </div>
    </div>
  );
}

function AgentStep({ text }: { text: string }) {
  return (
    <div className="agent-step">
      <span className="check">✓</span>
      <span>{text}</span>
      <small>Done</small>
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
    <div className="tree">
      <div className="tree-row">
        {folder ? (
          <ChevronRight size={12} />
        ) : (
          <span className="tree-space" />
        )}

        {folder ? (
          <Folder size={13} className="folder" />
        ) : (
          <FileCode2 size={13} />
        )}

        <span>{name}</span>
      </div>

      {children && <div className="tree-children">{children}</div>}
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
    <section className="info">
      <div className="section-title">{title}</div>
      {children}
    </section>
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
      <i />
      <span>{text}</span>
    </div>
  );
}
