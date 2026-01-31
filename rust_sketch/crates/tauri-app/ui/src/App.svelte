<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import MessageBubble from './lib/components/MessageBubble.svelte';
  import ConversationHistory from './lib/components/ConversationHistory.svelte';
  import ToolBrowser from './lib/components/ToolBrowser.svelte';
  import type { AgentEvent, ToolInfo, SendMessageResult } from './lib/types';
  import {
    chatState,
    addUserMessage,
    setLoading,
    setConversationId,
    handleAgentEvent,
    clearMessages,
  } from './lib/stores/chat.svelte';

  let inputText = $state('');
  let tools = $state<ToolInfo[]>([]);
  let messagesContainer: HTMLDivElement;
  let historyComponent: ConversationHistory;

  // Scroll to bottom when messages change
  $effect(() => {
    if (chatState.messages.length && messagesContainer) {
      messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }
  });

  onMount(async () => {
    // Load available tools
    try {
      tools = await invoke<ToolInfo[]>('get_tools');
    } catch (e) {
      console.error('Failed to load tools:', e);
    }

    // Listen for agent events
    const unlistenTurnStarted = await listen<AgentEvent>('agent:turn_started', (event) => {
      handleAgentEvent(event.payload);
    });

    const unlistenToolCalling = await listen<AgentEvent>('agent:tool_calling', (event) => {
      handleAgentEvent(event.payload);
    });

    const unlistenToolResult = await listen<AgentEvent>('agent:tool_result', (event) => {
      handleAgentEvent(event.payload);
    });

    const unlistenFinalAnswer = await listen<AgentEvent>('agent:final_answer', (event) => {
      handleAgentEvent(event.payload);
      // Refresh conversation history after getting an answer
      historyComponent?.refresh();
    });

    const unlistenError = await listen<AgentEvent>('agent:error', (event) => {
      handleAgentEvent(event.payload);
    });

    return () => {
      unlistenTurnStarted();
      unlistenToolCalling();
      unlistenToolResult();
      unlistenFinalAnswer();
      unlistenError();
    };
  });

  async function sendMessage() {
    if (!inputText.trim() || chatState.isLoading) return;

    const userMessage = inputText.trim();
    inputText = '';

    addUserMessage(userMessage);
    setLoading(true);

    try {
      // Pass conversation_id if we're continuing a conversation
      const result = await invoke<SendMessageResult>('send_message', {
        conversationId: chatState.conversationId,
        message: userMessage,
      });
      // Update the conversation ID (may be new or existing)
      setConversationId(result.conversation_id);
    } catch (error) {
      handleAgentEvent({ type: 'error', message: String(error) });
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      sendMessage();
    }
  }

  function handleNewChat() {
    clearMessages();
  }
</script>

