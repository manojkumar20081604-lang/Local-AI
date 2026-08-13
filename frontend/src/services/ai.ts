export interface AIModel {
  id: string;
  object: string;
  owned_by: string;
}

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

interface ModelsResponse {
  data: AIModel[];
}

interface ChatResponse {
  choices: Array<{
    message: {
      role: string;
      content: string;
    };
  }>;
}

interface StreamChunk {
  choices?: Array<{
    delta?: {
      content?: string;
    };
  }>;
}

const BASE_URL = "http://localhost:1234/v1";

export async function getModels(): Promise<AIModel[]> {
  try {
    const response = await fetch(`${BASE_URL}/models`);

    if (!response.ok) {
      throw new Error(`HTTP error! Status: ${response.status}`);
    }

    const json = (await response.json()) as ModelsResponse;

    return json.data;
  } catch (error) {
    const errorMessage =
      error instanceof Error ? error.message : String(error);

    console.error("getModels error:", errorMessage);

    throw new Error(`Failed to fetch models: ${errorMessage}`);
  }
}

export async function chat(
  messages: ChatMessage[],
  model: string,
  signal?: AbortSignal,
): Promise<string> {
  try {
    const response = await fetch(`${BASE_URL}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      signal,
      body: JSON.stringify({
  model,
  messages,
  temperature: 0.7,
  max_tokens: 8192,
  stream: true,
}),
    });

    if (!response.ok) {
      let message = `HTTP error! Status: ${response.status}`;

      try {
        const errorJson = await response.json();

        if (errorJson?.error?.message) {
          message += ` - ${errorJson.error.message}`;
        }
      } catch {
        // Ignore invalid error response.
      }

      throw new Error(message);
    }

    const json = (await response.json()) as ChatResponse;

    const content = json.choices?.[0]?.message?.content;

    if (!content) {
      throw new Error("No chat completion result found in response.");
    }

    return content;
  } catch (error) {
    if (
      error instanceof DOMException &&
      error.name === "AbortError"
    ) {
      throw error;
    }

    if (
      error instanceof Error &&
      error.name === "AbortError"
    ) {
      throw error;
    }

    const errorMessage =
      error instanceof Error ? error.message : String(error);

    console.error("chat error:", errorMessage);

    throw new Error(`Failed to chat: ${errorMessage}`);
  }
}

export async function chatWithModel(
  model: string,
  messages: ChatMessage[],
  signal?: AbortSignal,
): Promise<string> {
  return chat(messages, model, signal);
}

export async function streamChatWithModel(
  model: string,
  messages: ChatMessage[],
  onChunk: (chunk: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  try {
    const response = await fetch(`${BASE_URL}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      signal,
      body: JSON.stringify({
        model,
        messages,
        temperature: 0.7,
        stream: true,
      }),
    });

    if (!response.ok) {
      let message = `HTTP error! Status: ${response.status}`;

      try {
        const errorJson = await response.json();

        if (errorJson?.error?.message) {
          message += ` - ${errorJson.error.message}`;
        }
      } catch {
        // Ignore invalid error response.
      }

      throw new Error(message);
    }

    if (!response.body) {
      throw new Error("Streaming response body is unavailable.");
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();

    let buffer = "";

    while (true) {
      const { value, done } = await reader.read();

      if (done) {
        break;
      }

      buffer += decoder.decode(value, { stream: true });

      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();

        if (!trimmed || !trimmed.startsWith("data:")) {
          continue;
        }

        const data = trimmed.slice(5).trim();

        if (data === "[DONE]") {
          return;
        }

        try {
          const json = JSON.parse(data) as StreamChunk;

          const content =
            json.choices?.[0]?.delta?.content;

          if (content) {
            onChunk(content);
          }
        } catch {
          // Ignore malformed SSE chunks.
        }
      }
    }

    const trimmed = buffer.trim();

    if (trimmed.startsWith("data:")) {
      const data = trimmed.slice(5).trim();

      if (data !== "[DONE]") {
        try {
          const json = JSON.parse(data) as StreamChunk;

          const content =
            json.choices?.[0]?.delta?.content;

          if (content) {
            onChunk(content);
          }
        } catch {
          // Ignore malformed final chunk.
        }
      }
    }
  } catch (error) {
    if (
      error instanceof DOMException &&
      error.name === "AbortError"
    ) {
      throw error;
    }

    if (
      error instanceof Error &&
      error.name === "AbortError"
    ) {
      throw error;
    }

    const errorMessage =
      error instanceof Error ? error.message : String(error);

    console.error(
      "streamChatWithModel error:",
      errorMessage,
    );

    throw new Error(
      `Failed to stream chat: ${errorMessage}`,
    );
  }
}
