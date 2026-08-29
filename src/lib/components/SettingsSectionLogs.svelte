<script lang="ts">
  /**
   * SettingsSectionLogs.svelte — 0.4.7
   *
   * Surfaces the per-app `logs/app.YYYY-MM-DD.json` files written by
   * the Rust `tracing-appender` rolling file layer. The user can:
   *
   * 1. See a list of available log files (newest first, with size
   *    and modification time).
   * 2. Click one to read its tail (up to 256 KB; the rest is
   *    available on disk for hand inspection).
   * 3. Clear all log files (useful before reproducing a bug).
   * 4. Reveal the `logs/` directory in Finder/Explorer.
   *
   * The viewer shows the raw JSON lines as-is — they're already
   * structured, so a separate parser isn't needed. Power users
   * pipe the file into `jq`; the UI is the convenience layer.
   */

  import { onMount } from "svelte";
  import FileText from "@lucide/svelte/icons/file-text";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import X from "@lucide/svelte/icons/x";
  import Inbox from "@lucide/svelte/icons/inbox";

  import { logs } from "$lib/stores/logs.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { invoke } from "@tauri-apps/api/core";

  onMount(() => {
    void logs.refresh();
  });

  async function handleClear() {
    if (!confirm(i18n.t("logs.confirmClear"))) return;
    await logs.clear();
  }

  async function handleOpenFolder() {
    try {
      const path = await invoke<string>("logs_folder_path");
      await invoke("reveal_path", { path });
      toast.success(i18n.t("logs.openedFolderOk"));
    } catch (e) {
      const error = String(e);
      toast.error(i18n.t("logs.openFolderFailed", { error }));
    }
  }
</script>

<div class="section">
  <div class="section-head">
    <div class="section-title">
      <FileText size={18} aria-hidden="true" />
      <h2>{i18n.t("logs.title")}</h2>
    </div>
    <div class="section-actions">
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("logs.openingFolder")}
        onclick={handleOpenFolder}
      >
        <FolderOpen size={14} />
      </button>
      <button
        type="button"
        class="iconbtn danger"
        title={i18n.t("logs.clearAll")}
        disabled={logs.clearing || logs.files.length === 0}
        onclick={handleClear}
      >
        <Trash2 size={14} />
      </button>
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("logs.refresh")}
        disabled={logs.loading}
        onclick={() => logs.refresh()}
      >
        <RefreshCw size={14} class={logs.loading ? "spin" : ""} />
      </button>
    </div>
  </div>

  <p class="lead">{i18n.t("logs.description")}</p>

  <div class="split">
    <!-- File list -->
    <div class="pane files-pane">
      {#if logs.files.length === 0}
        <div class="empty">
          <Inbox size={28} aria-hidden="true" />
          <p>{i18n.t("logs.empty")}</p>
        </div>
      {:else}
        <ul class="file-list" role="list">
          {#each logs.files as f (f.name)}
            <li>
              <button
                type="button"
                class="file-item"
                class:selected={logs.openFile === f.name}
                onclick={() => logs.open(f.name)}
                aria-current={logs.openFile === f.name ? "true" : undefined}
              >
                <span class="mono name" title={f.name}>{f.name}</span>
                <span class="meta">
                  <span class="when">{logs.formatWhen(f)}</span>
                  <span class="size">{logs.formatSize(f)}</span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <!-- Tail viewer -->
    <div class="pane viewer-pane">
      {#if !logs.openFile}
        <div class="placeholder">
          <FileText size={32} aria-hidden="true" />
          <p>{i18n.t("logs.pickFile")}</p>
        </div>
      {:else}
        <div class="viewer-head">
          <span class="mono name">{logs.openFile}</span>
          <button
            type="button"
            class="iconbtn"
            aria-label={i18n.t("common.close")}
            onclick={() => logs.close()}
          >
            <X size={14} />
          </button>
        </div>
        {#if logs.reading}
          <p class="hint">{i18n.t("logs.reading")}</p>
        {:else}
          <pre class="tail" aria-label={i18n.t("logs.tailLabel")}>{logs.current || ""}</pre>
        {/if}
      {/if}
    </div>
  </div>
</div>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }
  .section-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .section-title h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }
  .section-actions {
    display: flex;
    gap: 0.25rem;
  }
  .iconbtn {
    background: transparent;
    border: 1px solid var(--border, #2a2a2a);
    border-radius: 4px;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: inherit;
  }
  .iconbtn:hover:not(:disabled) {
    background: var(--hover, rgba(255, 255, 255, 0.05));
  }
  .iconbtn:disabled {
    cursor: default;
    opacity: 0.5;
  }
  .iconbtn.danger:hover:not(:disabled) {
    color: var(--color-danger, #e35b5b);
    border-color: var(--color-danger, #e35b5b);
  }
  .spin {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
  .lead {
    margin: 0;
    color: var(--muted, #888);
    font-size: 0.875rem;
  }
  .split {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
    gap: 0.75rem;
    border: 1px solid var(--border, #2a2a2a);
    border-radius: 6px;
    overflow: hidden;
    min-height: 360px;
  }
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .files-pane {
    border-right: 1px solid var(--border, #2a2a2a);
    overflow-y: auto;
  }
  .file-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .file-item {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    width: 100%;
    padding: 0.5rem 0.75rem;
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border, #2a2a2a);
    text-align: left;
    cursor: pointer;
    color: inherit;
  }
  .file-item:hover {
    background: var(--hover, rgba(255, 255, 255, 0.03));
  }
  .file-item.selected {
    background: var(--selected, rgba(91, 140, 255, 0.1));
    color: var(--color-text-primary, #f5f5f5);
  }
  .file-item .name {
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file-item .meta {
    display: flex;
    justify-content: space-between;
    font-size: 0.6875rem;
    color: var(--muted, #888);
  }
  .empty,
  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 2rem 1rem;
    color: var(--muted, #888);
    text-align: center;
    height: 100%;
  }
  .empty p,
  .placeholder p {
    margin: 0;
    max-width: 16rem;
  }
  .viewer-pane {
    overflow: hidden;
  }
  .viewer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border, #2a2a2a);
    background: var(--header, rgba(255, 255, 255, 0.02));
  }
  .viewer-head .name {
    font-size: 0.75rem;
  }
  .tail {
    margin: 0;
    padding: 0.75rem;
    font-family: var(--mono, ui-monospace, "SF Mono", Menlo, monospace);
    font-size: 0.6875rem;
    line-height: 1.4;
    overflow: auto;
    white-space: pre;
    flex: 1;
    min-height: 0;
  }
  .mono {
    font-family: var(--mono, ui-monospace, "SF Mono", Menlo, monospace);
  }
  .hint {
    margin: 0;
    padding: 0.75rem;
    color: var(--muted, #888);
    font-size: 0.875rem;
  }
</style>
