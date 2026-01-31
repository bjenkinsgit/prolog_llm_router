// Chat state store using Svelte 5 runes
import type { Message, ToolResult, AgentEvent } from '../types';

// Note info for quick actions
export interface NoteInfo {
  id: string;
  title: string;
  folder?: string;
}

// Generate unique IDs for messages
let messageIdCounter = 0;
function generateId(): string {
  return `msg-${Date.now()}-${++messageIdCounter}`;
}

// Extract notes from tool output
function extractNotesFromOutput(output: string): NoteInfo[] {
  try {
    const parsed = JSON.parse(output);

    // Helper to extract id from an item (handles both 'id' and 'note_id' keys)
    const getNoteId = (item: any): string | null => {
      const id = item.note_id || item.id;
      return id && id.startsWith('x-coredata://') ? id : null;
    };

    // Handle array of notes directly
    if (Array.isArray(parsed)) {
      return parsed
        .filter((item: any) => getNoteId(item))
        .map((item: any) => ({
          id: getNoteId(item)!,
          title: item.title || 'Untitled',
          folder: item.folder
        }));
    }

    // Handle {results: [...]} format (from search_notes)
    if (parsed.results && Array.isArray(parsed.results)) {
      return parsed.results
        .filter((item: any) => getNoteId(item))
        .map((item: any) => ({
          id: getNoteId(item)!,
          title: item.title || 'Untitled',
          folder: item.folder
        }));
    }

    // Handle {notes: [...]} format
    if (parsed.notes && Array.isArray(parsed.notes)) {
      return parsed.notes
        .filter((item: any) => getNoteId(item))
        .map((item: any) => ({
          id: getNoteId(item)!,
          title: item.title || 'Untitled',
          folder: item.folder
        }));
    }

    // Handle single note with id/note_id
    if (getNoteId(parsed)) {
      return [{
        id: getNoteId(parsed)!,
        title: parsed.title || 'Untitled',
        folder: parsed.folder
      }];
    }

    // Handle {note: {...}} format
    if (parsed.note && getNoteId(parsed.note)) {
      return [{
        id: getNoteId(parsed.note)!,
        title: parsed.note.title || 'Untitled',
        folder: parsed.note.folder
      }];
    }
  } catch {
    // Not JSON - ignore
  }
  return [];
}

// Chat state
export const chatState = $state({
  messages: [] as Message[],
  isLoading: false,
  currentTurn: 0,
  maxTurns: 10,
  pendingTool: null as string | null,
  conversationId: null as string | null,
  // Notes found in the current turn's tool results
  pendingNotes: [] as NoteInfo[],
});

// Actions
export function addUserMessage(content: string): Message {
  const message: Message = {
    id: generateId(),
    role: 'user',
    content,
    timestamp: new Date(),
  };
  chatState.messages = [...chatState.messages, message];
  // Clear pending notes when user sends a new message
  chatState.pendingNotes = [];
  return message;
}

export function addAssistantMessage(content: string, notes?: NoteInfo[]): Message {
  const message: Message = {
    id: generateId(),
    role: 'assistant',
    content,
    timestamp: new Date(),
    // Attach notes to the assistant message
    notes: notes || [],
  };
  chatState.messages = [...chatState.messages, message];
  return message;
}

export function addToolMessage(toolResult: ToolResult): Message {
  const message: Message = {
    id: generateId(),
    role: 'tool',
    content: `Tool: ${toolResult.tool}`,
    timestamp: new Date(),
    toolResult,
  };
  chatState.messages = [...chatState.messages, message];
  return message;
}

export function setLoading(loading: boolean) {
  chatState.isLoading = loading;
}

export function setPendingTool(tool: string | null) {
  chatState.pendingTool = tool;
}

export function setTurn(turn: number, maxTurns: number) {
  chatState.currentTurn = turn;
  chatState.maxTurns = maxTurns;
}

export function setConversationId(id: string | null) {
  chatState.conversationId = id;
}

export function clearMessages() {
  chatState.messages = [];
  chatState.currentTurn = 0;
  chatState.pendingTool = null;
  chatState.conversationId = null;
  chatState.pendingNotes = [];
}

// Load a stored conversation into current state
export function loadConversation(id: string, messages: Message[]) {
  chatState.conversationId = id;
  chatState.messages = messages;
  chatState.currentTurn = 0;
  chatState.pendingTool = null;
  chatState.isLoading = false;
  chatState.pendingNotes = [];
}

// Handle agent events
export function handleAgentEvent(event: AgentEvent) {
  switch (event.type) {
    case 'turn_started':
      setTurn(event.turn, event.max_turns);
      break;

    case 'tool_calling':
      setPendingTool(event.tool);
      // Add a placeholder for the tool call
      addToolMessage({
        tool: event.tool,
        success: true,
        output: 'Running...',
        args: event.args,
      });
      break;

    case 'tool_result':
      setPendingTool(null);
      // Update the last tool message with the result
      const messages = chatState.messages;
      const lastToolIndex = messages.findLastIndex(m => m.role === 'tool' && m.toolResult?.tool === event.tool);
      if (lastToolIndex !== -1) {
        const updated = [...messages];
        updated[lastToolIndex] = {
          ...updated[lastToolIndex],
          toolResult: {
            tool: event.tool,
            success: event.success,
            output: event.output,
            args: updated[lastToolIndex].toolResult?.args,
          },
        };
        chatState.messages = updated;
      }

      // Extract notes from tool result and accumulate them
      if (event.success) {
        const foundNotes = extractNotesFromOutput(event.output);
        if (foundNotes.length > 0) {
          // Deduplicate by ID
          const existingIds = new Set(chatState.pendingNotes.map(n => n.id));
          const newNotes = foundNotes.filter(n => !existingIds.has(n.id));
          chatState.pendingNotes = [...chatState.pendingNotes, ...newNotes];
        }
      }
      break;

    case 'final_answer':
      // Attach accumulated notes to the final answer
      addAssistantMessage(event.answer, chatState.pendingNotes);
      // Clear pending notes after attaching
      chatState.pendingNotes = [];
      setLoading(false);
      break;

    case 'error':
      addAssistantMessage(`Error: ${event.message}`);
      chatState.pendingNotes = [];
      setLoading(false);
      break;
  }
}
