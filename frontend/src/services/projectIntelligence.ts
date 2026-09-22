export type ProjectIntent =
  | "general"
  | "explain"
  | "debug"
  | "find"
  | "architecture"
  | "edit"
  | "review"
  | "run"
  | "unknown";

export interface IntentResult {
  intent: ProjectIntent;
  confidence: number;
  keywords: string[];
}

export interface ProjectFileLike {
  name: string;
  path: string;
  is_directory: boolean;
}

export interface RelevantFile {
  file: ProjectFileLike;
  score: number;
  reasons: string[];
}

const INTENT_RULES: Array<{
  intent: ProjectIntent;
  keywords: string[];
}> = [
  {
    intent: "debug",
    keywords: [
      "error",
      "bug",
      "broken",
      "crash",
      "fails",
      "failed",
      "failure",
      "exception",
      "not working",
      "doesn't work",
      "doesnt work",
      "issue",
      "problem",
      "fix",
    ],
  },
  {
    intent: "edit",
    keywords: [
      "change",
      "modify",
      "update",
      "edit",
      "replace",
      "add",
      "remove",
      "delete",
      "implement",
      "make it",
      "write",
      "create",
    ],
  },
  {
    intent: "explain",
    keywords: [
      "explain",
      "what does",
      "what is",
      "how does",
      "why does",
      "understand",
      "meaning",
      "describe",
    ],
  },
  {
    intent: "find",
    keywords: [
      "find",
      "where",
      "which file",
      "locate",
      "search",
      "show me",
      "look for",
    ],
  },
  {
    intent: "architecture",
    keywords: [
      "architecture",
      "structure",
      "project structure",
      "how is the project",
      "components",
      "dependencies",
      "flow",
      "frontend",
      "backend",
      "database",
      "api",
    ],
  },
  {
    intent: "review",
    keywords: [
      "review",
      "audit",
      "improve",
      "improvements",
      "quality",
      "clean",
      "refactor",
      "optimize",
      "optimization",
    ],
  },
  {
    intent: "run",
    keywords: [
      "run",
      "build",
      "compile",
      "test",
      "start",
      "launch",
      "command",
      "terminal",
    ],
  },
];

export function detectProjectIntent(
  text: string,
): IntentResult {
  const normalized = text.toLowerCase().trim();

  if (!normalized) {
    return {
      intent: "unknown",
      confidence: 0,
      keywords: [],
    };
  }

  let bestIntent: ProjectIntent = "general";
  let bestScore = 0;
  let matchedKeywords: string[] = [];

  for (const rule of INTENT_RULES) {
    const matches = rule.keywords.filter((keyword) =>
      normalized.includes(keyword),
    );

    if (matches.length > bestScore) {
      bestScore = matches.length;
      bestIntent = rule.intent;
      matchedKeywords = matches;
    }
  }

  if (bestScore === 0) {
    return {
      intent: "general",
      confidence: 0.2,
      keywords: [],
    };
  }

  const confidence = Math.min(
    0.95,
    0.45 + bestScore * 0.15,
  );

  return {
    intent: bestIntent,
    confidence,
    keywords: matchedKeywords,
  };
}

const IGNORED_PATH_PARTS = new Set([
  "node_modules", ".git", "dist", "build", "target", ".next", ".vite", "coverage", ".cache", "__pycache__", ".venv", "venv", ".idea", ".vscode",
]);

const GENERATED_FILE_NAMES = new Set([
  "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "cargo.lock",
]);

const CORE_FILE_NAMES = new Set([
  "app.tsx", "app.ts", "main.tsx", "main.ts", "index.tsx", "index.ts", "main.rs", "lib.rs", "package.json", "vite.config.ts", "vite.config.js", "tsconfig.json",
]);

const STOP_WORDS = new Set([

  "the", "and", "for", "that", "this", "with", "from",

  "what", "why", "how", "does", "is", "are", "was",

  "not", "working", "work", "please", "can", "you",

  "my", "project", "file", "code", "make", "it",
]);

const EXTENSION_WEIGHTS: Record<string, number> = {

  ".ts": 0.12,
  ".tsx": 0.14,
  ".js": 0.10,
  ".jsx": 0.12,
  ".css": 0.07,
  ".scss": 0.06,
  ".html": 0.06,
  ".json": 0.05,
  ".rs": 0.14,
  ".py": 0.12,
  ".go": 0.11,
  ".java": 0.10,
  ".kt": 0.10,
  ".cpp": 0.10,
  ".c": 0.10,
  ".h": 0.08,
  ".md": 0.03,
};

function tokenize(text: string): string[] {
  return Array.from(
    new Set(
      text
        .toLowerCase()
        .replace(/[^Q-z0-9._\./-]+/g, " ")
        .split(/\s+/)
        .filter(
          (token) =>
            token.length >= 2 && !STOP_WORDS.has(token)
         ),
    )
  );

}

