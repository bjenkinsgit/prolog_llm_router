// Types for the chat application

export type MessageRole = 'user' | 'assistant' | 'tool';

export interface ToolResult {
  tool: string;
  success: boolean;
  output: string;
  args?: Record<string, unknown>;
}

// Note info for quick open actions
export interface NoteInfo {
  id: string;
  title: string;
  folder?: string;
}

export interface Message {
  id: string;
  role: MessageRole;
  content: string;
  timestamp: Date;
  toolResult?: ToolResult;
  // Notes attached to assistant messages for quick open
  notes?: NoteInfo[];
}

// Tool types with parameter schemas
export interface ToolParameter {
  name: string;
  description: string;
  required: boolean;
  param_type: string;
}

export interface ToolInfo {
  name: string;
  description: string;
  parameters: ToolParameter[];
}

// Agent event types (matching Rust AgentEventPayload)
export type AgentEvent =
  | { type: 'turn_started'; turn: number; max_turns: number }
  | { type: 'tool_calling'; tool: string; args: Record<string, unknown> }
  | { type: 'tool_result'; tool: string; success: boolean; output: string }
  | { type: 'final_answer'; answer: string }
  | { type: 'error'; message: string };

// Conversation types for history
export interface ConversationSummary {
  id: string;
  title: string;
  last_message: string | null;
  updated_at: string;
  message_count: number;
}

export interface StoredToolResult {
  tool: string;
  success: boolean;
  output: string;
  args?: Record<string, unknown>;
}

export interface StoredMessage {
  role: string;
  content: string;
  tool_result?: StoredToolResult;
  timestamp: string;
}

export interface StoredConversation {
  title: string;
  messages: StoredMessage[];
  created_at: string;
  updated_at: string;
}

export interface SendMessageResult {
  conversation_id: string;
  answer: string;
}

export interface ConversationState {
  messages: Message[];
  isLoading: boolean;
  currentTurn: number;
  maxTurns: number;
  pendingTool: string | null;
  conversationId: string | null;
}
