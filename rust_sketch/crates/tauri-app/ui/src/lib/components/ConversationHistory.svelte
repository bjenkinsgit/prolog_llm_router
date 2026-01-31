<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import type { ConversationSummary, StoredConversation, Message } from '../types';
  import { loadConversation, chatState } from '../stores/chat.svelte';

  let conversations = $state<ConversationSummary[]>([]);
  let loading = $state(false);

  onMount(() => {
    loadConversations();
  });

  async function loadConversations() {
    loading = true;
    try {
      conversations = await invoke<ConversationSummary[]>('list_conversations');
    } catch (e) {
      console.error('Failed to load conversations:', e);
    } finally {
      loading = false;
    }
  }

  async function selectConversation(id: string) {
    try {
      const conv = await invoke<StoredConversation | null>('get_conversation', { id });
      if (conv) {
        // Convert stored messages to UI messages
        const messages: Message[] = conv.messages.map((m, i) => ({
          id: `stored-${id}-${i}`,
          role: m.role as 'user' | 'assistant' | 'tool',
          content: m.content,
          timestamp: new Date(m.timestamp),
          toolResult: m.tool_result ? {
            tool: m.tool_result.tool,
            success: m.tool_result.success,
            output: m.tool_result.output,
            args: m.tool_result.args,
          } : undefined,
        }));
        loadConversation(id, messages);
      }
    } catch (e) {
      console.error('Failed to load conversation:', e);
    }
  }

  async function deleteConversation(id: string, event: MouseEvent) {
    event.stopPropagation();
    if (!confirm('Delete this conversation?')) return;

    try {
      await invoke('delete_conversation', { id });
      conversations = conversations.filter(c => c.id !== id);
      // If we deleted the current conversation, clear the chat
      if (chatState.conversationId === id) {
        loadConversation('', []);
      }
    } catch (e) {
      console.error('Failed to delete conversation:', e);
    }
  }

  function formatDate(dateStr: string): string {
    const date = new Date(dateStr);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

    if (diffDays === 0) {
      return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    } else if (diffDays === 1) {
      return 'Yesterday';
    } else if (diffDays < 7) {
      return date.toLocaleDateString([], { weekday: 'short' });
    } else {
      return date.toLocaleDateString([], { month: 'short', day: 'numeric' });
    }
  }

  // Expose refresh function
  export function refresh() {
    loadConversations();
  }
</script>

<div class="conversation-history">
  <div class="section-header">
    <h3>History</h3>
    <button class="refresh-btn" onclick={() => loadConversations()} title="Refresh">
      {loading ? '...' : '↻'}
    </button>
  </div>

  {#if conversations.length === 0}
    <p class="empty">No conversations yet</p>
  {:else}
    <ul class="conversation-list">
      {#each conversations as conv (conv.id)}
        <li
          class="conversation-item"
          class:active={chatState.conversationId === conv.id}
          onclick={() => selectConversation(conv.id)}
        >
          <div class="conv-title">{conv.title}</div>
          <div class="conv-meta">
            <span class="conv-date">{formatDate(conv.updated_at)}</span>
            <button
              class="delete-btn"
              onclick={(e) => deleteConversation(conv.id, e)}
              title="Delete"
            >×</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .conversation-history {
    display: flex;
    flex-direction: column;
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
  }

  .section-header h3 {
    margin: 0;
    font-size: 0.75rem;
    font-weight: 600;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .refresh-btn {
    background: none;
    border: none;
    font-size: 0.875rem;
    cursor: pointer;
    padding: 0.25rem;
    color: #666;
  }

  .refresh-btn:hover {
    color: #007aff;
  }

  .empty {
    color: #999;
    font-size: 0.8rem;
    font-style: italic;
    margin: 0;
  }

  .conversation-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .conversation-item {
    padding: 0.5rem;
    border-radius: 0.375rem;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }

  .conversation-item:hover {
    background: rgba(0, 0, 0, 0.05);
  }

  .conversation-item.active {
    background: rgba(0, 122, 255, 0.1);
  }

  .conv-title {
    font-size: 0.8rem;
    color: #333;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .conv-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .conv-date {
    font-size: 0.7rem;
    color: #999;
  }

  .delete-btn {
    background: none;
    border: none;
    font-size: 0.875rem;
    cursor: pointer;
    padding: 0 0.25rem;
    color: #999;
    opacity: 0;
    transition: opacity 0.2s;
  }

  .conversation-item:hover .delete-btn {
    opacity: 1;
  }

  .delete-btn:hover {
    color: #ff3b30;
  }

  /* Dark mode */
  @media (prefers-color-scheme: dark) {
    .section-header h3 {
      color: #999;
    }

    .conv-title {
      color: #eee;
    }

    .conversation-item:hover {
      background: rgba(255, 255, 255, 0.05);
    }

    .conversation-item.active {
      background: rgba(0, 122, 255, 0.2);
    }
  }
</style>