function getExtension(path: string): string {
  const match = path.toLowerCase().match(/\.[a-z0-9]+$/);
  return match?.[1] ?? "";
}

export function rankRelevantProjectFiles(
  text: string,
  files: ProjectFileLike[],
  intent: IntentResult = detectProjectIntent(text),
): RelevantFile[] {
  const queryTokens = tokenize(text);

  return files
    .filter((file) => !file.is_directory)
    .map((file) => {
      const pathLower = file.path.toLowerCase();
      const nameLower = file.name.toLowerCase();
      const pathParts: string[] = pathLower.split(/[\/\\]+/).filter(Boolean);

      let score = 0;
      const reasons: string[] = [];

      const ignored =
        pathParts.some((part) => IGNORED_PATH_PARTS.has(part)) ||
        GENERATED_FILE_NAMES.has(nameLower);

      if (ignored) {
        return {
          file,
          score: 0,
          reasons: ["ignored/generated path"],
        };
      }

      if (CORE_FILE_NAMES.has(nameLower)) {
        score += 0.18;
        reasons.push("core project file");
      }

      const exact = queryTokens.some(
        (token) =>
          nameLower === token ||
          nameLower.replace(/\.[^.]+$/, "") === token ||
          pathLower.includes(token),
      );

      if (exact) {
        score += 0.55;
        reasons.push("query/path match");
      }

      const matches = queryTokens.filter(
        (token) =>
          nameLower.includes(token) ||
          pathLower.includes(token),
      );

      if (matches.length > 0) {
        score += Math.min(0.25, matches.length * 0.08);
        reasons.push(
          `keyword match: ${matches.slice(0, 3).join(", ")}`,
        );
      }

      const extension = getExtension(pathLower);
      score += EXTENSION_WEIGHTS[extension] ?? 0.02;

      if (
        /(^|[\/])(src|app|components|services|lib|backend|frontend)([\/]|$)/i.test(
          pathLower,
        )
      ) {
        score += 0.06;
        reasons.push("source directory");
      }

      if (
        intent.intent === "debug" &&
        [".ts", ".tsx", ".js", ".jsx", ".rs", ".py", ".go", ".java", ".kt"].includes(
          extension,
        )
      ) {
        score += 0.04;
        reasons.push("debuggable source");
      }

      if (
        intent.intent === "architecture" &&
        (nameLower.includes("config") ||
          nameLower.includes("main") ||
          nameLower.includes("lib") ||
          nameLower.includes("app") ||
          nameLower.includes("service"))
      ) {
        score += 0.05;
        reasons.push("architecture-relevant");
      }

      return {
        file,
        score: Math.min(1, score),
        reasons,
      };
    })
    .filter((item) => item.score > 0)
    .sort((a, b) => b.score - a.score);
}

export function selectRelevantProjectFiles(
  text: string,
  files: ProjectFileLike[],
  limit = 6,
): RelevantFile[] {
  return rankRelevantProjectFiles(text, files)
    .filter(item => item.score >= 0.10)
    .slice(0, limit);
}

export function getIntentInstructions(
  intent: ProjectIntent,
): string {
  switch (intent) {
    case "debug":
      return `
The user appears to be debugging a problem.

Focus on:
- actual errors and failure points
- relevant source files
- imports and dependencies
- data flow
- likely root cause
- concrete fixes

Do not invent errors that are not present in the provided files.
`;

    case "edit":
      return `
The user wants to change the project.

Before suggesting changes:
- identify the relevant files
- understand the existing implementation
- preserve existing behavior unless the user asks otherwise
- mention exact file paths
- clearly distinguish existing code from proposed changes
`;

    case "explain":
      return `
The user wants an explanation.

Use the actual project files as the source of truth.
Explain the implementation using exact file paths and relevant functions/components.
Do not invent project structure.
`;

    case "find":
      return `
The user wants to locate something in the project.

Identify the most relevant files and paths.
Use only files actually provided by the project context.
`;

    case "architecture":
      return `
The user is asking about project architecture or structure.

Analyze:
- major directories
- important source files
- frontend/backend boundaries
- data flow
- important services
- native/backend integration

Only describe architecture supported by the provided project files.
`;

    case "review":
      return `
The user wants a code or project review.

Separate:
1. Facts found in the actual files.
2. Problems or risks.
3. Suggested improvements.

Mention exact file paths for findings.
`;

    case "run":
      return `
The user is asking about running, building, testing, or compiling the project.

Use actual project files and configuration when available.
Do not invent commands or configuration.
`;

    default:
      return `
Answer using the provided project context when relevant.
Prefer facts from the actual project files over assumptions.
`;
  }
}