<div class="app">
  <header class="header">
    <h1>Prolog Router</h1>
    <div class="header-actions">
      <button class="new-chat-btn" onclick={handleNewChat}>New Chat</button>
    </div>
  </header>

  <div class="main-content">
    <aside class="sidebar">
      <div class="sidebar-section">
        <div class="status-section">
          <h3>Status</h3>
          {#if chatState.isLoading}
            <p class="status">
              Turn {chatState.currentTurn}/{chatState.maxTurns}
              {#if chatState.pendingTool}
                <br />Running: {chatState.pendingTool}
              {/if}
            </p>
          {:else}
            <p class="status idle">Ready</p>
          {/if}
        </div>
      </div>

      <div class="sidebar-section history-section">
        <ConversationHistory bind:this={historyComponent} />
      </div>

      <div class="sidebar-section tools-section">
        <ToolBrowser {tools} />
      </div>
    </aside>

    <main class="chat-panel">
      <div class="messages" bind:this={messagesContainer}>
        {#if chatState.messages.length === 0}
          <div class="empty-state">
            <p>Send a message to start a conversation</p>
            <p class="hint">Try: "What's the weather in Seattle?"</p>
          </div>
        {:else}
          {#each chatState.messages as message (message.id)}
            <MessageBubble
              {message}
              isPendingTool={chatState.pendingTool === message.toolResult?.tool}
            />
          {/each}
        {/if}
        {#if chatState.isLoading && !chatState.pendingTool && chatState.messages.at(-1)?.role !== 'tool'}
          <div class="message assistant loading">
            <div class="message-content">Thinking...</div>
          </div>
        {/if}
      </div>

      <div class="input-bar">
        <textarea
          bind:value={inputText}
          onkeydown={handleKeydown}
          placeholder="Type your message..."
          rows="1"
          disabled={chatState.isLoading}
        ></textarea>
        <button
          onclick={sendMessage}
          disabled={chatState.isLoading || !inputText.trim()}
        >
          {chatState.isLoading ? '...' : 'Send'}
        </button>
      </div>
    </main>
  </div>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid #e0e0e0;
    background: #fafafa;
  }

  .header h1 {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 600;
  }

  .header-actions {
    display: flex;
    gap: 0.5rem;
  }

  .new-chat-btn {
    padding: 0.5rem 1rem;
    background: #e0e0e0;
    border: none;
    border-radius: 0.5rem;
    font-family: inherit;
    font-size: 0.875rem;
    cursor: pointer;
  }

  .new-chat-btn:hover {
    background: #d0d0d0;
  }

  .main-content {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  .sidebar {
    width: 220px;
    border-right: 1px solid #e0e0e0;
    background: #fafafa;
    overflow-y: auto;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .sidebar-section {
    display: flex;
    flex-direction: column;
  }

  .status-section h3 {
    margin: 0 0 0.5rem 0;
    font-size: 0.75rem;
    font-weight: 600;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .history-section {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .tools-section {
    border-top: 1px solid #e0e0e0;
    padding-top: 1rem;
    max-height: 40%;
    overflow-y: auto;
  }

  .status {
    font-size: 0.875rem;
    color: #007aff;
    margin: 0;
  }

  .status.idle {
    color: #34c759;
  }

  .chat-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #999;
    text-align: center;
  }

  .empty-state .hint {
    font-size: 0.875rem;
    margin-top: 0.5rem;
    color: #bbb;
  }

  .message.loading .message-content {
    background: #e9e9eb;
    color: #1c1c1e;
    border-bottom-left-radius: 0.25rem;
    padding: 0.75rem 1rem;
    border-radius: 1rem;
    opacity: 0.6;
  }

  .input-bar {
    display: flex;
    gap: 0.5rem;
    padding: 1rem;
    border-top: 1px solid #e0e0e0;
    background: #fff;
  }

  .input-bar textarea {
    flex: 1;
    padding: 0.75rem 1rem;
    border: 1px solid #e0e0e0;
    border-radius: 1.5rem;
    font-family: inherit;
    font-size: 1rem;
    resize: none;
    outline: none;
  }

  .input-bar textarea:focus {
    border-color: #007aff;
  }

  .input-bar textarea:disabled {
    background: #f5f5f5;
  }

  .input-bar button {
    padding: 0.75rem 1.5rem;
    background: #007aff;
    color: white;
    border: none;
    border-radius: 1.5rem;
    font-family: inherit;
    font-size: 1rem;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
  }

  .input-bar button:hover:not(:disabled) {
    background: #0056b3;
  }

  .input-bar button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Dark mode */
  @media (prefers-color-scheme: dark) {
    .header {
      background: #2c2c2e;
      border-bottom-color: #3a3a3c;
    }

    .new-chat-btn {
      background: #3a3a3c;
      color: #f6f6f6;
    }

    .new-chat-btn:hover {
      background: #4a4a4c;
    }

    .sidebar {
      background: #2c2c2e;
      border-right-color: #3a3a3c;
    }

    .status-section h3 {
      color: #999;
    }

    .tools-section {
      border-top-color: #3a3a3c;
    }

    .message.loading .message-content {
      background: #3a3a3c;
      color: #f6f6f6;
    }

    .input-bar {
      background: #1a1a1a;
      border-top-color: #3a3a3c;
    }

    .input-bar textarea {
      background: #2c2c2e;
      border-color: #3a3a3c;
      color: #f6f6f6;
    }

    .input-bar textarea:focus {
      border-color: #007aff;
    }

    .input-bar textarea:disabled {
      background: #1a1a1a;
    }
  }
</style>
