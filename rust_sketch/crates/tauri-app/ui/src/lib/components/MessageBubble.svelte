<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { Message } from '../types';
  import ToolResultCard from './ToolResultCard.svelte';

  interface Props {
    message: Message;
    isPendingTool?: boolean;
  }

  let { message, isPendingTool = false }: Props = $props();
  let openingNote = $state<string | null>(null);

  async function handleOpenNote(noteId: string) {
    openingNote = noteId;
    try {
      await invoke('open_note', { noteId });
    } catch (e) {
      console.error('Failed to open note:', e);
    } finally {
      openingNote = null;
    }
  }

  let hasNotes = $derived(message.notes && message.notes.length > 0);
</script>

{#if message.role === 'tool' && message.toolResult}
  <div class="message tool">
    <ToolResultCard toolResult={message.toolResult} isPending={isPendingTool} />
  </div>
{:else}
  <div class="message {message.role}">
    <div class="message-content">{message.content}</div>

    {#if message.role === 'assistant' && hasNotes}
      <div class="notes-bar">
        <span class="notes-label">Open in Notes:</span>
        {#each message.notes as note (note.id)}
          <button
            class="open-note-btn"
            onclick={() => handleOpenNote(note.id)}
            disabled={openingNote === note.id}
            title="Open '{note.title}' in Notes.app"
          >
            {#if openingNote === note.id}
              <span class="spinner">⏳</span>
            {:else}
              <span class="icon">📝</span>
              <span class="title">{note.title.length > 25 ? note.title.substring(0, 25) + '...' : note.title}</span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .message {
    margin-bottom: 1rem;
    max-width: 85%;
  }

  .message.user {
    margin-left: auto;
  }

  .message.assistant {
    margin-right: auto;
  }

  .message.tool {
    margin-right: auto;
    max-width: 90%;
  }

  .message-content {
    padding: 0.75rem 1rem;
    border-radius: 1rem;
    white-space: pre-wrap;
    word-wrap: break-word;
    line-height: 1.4;
  }

  .message.user .message-content {
    background: #007aff;
    color: white;
    border-bottom-right-radius: 0.25rem;
  }

  .message.assistant .message-content {
    background: #e9e9eb;
    color: #1c1c1e;
    border-bottom-left-radius: 0.25rem;
  }

  /* Notes bar below assistant message */
  .notes-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.5rem;
    padding: 0.5rem 0.75rem;
    background: #f0f4f8;
    border-radius: 0.75rem;
    border: 1px solid #d8e1eb;
  }

  .notes-label {
    font-size: 0.75rem;
    color: #5c6f82;
    font-weight: 500;
  }

  .open-note-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.35rem 0.6rem;
    background: #007aff;
    color: white;
    border: none;
    border-radius: 0.5rem;
    font-size: 0.75rem;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.15s, transform 0.1s;
  }

  .open-note-btn:hover:not(:disabled) {
    background: #0056b3;
    transform: translateY(-1px);
  }

  .open-note-btn:active:not(:disabled) {
    transform: translateY(0);
  }

  .open-note-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }

  .open-note-btn .icon {
    font-size: 0.9rem;
  }

  .open-note-btn .title {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .spinner {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  /* Dark mode */
  @media (prefers-color-scheme: dark) {
    .message.assistant .message-content {
      background: #3a3a3c;
      color: #f6f6f6;
    }

    .notes-bar {
      background: #2a3540;
      border-color: #3d4d5c;
    }

    .notes-label {
      color: #8899a8;
    }
  }
</style>
