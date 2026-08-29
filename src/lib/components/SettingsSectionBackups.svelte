<script lang="ts">
  /**
   * SettingsSectionBackups.svelte — 0.4.5
   *
   * Surfaces the per-app `backups/index.json` ledger and lets the user
   * roll any of the listed snapshots back to the original `dest`. Backups
   * are written automatically by `install::do_install` / `do_update`
   * (via `record_backup_entries`) before any destructive overwrite, so
   * this section is read-and-act: no settings, no toggles, just a list
   * and a Restore button per row.
   *
   * Mirrors the Rust API at `src-tauri/src/install/mod.rs`
   * (`backup_list` / `backup_restore`).
   */

  import { onMount } from "svelte";
  import History from "@lucide/svelte/icons/history";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import RotateCcw from "@lucide/svelte/icons/rotate-ccw";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import Inbox from "@lucide/svelte/icons/inbox";

  import { backup } from "$lib/stores/backup.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { invoke } from "@tauri-apps/api/core";

  onMount(() => {
    void backup.refresh();
  });

  async function handleRestore(filename: string) {
    if (!confirm(i18n.t("backups.confirmRestore"))) return;
    await backup.restore(filename);
  }

  /** Open the `backups/` directory in Finder/Explorer. We don't have a
      dedicated IPC for "open arbitrary path inside app data", so we go
      through `reveal_path` which already does exactly that. */
  async function handleOpenFolder() {
    try {
      // Ask Rust for the absolute backups dir, then reveal it.
      const path = await invoke<string>("backup_folder_path");
      await invoke("reveal_path", { path });
      toast.success(i18n.t("backups.openedFolderOk"));
    } catch (e) {
      const error = String(e);
      toast.error(i18n.t("backups.openFolderFailed", { error }));
    }
  }
</script>

<div class="section">
  <div class="section-head">
    <div class="section-title">
      <History size={18} aria-hidden="true" />
      <h2>{i18n.t("backups.title")}</h2>
    </div>
    <div class="section-actions">
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("backups.openingFolder")}
        onclick={handleOpenFolder}
      >
        <FolderOpen size={14} />
      </button>
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("backups.refresh")}
        disabled={backup.loading}
        onclick={() => backup.refresh()}
      >
        <RefreshCw size={14} class={backup.loading ? "spin" : ""} />
      </button>
    </div>
  </div>

  <p class="lead">{i18n.t("backups.description")}</p>

  {#if backup.entries.length === 0}
    <div class="empty">
      <Inbox size={32} aria-hidden="true" />
      <p>{i18n.t("backups.empty")}</p>
    </div>
  {:else}
    <div class="table-wrap">
      <table class="backups" aria-label={i18n.t("backups.title")}>
        <thead>
          <tr>
            <th scope="col">{i18n.t("backups.columnWhen")}</th>
            <th scope="col">{i18n.t("backups.columnSlug")}</th>
            <th scope="col">{i18n.t("backups.columnTool")}</th>
            <th scope="col">{i18n.t("backups.columnSize")}</th>
            <th scope="col">{i18n.t("backups.columnDest")}</th>
            <th scope="col" class="row-action"><span class="sr-only">{i18n.t("backups.restore")}</span></th>
          </tr>
        </thead>
        <tbody>
          {#each backup.entries as e (e.filename)}
            <tr>
              <td class="mono when">{backup.formatWhen(e)}</td>
              <td class="mono slug">{e.slug}</td>
              <td class="mono tool">{e.tool}</td>
              <td class="num">{backup.formatSize(e)}</td>
              <td class="mono dest" title={e.dest}>{e.dest}</td>
              <td class="row-action">
                <button
                  type="button"
                  class="iconbtn"
                  title={i18n.t("backups.restore")}
                  aria-label={i18n.t("backups.restore")}
                  disabled={backup.restoring !== null}
                  onclick={() => handleRestore(e.filename)}
                >
                  {#if backup.restoring === e.filename}
                    <RefreshCw size={14} class="spin" />
                  {:else}
                    <RotateCcw size={14} />
                  {/if}
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
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
  .spin {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }
  .lead {
    margin: 0;
    color: var(--muted, #888);
    font-size: 0.875rem;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    padding: 2rem 1rem;
    border: 1px dashed var(--border, #2a2a2a);
    border-radius: 6px;
    color: var(--muted, #888);
    text-align: center;
  }
  .empty p {
    margin: 0;
    max-width: 28rem;
  }
  .table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border, #2a2a2a);
    border-radius: 6px;
  }
  table.backups {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8125rem;
  }
  table.backups th,
  table.backups td {
    padding: 0.5rem 0.75rem;
    text-align: left;
    border-bottom: 1px solid var(--border, #2a2a2a);
  }
  table.backups thead th {
    font-weight: 600;
    color: var(--muted, #888);
    background: var(--header, rgba(255, 255, 255, 0.02));
    position: sticky;
    top: 0;
  }
  table.backups tbody tr:last-child td {
    border-bottom: none;
  }
  table.backups tbody tr:hover {
    background: var(--hover, rgba(255, 255, 255, 0.03));
  }
  .mono {
    font-family: var(--mono, ui-monospace, "SF Mono", Menlo, monospace);
    font-size: 0.75rem;
  }
  .num {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .when {
    white-space: nowrap;
  }
  .dest {
    max-width: 18rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-action {
    width: 1%;
    white-space: nowrap;
    text-align: right;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
</style>
