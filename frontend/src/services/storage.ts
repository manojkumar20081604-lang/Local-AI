export interface StoredMessage {
  role: "user" | "assistant";
  content: string;
}

export interface StoredChat {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  messages: StoredMessage[];
}

export interface StoredProject {
  id: string;
  name: string;
  createdAt: number;
  updatedAt: number;
  chats: StoredChat[];
}

const STORAGE_KEY = "local-ai-projects";

export function loadProjects(): StoredProject[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);

    if (!raw) {
      return [];
    }

    const parsed = JSON.parse(raw);

    if (!Array.isArray(parsed)) {
      return [];
    }

    return parsed as StoredProject[];
  } catch (error) {
    console.error("Failed to load projects:", error);
    return [];
  }
}

export function saveProjects(
  projects: StoredProject[],
): void {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify(projects),
    );
  } catch (error) {
    console.error("Failed to save projects:", error);
  }
}

export function createProject(
  name: string,
): StoredProject {
  const now = Date.now();

  return {
    id: crypto.randomUUID(),
    name,
    createdAt: now,
    updatedAt: now,
    chats: [],
  };
}

export function createChat(
  title = "New Chat",
): StoredChat {
  const now = Date.now();

  return {
    id: crypto.randomUUID(),
    title,
    createdAt: now,
    updatedAt: now,
    messages: [],
  };
}
