<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { ToolResult } from '../types';

  interface Props {
    toolResult: ToolResult;
    isPending?: boolean;
  }

  interface NoteInfo {
    id: string;
    title: string;
    folder?: string;
  }

  let { toolResult, isPending = false }: Props = $props();

  let isExpanded = $state(false);
  let openingNote = $state<string | null>(null);

  function toggleExpanded() {
    isExpanded = !isExpanded;
  }

  // Truncate long output for display
  function truncateOutput(output: string, maxLength: number = 200): string {
    if (output.length <= maxLength) return output;
    return output.substring(0, maxLength) + '...';
  }

  // Extract notes with x-coredata IDs from the output
  function extractNotes(output: string): NoteInfo[] {
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
      // Not JSON or parse error - check for x-coredata patterns
      const matches = output.match(/x-coredata:\/\/[^\s"'\]},]+/g);
      if (matches) {
        return matches.map(id => ({ id, title: 'Note' }));
      }
    }
    return [];
  }

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

  // Get extracted notes for display
  let notes = $derived(toolResult.success ? extractNotes(toolResult.output) : []);
  let hasNotes = $derived(notes.length > 0);
</script>

<div class="tool-card" class:pending={isPending} class:error={!toolResult.success}>
  <button class="tool-header" onclick={toggleExpanded}>
    <span class="tool-icon">{isPending ? '⏳' : toolResult.success ? '✓' : '✗'}</span>
    <span class="tool-name">{toolResult.tool}</span>
    <span class="expand-icon">{isExpanded ? '▼' : '▶'}</span>
  </button>

  {#if isExpanded}
    <div class="tool-details">
      {#if toolResult.args}
        <div class="tool-args">
          <strong>Arguments:</strong>
          <pre>{JSON.stringify(toolResult.args, null, 2)}</pre>
        </div>
      {/if}
      <div class="tool-output">
        <strong>Output:</strong>
        <pre>{toolResult.output}</pre>
      </div>

      {#if hasNotes}
        <div class="notes-actions">
          <strong>Quick Actions:</strong>
          <div class="note-buttons">
            {#each notes as note (note.id)}
              <button
                class="open-note-btn"
                onclick={() => handleOpenNote(note.id)}
                disabled={openingNote === note.id}
                title="Open in Notes.app"
              >
                {#if openingNote === note.id}
                  <span class="spinner">⏳</span>
                {:else}
                  <span class="icon">📝</span>
                {/if}
                <span class="note-title">{note.title}</span>
                {#if note.folder}
                  <span class="note-folder">({note.folder})</span>
                {/if}
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  {:else if !isPending}
    <div class="tool-preview">
      {truncateOutput(toolResult.output)}
    </div>

    {#if hasNotes}
      <div class="notes-preview">
        {#each notes.slice(0, 3) as note (note.id)}
          <button
            class="open-note-btn-small"
            onclick={() => handleOpenNote(note.id)}
            disabled={openingNote === note.id}
            title="Open '{note.title}' in Notes.app"
          >
            {#if openingNote === note.id}
              ⏳
            {:else}
              📝 {note.title.length > 20 ? note.title.substring(0, 20) + '...' : note.title}
            {/if}
          </button>
        {/each}
        {#if notes.length > 3}
          <span class="more-notes">+{notes.length - 3} more</span>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .tool-card {
    background: #f5f5f5;
    border-radius: 0.5rem;
    margin: 0.5rem 0;
    overflow: hidden;
    border-left: 3px solid #007aff;
  }

  .tool-card.pending {
    border-left-color: #ff9500;
    opacity: 0.8;
  }

  .tool-card.error {
    border-left-color: #ff3b30;
  }

  .tool-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.75rem;
    background: none;
    border: none;
    cursor: pointer;
    font-family: inherit;
    font-size: 0.875rem;
    text-align: left;
  }

  .tool-header:hover {
    background: rgba(0, 0, 0, 0.05);
  }

  .tool-icon {
    font-size: 1rem;
  }

  .tool-name {
    flex: 1;
    font-weight: 500;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  }

  .expand-icon {
    font-size: 0.75rem;
    color: #666;
  }

  .tool-preview {
    padding: 0 0.75rem 0.75rem;
    font-size: 0.8rem;
    color: #666;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .tool-details {
    padding: 0 0.75rem 0.75rem;
    font-size: 0.8rem;
  }

  .tool-args, .tool-output {
    margin-top: 0.5rem;
  }

  .tool-details pre {
    margin: 0.25rem 0 0 0;
    padding: 0.5rem;
    background: #e0e0e0;
    border-radius: 0.25rem;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 0.75rem;
  }

  /* Notes actions styling */
  .notes-actions {
    margin-top: 0.75rem;
    padding-top: 0.75rem;
    border-top: 1px solid #e0e0e0;
  }

  .note-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .open-note-btn {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.4rem 0.65rem;
    background: #007aff;
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-size: 0.75rem;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.15s;
  }

  .open-note-btn:hover:not(:disabled) {
    background: #0056b3;
  }

  .open-note-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }

  .open-note-btn .icon {
    font-size: 0.85rem;
  }

  .open-note-btn .note-title {
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .open-note-btn .note-folder {
    opacity: 0.75;
    font-size: 0.65rem;
  }

  .spinner {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  /* Collapsed note preview buttons */
  .notes-preview {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0 0.75rem 0.75rem;
  }

  .open-note-btn-small {
    padding: 0.25rem 0.5rem;
    background: #e8f0fe;
    color: #1a73e8;
    border: 1px solid #c2d7f7;
    border-radius: 1rem;
    font-size: 0.7rem;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s;
  }

  .open-note-btn-small:hover:not(:disabled) {
    background: #d2e3fc;
    border-color: #aecbfa;
  }

  .open-note-btn-small:disabled {
    opacity: 0.6;
    cursor: wait;
  }

  .more-notes {
    font-size: 0.7rem;
    color: #666;
    padding: 0.25rem 0.5rem;
  }

  /* Dark mode */
  @media (prefers-color-scheme: dark) {
    .tool-card {
      background: #2c2c2e;
    }

    .tool-header:hover {
      background: rgba(255, 255, 255, 0.05);
    }

    .tool-preview {
      color: #999;
    }

    .tool-details pre {
      background: #3a3a3c;
    }

    .notes-actions {
      border-top-color: #3a3a3c;
    }

    .open-note-btn-small {
      background: #1a3a5c;
      color: #8ab4f8;
      border-color: #2d4a6a;
    }

    .open-note-btn-small:hover:not(:disabled) {
      background: #254a7a;
      border-color: #3d5a8a;
    }

    .more-notes {
      color: #888;
    }
  }
</style>
