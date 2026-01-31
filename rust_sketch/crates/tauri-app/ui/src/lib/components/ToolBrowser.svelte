<script lang="ts">
  import type { ToolInfo } from '../types';

  interface Props {
    tools: ToolInfo[];
  }

  let { tools }: Props = $props();
  let expandedTool = $state<string | null>(null);

  function toggleTool(name: string) {
    expandedTool = expandedTool === name ? null : name;
  }
</script>

<div class="tool-browser">
  <h3>Tools ({tools.length})</h3>

  {#if tools.length === 0}
    <p class="empty">No tools loaded</p>
  {:else}
    <ul class="tool-list">
      {#each tools as tool (tool.name)}
        <li class="tool-item">
          <button
            class="tool-header"
            class:expanded={expandedTool === tool.name}
            onclick={() => toggleTool(tool.name)}
          >
            <span class="tool-name">{tool.name}</span>
            <span class="expand-icon">{expandedTool === tool.name ? '▼' : '▶'}</span>
          </button>

          {#if expandedTool === tool.name}
            <div class="tool-details">
              <p class="tool-description">{tool.description}</p>

              {#if tool.parameters && tool.parameters.length > 0}
                <div class="params-section">
                  <h4>Parameters</h4>
                  <ul class="param-list">
                    {#each tool.parameters as param}
                      <li class="param-item">
                        <span class="param-name">
                          {param.name}
                          {#if param.required}
                            <span class="required">*</span>
                          {/if}
                        </span>
                        <span class="param-type">{param.param_type}</span>
                        {#if param.description}
                          <span class="param-desc">{param.description}</span>
                        {/if}
                      </li>
                    {/each}
                  </ul>
                </div>
              {:else}
                <p class="no-params">No parameters</p>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .tool-browser h3 {
    margin: 0 0 0.5rem 0;
    font-size: 0.75rem;
    font-weight: 600;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .empty {
    color: #999;
    font-size: 0.8rem;
    font-style: italic;
    margin: 0;
  }

  .tool-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }

  .tool-item {
    border-radius: 0.375rem;
    overflow: hidden;
  }

  .tool-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 0.375rem 0.5rem;
    background: none;
    border: none;
    cursor: pointer;
    font-family: inherit;
    text-align: left;
  }

  .tool-header:hover {
    background: rgba(0, 0, 0, 0.05);
  }

  .tool-header.expanded {
    background: rgba(0, 122, 255, 0.1);
  }

  .tool-name {
    font-size: 0.8rem;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    color: #333;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .expand-icon {
    font-size: 0.625rem;
    color: #999;
    flex-shrink: 0;
  }

  .tool-details {
    padding: 0.5rem;
    background: rgba(0, 0, 0, 0.02);
    border-top: 1px solid rgba(0, 0, 0, 0.05);
  }

  .tool-description {
    font-size: 0.75rem;
    color: #666;
    margin: 0 0 0.5rem 0;
    line-height: 1.4;
  }

  .params-section h4 {
    font-size: 0.7rem;
    font-weight: 600;
    color: #888;
    margin: 0 0 0.25rem 0;
    text-transform: uppercase;
  }

  .param-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .param-item {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.5rem;
    font-size: 0.7rem;
    padding: 0.25rem;
    background: rgba(0, 0, 0, 0.03);
    border-radius: 0.25rem;
  }

  .param-name {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    color: #333;
    font-weight: 500;
  }

  .required {
    color: #ff3b30;
  }

  .param-type {
    color: #007aff;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 0.65rem;
    padding: 0.1rem 0.25rem;
    background: rgba(0, 122, 255, 0.1);
    border-radius: 0.25rem;
  }

  .param-desc {
    width: 100%;
    color: #888;
    font-size: 0.65rem;
  }

  .no-params {
    font-size: 0.7rem;
    color: #999;
    font-style: italic;
    margin: 0;
  }

  /* Dark mode */
  @media (prefers-color-scheme: dark) {
    .tool-browser h3 {
      color: #999;
    }

    .tool-header:hover {
      background: rgba(255, 255, 255, 0.05);
    }

    .tool-header.expanded {
      background: rgba(0, 122, 255, 0.2);
    }

    .tool-name {
      color: #eee;
    }

    .tool-details {
      background: rgba(255, 255, 255, 0.02);
      border-top-color: rgba(255, 255, 255, 0.05);
    }

    .tool-description {
      color: #aaa;
    }

    .param-item {
      background: rgba(255, 255, 255, 0.03);
    }

    .param-name {
      color: #eee;
    }
  }
</style>
