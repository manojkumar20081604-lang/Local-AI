import {
  Bot,
  ChevronDown,
  ChevronRight,
  Circle,
  FileCode2,
  Folder,
  FolderOpen,
  History,
  Pause,
  Play,
  Plus,
  Square,
  User,
  Wrench,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";

import {
  getModels,
  streamChatWithModel,
} from "./services/ai";

import type { AIModel } from "./services/ai";


import {
  detectProjectIntent,
  getIntentInstructions,
  selectRelevantProjectFiles,
} from "./services/projectIntelligence";

import {
  createProject,
  deleteProject,
  getProjects,
  listProjectFiles,
  readProjectFile,
  deleteProjectFile,
  writeProjectFile,
  runProjectCommand,
  saveProject,
  type ProjectFile,
  type StoredProject,
} from "./services/projectStore";




interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

interface PendingEdit {
  type: "create" | "edit" | "delete";
  filePath: string;
  content?: string;
  search?: string;
  replace?: string;
}




interface EditBackup {
  filePath: string;
  existed: boolean;
  content: string;
}


export default function App() {
  const [models, setModels] = useState<AIModel[]>([]);
  const [selectedModel, setSelectedModel] = useState("qwen/qwen3.5-9b");

  const [projects, setProjects] = useState<StoredProject[]>([]);
  const [selectedProject, setSelectedProject] =
    useState<StoredProject | null>(null);

  const [projectFiles, setProjectFiles] = useState<ProjectFile[]>([]);


  const [pendingEdits, setPendingEdits] =
  useState<PendingEdit[]>([]);
 



 const [lastEditBackups, setLastEditBackups] =
  useState<EditBackup[]>([]);

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [terminalCommand, setTerminalCommand] = useState("");
 const [terminalRunning, setTerminalRunning] = useState(false);
  const [modelsLoading, setModelsLoading] = useState(true);
  const [projectLoading, setProjectLoading] = useState(true);
interface TerminalLine {
  command: string;
  output: string;
  success: boolean;
}

const [terminalHistory, setTerminalHistory] =
  useState<TerminalLine[]>([]);
const [terminalHistoryIndex, setTerminalHistoryIndex] =
  useState(-1);

 const [bottomTab, setBottomTab] = useState<
  "tasks" | "changes" | "simulation" | "terminal" | "logs"
>("tasks"); 

const [rightTab, setRightTab] = useState<
  "files" | "search" | "git"
>("files");

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
    
     console.log(
  "[UNDO] backups:",
  lastEditBackups,
);    

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



async function deleteSelectedProject() {
  if (!selectedProject) {
    return;
  }

  const confirmed = window.confirm(
    `Delete project "${selectedProject.name}"? This will remove the project from Local AI but will NOT delete the actual project folder.`,
  );

  if (!confirmed) {
    return;
  }

  try {
    setError("");

    await deleteProject(selectedProject.id);

    const remainingProjects = projects.filter(
      (project) => project.id !== selectedProject.id,
    );

    setProjects(remainingProjects);

    if (remainingProjects.length > 0) {
      await selectProject(remainingProjects[0]);
    } else {
      setSelectedProject(null);
      setProjectFiles([]);
      setMessages([]);
    }
  } catch (err) {
    console.error("Failed to delete project:", err);

    setError(
      err instanceof Error
        ? err.message
        : "Failed to delete project.",
    );
  }
}



async function buildProjectContext(
  text: string,
): Promise<string> {
  if (!selectedProject || projectFiles.length === 0) {
    return "";
  }

  const intent = detectProjectIntent(text);
  const lowerText = text.toLowerCase();

  const relevantFiles = selectRelevantProjectFiles(
    text,
    projectFiles,
    6,
  );


  const filesToRead = relevantFiles.map((item) => item.file);

  const mentionedFiles = projectFiles.filter((file) => {
    if (file.is_directory) {
      return false;
    }

    const fileName = file.name.toLowerCase();
    const relativePath = file.path.toLowerCase();

    return (
      lowerText.includes(fileName) ||
      lowerText.includes(relativePath)
    );
  });

  /*
   * For now, keep the existing safe behavior:
   * only read explicitly mentioned files.
   *
   * Intent detection is now available to the
   * intelligence layer without dramatically
   * increasing context size.
   */
  const filesToUse =
    mentionedFiles.length > 0
      ? mentionedFiles
      : filesToRead;

  const fileContexts: string[] = [];
  const MAX_FILE_CHARS = 40000; 

  for (const file of filesToUse.slice(0, 6)) {

    try {
          
          const content = await readProjectFile(
  selectedProject,
  file.path,
);

const limitedContent = content.slice(0, MAX_FILE_CHARS);      

      fileContexts.push(
          `FILE: ${file.path}\n\n${limitedContent}`,
      );
    
     console.log(
  "[Project Intelligence] Read file:",
  file.path,
  "chars:",
  content.length,
);
  
    } catch (err) {
      console.error(
        `Failed to read ${file.path}:`,
        err,
      );
    }
  }

  if (fileContexts.length === 0) {
    return "";
  }

  return `
PROJECT INTENT:

${intent.intent}

CONFIDENCE:

${intent.confidence.toFixed(2)}

${getIntentInstructions(intent.intent)}

The user is asking about files from the attached project.

Use the following REAL project files as authoritative context.

${fileContexts.join(
  "\n\n==============================\n\n",
)}
`;
}


function parseEditResponses(response: string): PendingEdit[] {
  const proposals: PendingEdit[] = [];

  const createRegex =
    /<CREATE_FILE>\s*FILE:\s*(.+?)\s*CONTENT:\s*\n([\s\S]*?)\s*<\/CREATE_FILE>/gi;

  for (const match of response.matchAll(createRegex)) {
    proposals.push({
      type: "create",
      filePath: match[1].trim(),
      content: match[2],
    });
  }

  const editRegex =
    /<EDIT>\s*FILE:\s*(.+?)\s*SEARCH:\s*\n([\s\S]*?)\s*REPLACE:\s*\n([\s\S]*?)\s*<\/EDIT>/gi;

  for (const match of response.matchAll(editRegex)) {
    proposals.push({
      type: "edit",
      filePath: match[1].trim(),
      search: match[2],
      replace: match[3],
    });
  }

  const deleteRegex =
    /<DELETE>\s*FILE:\s*(.+?)\s*<\/DELETE>/gi;

  for (const match of response.matchAll(deleteRegex)) {
    proposals.push({
      type: "delete",
      filePath: match[1].trim(),
    });
  }

  const deleteEditRegex =
    /<EDIT>\s*FILE:\s*(.+?)\s*ACTION:\s*DELETE\s*<\/EDIT>/gi;

  for (const match of response.matchAll(deleteEditRegex)) {
    proposals.push({
      type: "delete",
      filePath: match[1].trim(),
    });
  }


  return proposals;
}



function rejectPendingEdits() {
  setPendingEdits([]);
  setError("");
}

async function applyPendingEdits() {
  if (pendingEdits.length === 0 || !selectedProject) {
    return;
  }

  try {
    setError("");

    const backups: EditBackup[] = [];

    // Capture the original state of every proposed file.
    for (const pendingEdit of pendingEdits) {
      if (pendingEdit.type === "create") {
        backups.push({
          filePath: pendingEdit.filePath,
          existed: false,
          content: "",
        });

        continue;
      }

      const currentContent = await readProjectFile(
        selectedProject,
        pendingEdit.filePath,
      );

      backups.push({
        filePath: pendingEdit.filePath,
        existed: true,
        content: currentContent,
      });
    }

    try {
      for (const pendingEdit of pendingEdits) {
        // DELETE
        if (pendingEdit.type === "delete") {
          console.log(
            "[DELETE DEBUG] Applying delete:",
            pendingEdit.filePath,
          );

          await deleteProjectFile(
            selectedProject,
            pendingEdit.filePath,
          );

          // Verify the file was actually deleted.
          try {
            await readProjectFile(
              selectedProject,
              pendingEdit.filePath,
            );

            throw new Error(
              `DELETE verification failed: ${pendingEdit.filePath} still exists.`,
            );
          } catch (verificationError) {
            if (
              verificationError instanceof Error &&
              verificationError.message.startsWith(
                "DELETE verification failed:",
              )
            ) {
              throw verificationError;
            }
          }

          console.log(
            "[DELETE DEBUG] Delete verified:",
            pendingEdit.filePath,
          );

          continue;
        }

        // CREATE
        if (pendingEdit.type === "create") {
          await writeProjectFile(
            selectedProject,
            pendingEdit.filePath,
            pendingEdit.content ?? "",
          );

          const verifiedContent = await readProjectFile(
            selectedProject,
            pendingEdit.filePath,
          );

          if (
            verifiedContent !==
            (pendingEdit.content ?? "")
          ) {
            throw new Error(
              `Verification failed for ${pendingEdit.filePath}.`,
            );
          }

          continue;
        }

        // EDIT
        if (
          pendingEdit.search === undefined ||
          pendingEdit.replace === undefined
        ) {
          throw new Error("Invalid edit proposal.");
        }

        const currentContent = await readProjectFile(
          selectedProject,
          pendingEdit.filePath,
        );

        const occurrences =
          currentContent.split(pendingEdit.search).length - 1;

        if (occurrences === 0) {
          throw new Error(
            `The requested text was not found in ${pendingEdit.filePath}.`,
          );
        }

        if (occurrences > 1) {
          throw new Error(
            `The requested text appears ${occurrences} times in ${pendingEdit.filePath}. Refusing to apply an ambiguous edit.`,
          );
        }

        const updatedContent = currentContent.replace(
          pendingEdit.search,
          pendingEdit.replace,
        );

        await writeProjectFile(
          selectedProject,
          pendingEdit.filePath,
          updatedContent,
        );

        const verifiedContent = await readProjectFile(
          selectedProject,
          pendingEdit.filePath,
        );

        if (verifiedContent !== updatedContent) {
          throw new Error(
            `Verification failed for ${pendingEdit.filePath}.`,
          );
        }
      }
    } catch (applyError) {
      console.error(
        "Edit application failed. Restoring previous state:",
        applyError,
      );

      // Restore every file to its state before Apply.
      for (const backup of backups) {
        try {
          if (!backup.existed) {
            // The file did not exist before Apply.
            // Remove it if Apply created it.
            try {
              await readProjectFile(
                selectedProject,
                backup.filePath,
              );

              await deleteProjectFile(
                selectedProject,
                backup.filePath,
              );
            } catch {
              // File is already absent.
            }

            continue;
          }

          // File existed before Apply.
          // Restore its original contents.
          await writeProjectFile(
            selectedProject,
            backup.filePath,
            backup.content,
          );
        } catch (restoreError) {
          console.error(
            "Failed to restore:",
            backup.filePath,
            restoreError,
          );
        }
      }

      throw applyError;
    }

    // Everything succeeded.
    setLastEditBackups(backups);

    const files = await listProjectFiles(selectedProject);
    setProjectFiles(files);

    setPendingEdits([]);
  } catch (err) {
    console.error(
      "Failed to apply edits:",
      err,
    );

    setError(
      err instanceof Error
        ? `APPLY ERROR: ${err.message}`
        : "Failed to apply edits.",
    );
  }
}









async function undoLastEdit() {
  if (!selectedProject || lastEditBackups.length === 0) {
    return;
  }

  try {
    setError("");

    const currentState: EditBackup[] = [];

    // Capture the actual current state before undoing.
    // A deleted file is allowed to be missing here.
    for (const backup of lastEditBackups) {
      try {
        const currentContent = await readProjectFile(
          selectedProject,
          backup.filePath,
        );

        currentState.push({
          filePath: backup.filePath,
          existed: true,
          content: currentContent,
        });
      } catch {
        currentState.push({
          filePath: backup.filePath,
          existed: false,
          content: "",
        });
      }
    }
    try {
      // Apply the undo operation.
      for (const backup of lastEditBackups) {
        if (!backup.existed) {
          await deleteProjectFile(
            selectedProject,
            backup.filePath,
          );

          continue;
        }

        await writeProjectFile(
          selectedProject,
          backup.filePath,
          backup.content,
        );
      }

      // Verify every restored/deleted file.
      for (const backup of lastEditBackups) {
        if (!backup.existed) {
          try {
            await readProjectFile(
              selectedProject,
              backup.filePath,
            );

            throw new Error(
              `Undo verification failed: ${backup.filePath} still exists.`,
            );
          } catch (err) {
            if (
              err instanceof Error &&
              err.message.startsWith(
                "Undo verification failed:",
              )
            ) {
              throw err;
            }
          }

          continue;
        }

        const restoredContent =
          await readProjectFile(
            selectedProject,
            backup.filePath,
          );

        if (restoredContent !== backup.content) {
          throw new Error(
            `Undo verification failed for ${backup.filePath}.`,
          );
        }
      }
    } catch (undoError) {
      console.error(
        "Undo failed. Restoring previous state:",
        undoError,
      );

      // Roll back the partial undo so the project returns
      // to the state it had before Undo was started.
      for (const state of currentState) {
        if (!state.existed) {
          try {
            await deleteProjectFile(
              selectedProject,
              state.filePath,
            );
          } catch {
            // Ignore rollback errors here. The original
            // undo error is more important.
          }

          continue;
        }

        await writeProjectFile(
          selectedProject,
          state.filePath,
          state.content,
        );
      }

      throw undoError;
    }

    const files =
      await listProjectFiles(selectedProject);

    setProjectFiles(files);
    setLastEditBackups([]);
  } catch (err) {
    console.error("Failed to undo edit:", err);

    setError(
      err instanceof Error
        ? err.message
        : "Failed to undo file change.",
    );
  }
}



function renderEditPreview(edit: PendingEdit) {
  if (edit.type === "create") {
    return (
      <pre className="edit-diff">
        <span className="diff-added">
          {edit.content ?? ""}
        </span>
      </pre>
    );
  }

  return (
    <pre className="edit-diff">
      <span className="diff-removed">
        {edit.search ?? ""}
      </span>

      <span className="diff-added">
        {edit.replace ?? ""}
      </span>
    </pre>
  );
}


async function applySinglePendingEdit(index: number) {
  if (!selectedProject) {
    return;
  }

  const pendingEdit = pendingEdits[index];

  if (!pendingEdit) {
    return;
  }

  try {
    setError("");

    let backup: EditBackup;

    if (pendingEdit.type === "delete") {
      const currentContent = await readProjectFile(
        selectedProject,
        pendingEdit.filePath,
      );

      backup = {
        filePath: pendingEdit.filePath,
        existed: true,
        content: currentContent,
      };

      await deleteProjectFile(
        selectedProject,
        pendingEdit.filePath,
      );
    } else if (pendingEdit.type === "create") {
      backup = {
        filePath: pendingEdit.filePath,
        existed: false,
        content: "",
      };

      await writeProjectFile(
        selectedProject,
        pendingEdit.filePath,
        pendingEdit.content ?? "",
      );
    } else {
      if (
        pendingEdit.search === undefined ||
        pendingEdit.replace === undefined
      ) {
        throw new Error("Invalid edit proposal.");
      }

      const currentContent = await readProjectFile(
        selectedProject,
        pendingEdit.filePath,
      );

      backup = {
        filePath: pendingEdit.filePath,
        existed: true,
        content: currentContent,
      };

      const occurrences =
        currentContent.split(pendingEdit.search).length - 1;

      if (occurrences === 0) {
        throw new Error(
          `The requested text was not found in ${pendingEdit.filePath}.`,
        );
      }

      if (occurrences > 1) {
        throw new Error(
          `The requested text appears ${occurrences} times in ${pendingEdit.filePath}. Refusing to apply an ambiguous edit.`,
        );
      }

      const updatedContent = currentContent.replace(
        pendingEdit.search,
        pendingEdit.replace,
      );

      await writeProjectFile(
        selectedProject,
        pendingEdit.filePath,
        updatedContent,
      );
    }


      // Verify the result of the operation.
    if (pendingEdit.type === "delete") {
      try {
        await readProjectFile(
          selectedProject,
          pendingEdit.filePath,
        );

        // If the read succeeds, the file was not deleted.
        throw new Error(
          `DELETE verification failed: ${pendingEdit.filePath} still exists.`,
        );
      } catch (verificationError) {
        if (
          verificationError instanceof Error &&
          verificationError.message.startsWith(
            "DELETE verification failed:",
          )
        ) {
          throw verificationError;
        }

        // Any other read error means the file is gone,
        // which is the expected result of DELETE.
      }
    } else {
      const verifiedContent = await readProjectFile(
        selectedProject,
        pendingEdit.filePath,
      );

      const expectedContent =
        pendingEdit.type === "create"
          ? pendingEdit.content ?? ""
          : backup.content.replace(
              pendingEdit.search ?? "",
              pendingEdit.replace ?? "",
            );

      if (verifiedContent !== expectedContent) {
        throw new Error(
          `Verification failed for ${pendingEdit.filePath}.`,
        );
      }
    }


    // Store backup only after successful verification.
    setLastEditBackups([backup]);

    const files = await listProjectFiles(selectedProject);
    setProjectFiles(files);

    setPendingEdits((previous) =>
      previous.filter(
        (_, itemIndex) => itemIndex !== index,
      ),
    );
  } catch (err) {
    console.error("Failed to apply edit:", err);

    setError(
      err instanceof Error
        ? `APPLY ERROR: ${err.message}`
        : `APPLY ERROR: ${String(err)}`,
    );
  }
}


function rejectSinglePendingEdit(index: number) {
  setPendingEdits((previous) =>
    previous.filter((_, itemIndex) => itemIndex !== index),
  );
}



/**
 * Send a message to the AI model with project context
 * 
 * This function orchestrates the chat flow by:
 * 1. Building project context from relevant files
 * 2. Sending system prompt with Local AI rules
 * 3. Appending conversation history (max 2 messages)
 * 4. Processing AI response for file edits
 */





async function runTerminalCommand() {
  if (!selectedProject || !terminalCommand.trim()) {
    return;
  }

  const command = terminalCommand.trim();

  try {
    setTerminalRunning(true);
    setError("");

    const result = await runProjectCommand(
      selectedProject,
      command,
    );

    const output = [
      result.stdout.trimEnd(),
      result.stderr.trimEnd(),
    ]
      .filter(Boolean)
      .join("\n");

    setTerminalHistory((previous) => [
      ...previous,
      {
        command,
        output: output || "(no output)",
        success: result.success,
      },
    ]);

    setTerminalCommand("");
  } catch (err) {
    const message =
      err instanceof Error
        ? err.message
        : String(err);

    setTerminalHistory((previous) => [
      ...previous,
      {
        command,
        output: message,
        success: false,
      },
    ]);

    setError(message);
    setTerminalCommand("");
  } finally {
    setTerminalRunning(false);
  }
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


const MAX_CONVERSATION_MESSAGES = 2;

const fullMessages = [
  ...messages,
  userMessage,
];

const conversation = [
  ...messages.slice(-MAX_CONVERSATION_MESSAGES),
  userMessage,
];

setMessages(fullMessages);
setInput("");
setLoading(true);

  try {
    const projectContext =
      await buildProjectContext(text);
     
          console.log(
  "[Project Intelligence] Final context:",
  projectContext.length,
  "chars",
);
          

    const aiMessages = [
      {
        role: "system" as const,
content: `
You are Local AI, a local coding assistant.

You are working inside the user's attached project.

IMPORTANT PROJECT RULES:

- Real project files are the source of truth.
- Do not invent file contents.
- Do not invent files or directories.
- Do not claim you cannot access provided project files.
- Analyze the actual contents provided to you.
- When discussing code, mention the actual file path.
- Clearly distinguish facts from suggestions.
- If the provided files are insufficient, say which
  additional file would be useful.
- Keep answers grounded in the provided project context.

FILE EDITING RULES:

When the user explicitly asks you to change project files, you MUST return machine-readable file operation blocks.

The application parses these blocks and shows them to the user for approval. The application, not you, performs the actual file operation.

IMPORTANT:
- Never say you cannot access the project files.
- Never give terminal commands for requested file operations.
- Never give Markdown tables instead of operations.
- Never write a proposal document instead of an operation.
- Never tell the user to manually copy or edit files.

DELETE A FILE:

If the user explicitly asks to delete a file, output exactly:

<DELETE>
FILE: relative/path/to/file
</DELETE>

For deletion:
- Use the exact relative path from the user request or project context.
- Output no git commands.
- Output no explanation.
- Output no proposal document.
- Output no additional text.

EDIT AN EXISTING FILE:

Use exactly:

<EDIT>
FILE: relative/path/to/file
SEARCH:
exact existing text
REPLACE:
replacement text
</EDIT>

SEARCH must exactly match text from the real file provided in project context.

CREATE A FILE:

Use exactly:

<CREATE_FILE>
FILE: relative/path/to/file
CONTENT:
complete new file contents
</CREATE_FILE>

MULTI-FILE CHANGES:

Return one separate operation block for every file that must change.

For normal questions, explanations, reviews, and analysis, do not output any operation blocks.

PROJECT CONTEXT:

FILE OPERATIONS:

When the user asks you to CREATE a new file, you MUST propose it using exactly:

<CREATE_FILE>
FILE: relative/path/to/file
CONTENT:
complete file contents
</CREATE_FILE>

When the user asks you to MODIFY an existing file, you MUST propose it using exactly:

<EDIT>
FILE: relative/path/to/file
SEARCH:
exact existing text
REPLACE:
replacement text
</EDIT>

Rules:
- Use paths relative to the project root.
- For CREATE_FILE, provide the complete contents of the new file.
- For EDIT, SEARCH must exactly match text from the real file, including all whitespace and indentation.
- Do not invent existing file contents.
- Do not tell the user to manually copy and paste code.
- Do not merely describe the change.
- Actually return the CREATE_FILE or EDIT operation.
- The user must approve the operation before it is applied.

${projectContext}
`,
      },

      ...conversation.map((message) => ({
        role: message.role,
        content: message.content,
      })),
    ];

let response = "";

setMessages([
  ...fullMessages,
  {
    role: "assistant",
    content: "",
  },
]);

await streamChatWithModel(
  selectedModel,
  aiMessages,
  (chunk) => {
    response += chunk;
setMessages([
  ...fullMessages,
  {
    role: "assistant",
    content: response,
  },
]);

  },
);

const proposedEdits = parseEditResponses(response);
if (proposedEdits.length > 0) {
  setPendingEdits(proposedEdits);
}

const updatedMessages: ChatMessage[] = [
  ...fullMessages,
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

    <div className="brand-copy">
      <strong>LOCAL AI</strong>
      <span>Your AI Project Workspace</span>
    </div>
  </div>

  <button className="project-select">
    <Folder size={14} />
    <span>
      <strong>{projectName}</strong>
      <small>
        {selectedProject?.folder_path ?? "No project attached"}
      </small>
    </span>
    <ChevronDown size={13} />
  </button>


  <div className="top-right">
    <div className="model-status">
      <span className="status-dot" />
      <span>Model:</span>

      <select
        className="model"
        value={selectedModel}
        onChange={(event) => {
          setSelectedModel(event.target.value);
        }}
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
    </div>




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
           
{selectedProject && (
  <div
    className="sidebar-row muted"
    onClick={deleteSelectedProject}
    style={{
      cursor: "pointer",
      color: "#ff6b6b",
    }}
  >
    <Square size={12} />
    Delete Project
  </div>
)}

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
                            project.messages.length - 1
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
  onClick={() => selectProject(item.project)}
  style={{ cursor: "pointer" }}
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

  <button type="button">
    <Pause size={13} />
    Pause
  </button>
   <button
  type="button"
  className="undo-button"
  onClick={undoLastEdit}
  disabled={lastEditBackups.length === 0}
>
  Undo
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


    {pendingEdits.length > 0 && (
  <div className="edit-proposals">
    <div className="edit-proposal-header">
      <div>
        <strong>
          {`AI proposed ${pendingEdits.length} file${
            pendingEdits.length !== 1 ? "s" : ""
          }`}
        </strong>

        <span>Review and approve each change</span>
      </div>

      <div className="edit-proposal-actions">
        <button
          type="button"
          onClick={rejectPendingEdits}
        >
          Reject All
        </button>

        <button
          type="button"
          className="apply-edit"
          onClick={applyPendingEdits}
        >
          Apply All
        </button>
      </div>
    </div>

    <div className="edit-list">
      {pendingEdits.map((edit, index) => (
        <div
          key={`${edit.filePath}-${index}`}
          className="edit-item"
        >
          <div className="edit-file-path">
            <span>{edit.filePath}</span>

            <div className="edit-item-actions">
              <button
                type="button"
                onClick={() =>
                  rejectSinglePendingEdit(index)
                }
              >
                Reject
              </button>

              <button
                type="button"
                className="apply-edit"
                onClick={() =>
                  applySinglePendingEdit(index)
                }
              >
                Apply
              </button>
            </div>
          </div>
           
         {renderEditPreview(edit)}

        </div>
      ))}
    </div>
  </div>
)}     


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
                  ? "Ask Local AI anything about your project..."
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
    <button
      className={rightTab === "files" ? "active" : ""}
      onClick={() => setRightTab("files")}
    >
      FILES
    </button>

  </div>

  {rightTab === "files" && (
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
  )}

  {rightTab === "search" && (
    <div className="file-tree">
      <div className="section-title">
        PROJECT SEARCH
      </div>

      <p>
        Search will be available here.
      </p>

      <small>
        Search across the attached project files.
      </small>
    </div>
  )}

  {rightTab === "git" && (
    <div className="file-tree">
      <div className="section-title">
        GIT
      </div>

      <p>
        Git tools will be available here.
      </p>

      <small>
        Git tools will be available here.
      </small>
    </div>
  )}

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
    <button
      className={bottomTab === "tasks" ? "active" : ""}
      onClick={() => setBottomTab("tasks")}
    >
      TASKS
    </button>

    <button
      className={bottomTab === "changes" ? "active" : ""}
      onClick={() => setBottomTab("changes")}
    >
      CHANGES
    </button>

    <button
      className={bottomTab === "simulation" ? "active" : ""}
      onClick={() => setBottomTab("simulation")}
    >
      SIMULATION
    </button>

    <button
      className={bottomTab === "terminal" ? "active" : ""}
      onClick={() => setBottomTab("terminal")}
    >
      TERMINAL
    </button>

    <button
      className={bottomTab === "logs" ? "active" : ""}
      onClick={() => setBottomTab("logs")}
    >
      LOGS
    </button>
  </div>

  <div className="bottom-content">
    {bottomTab === "tasks" && (
      <>
        <div>
          <div className="section-title">
            PROJECT
          </div>

          <p>
            {selectedProject?.name ??
              "No project selected"}
          </p>

          <small>
            {projectFiles.length} items loaded
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
            {loading ? "Generating..." : "Ready"}
          </code>
        </div>

        <div className="expected">
          <div className="section-title">
            NEXT
          </div>

          <p>
            Simulation will preview AI changes
            before they are applied.
          </p>
        </div>
      </>
    )}

    {bottomTab === "changes" && (
      <div>
        <div className="section-title">
          PENDING CHANGES
        </div>

        <p>
          {pendingEdits.length === 0
            ? "No pending changes."
            : `${pendingEdits.length} pending change${
                pendingEdits.length !== 1 ? "s" : ""
              }.`}
        </p>
      </div>
    )}

    

{bottomTab === "simulation" && (
  <div>
    <div className="section-title">
      SIMULATION
    </div>

    {pendingEdits.length === 0 ? (
      <div>
        <p>No changes to simulate.</p>

        <small>
          Ask the AI to edit, create, or delete a project file.
          The proposed operation will appear here before it is applied.
        </small>
      </div>
    ) : (
      <>
        <p>
          Preview only — no files will be modified.
        </p>

        <div>
          {pendingEdits.map((edit, index) => (
            <div
              key={`${edit.filePath}-${index}`}
              style={{
                marginBottom: "10px",
                padding: "10px",
                border: "1px solid rgba(255,255,255,0.08)",
                borderRadius: "6px",
              }}
            >
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  marginBottom: "6px",
                }}
              >
                <strong>
                  {edit.type.toUpperCase()}
                </strong>

                <span>
                  {edit.filePath}
                </span>
              </div>

              {edit.type === "delete" && (
                <small>
                  This file would be deleted.
                </small>
              )}

              {edit.type === "create" && (
                <small>
                  A new file would be created with the proposed content.
                </small>
              )}

              {edit.type === "edit" && (
                <div>
                  <small>
                    The following text would be replaced:
                  </small>

                  <pre className="edit-diff">
                    <span className="diff-removed">
                      {edit.search ?? ""}
                    </span>

                    <span className="diff-added">
                      {edit.replace ?? ""}
                    </span>
                  </pre>
                </div>
              )}
            </div>
          ))}
        </div>

        <small>
          Simulation is read-only. Use Apply or Apply All above to
          actually modify the project.
        </small>
      </>
    )}
  </div>
)}




{bottomTab === "terminal" && (
  <>
    <div className="terminal-header">
      <div className="section-title">
        TERMINAL
      </div>

      <div>
        <small>
          {selectedProject?.folder_path ??
            "No project folder"}
        </small>

        <button
          type="button"
          onClick={() => setTerminalHistory([])}
          style={{
            marginLeft: "10px",
            padding: "3px 8px",
            fontSize: "11px",
          }}
        >
          CLEAR
        </button>
      </div>
    </div>

    <div className="terminal-screen">
      {terminalHistory.map((entry, index) => (
        <div
          className="terminal-entry"
          key={`${entry.command}-${index}`}
        >
          <div className="terminal-command">
            <span className="terminal-prompt">
              $
            </span>

            <span>{entry.command}</span>
          </div>

          <pre
            className={
              entry.success
                ? "terminal-result"
                : "terminal-result terminal-error"
            }
          >
            {entry.output}
          </pre>
        </div>
      ))}

      {selectedProject && (
        <div className="terminal-input-line">
          <span className="terminal-prompt">
            $
          </span>

          

<input
  autoFocus
  value={terminalCommand}
  onChange={(event) => {
    setTerminalCommand(event.target.value);
    setTerminalHistoryIndex(-1);
  }}
  onKeyDown={(event) => {
    if (
      event.key === "Enter" &&
      !terminalRunning
    ) {
      runTerminalCommand();
      return;
    }

    if (event.key === "ArrowUp") {
      event.preventDefault();

      if (terminalHistory.length === 0) {
        return;
      }

      const nextIndex =
        terminalHistoryIndex === -1
          ? terminalHistory.length - 1
          : Math.max(
              0,
              terminalHistoryIndex - 1,
            );

      setTerminalHistoryIndex(nextIndex);
      setTerminalCommand(
        terminalHistory[nextIndex].command,
      );

      return;
    }

    if (event.key === "ArrowDown") {
      event.preventDefault();

      if (terminalHistoryIndex === -1) {
        return;
      }

      const nextIndex =
        terminalHistoryIndex + 1;

      if (nextIndex >= terminalHistory.length) {
        setTerminalHistoryIndex(-1);
        setTerminalCommand("");
        return;
      }

      setTerminalHistoryIndex(nextIndex);
      setTerminalCommand(
        terminalHistory[nextIndex].command,
      );
    }
  }}
  disabled={terminalRunning}
  placeholder={
    terminalRunning
      ? "Running..."
      : ""
  }
/>



        </div>
      )}
    </div>
  </>
)}
  </div>
</section>
</div> 
);

}

interface FileTreeNode {
  name: string;
  path: string;
  is_directory: boolean;
  children: FileTreeNode[];
}


function ProjectFileTree({
  files,
}: {
  files: ProjectFile[];
}) {
  const tree = buildTree(files);

  return (
    <div className="project-file-tree">
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
    <div className={`message ${user ? "user" : "ai"}`}>
      <div className="message-avatar">
        {user ? <User size={15} /> : <Bot size={16} />}
      </div>

      <div className="message-body">
        <div className="message-name">
          {user ? "You" : "AI"}
        </div>

        {user ? (
          <p>{text}</p>
        ) : (
          <AgentMessageContent text={text} />
        )}
      </div>
    </div>
  );
}

function AgentMessageContent({
  text,
}: {
  text: string;
}) {
  const lines = text.split(String.fromCharCode(10));
  let inCodeBlock = false;
  let codeLines: string[] = [];

  const elements: React.ReactNode[] = [];

  function flushCode() {
    if (codeLines.length === 0) return;

    elements.push(
      <pre
        key={`code-${elements.length}`}
        className="agent-code-block"
      >
        <code>{codeLines.join(String.fromCharCode(10))}</code>
      </pre>,
    );

    codeLines = [];
  }

  lines.forEach((line, index) => {
    if (line.trim().startsWith("```")) {
      if (inCodeBlock) {
        flushCode();
        inCodeBlock = false;
      } else {
        inCodeBlock = true;
      }

      return;
    }

    if (inCodeBlock) {
      codeLines.push(line);
      return;
    }

    const trimmed = line.trim();

    if (!trimmed) {
      elements.push(
        <div
          key={`space-${index}`}
          className="agent-spacer"
        />,
      );
      return;
    }

    if (trimmed.startsWith("### ")) {
      elements.push(
        <h4
          key={index}
          className="agent-heading small"
        >
          {trimmed.slice(4)}
        </h4>,
      );
      return;
    }

    if (trimmed.startsWith("## ")) {
      elements.push(
        <h3
          key={index}
          className="agent-heading"
        >
          {trimmed.slice(3)}
        </h3>,
      );
      return;
    }

    if (trimmed.startsWith("# ")) {
      elements.push(
        <h2
          key={index}
          className="agent-heading large"
        >
          {trimmed.slice(2)}
        </h2>,
      );
      return;
    }

    if (
      trimmed.startsWith("- ") ||
      trimmed.startsWith("* ")
    ) {
      elements.push(
        <div
          key={index}
          className="agent-list-item"
        >
          <span className="agent-bullet">•</span>
          <span>{trimmed.slice(2)}</span>
        </div>,
      );
      return;
    }

    if (/^\d+\.\s/.test(trimmed)) {
      const match = trimmed.match(/^(\d+)\.\s(.*)$/);

      elements.push(
        <div
          key={index}
          className="agent-list-item"
        >
          <span className="agent-number">
            {match?.[1]}
          </span>

          <span>
            {match?.[2] ?? trimmed}
          </span>
        </div>,
      );
      return;
    }

    if (
      trimmed.startsWith("<EDIT>") ||
      trimmed.startsWith("<CREATE_FILE>")
    ) {
      elements.push(
        <div
          key={index}
          className="agent-operation"
        >
          {trimmed}
        </div>,
      );
      return;
    }

    if (
      trimmed.startsWith("FILE:") ||
      trimmed.startsWith("SEARCH:") ||
      trimmed.startsWith("REPLACE:") ||
      trimmed.startsWith("CONTENT:")
    ) {
      elements.push(
        <div
          key={index}
          className="agent-code-line"
        >
          {trimmed}
        </div>,
      );
      return;
    }

    if (
      trimmed === "Analysis" ||
      trimmed === "Summary" ||
      trimmed === "Key Features" ||
      trimmed === "Safety Constraints"
    ) {
      elements.push(
        <div
          key={index}
          className="agent-section"
        >
          {trimmed}
        </div>,
      );
      return;
    }

    elements.push(
      <p key={index}>{line}</p>,
    );
  });

  if (inCodeBlock) {
    flushCode();
  }

  return (
    <div className="agent-message-content">
      {elements}
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

