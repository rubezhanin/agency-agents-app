/**
 * Backups store — surfaces the per-app `backups/index.json` ledger
 * and drives the `backup_restore` IPC. Mirrors `src-tauri/src/install/mod.rs`
 * (the `BackupIndex` / `record_backup_entries` / `backup_restore` trio).
 *
 * The list is owned by Rust: this store is a pure read/write view of
 * the on-disk ledger. Every invoke is wrapped so a backend-not-ready
 * build degrades to empty state rather than throwing.
 */

import { invoke } from "@tauri-apps/api/core";

import { i18n } from "$lib/stores/i18n.svelte";
import { toast } from "$lib/stores/toast.svelte";
import type { BackupEntry } from "$lib/types";

class BackupStore {
  /** The most recent `backup_list` result, newest first. */
  entries: BackupEntry[] = $state([]);
  /** True while a refresh is in flight. */
  loading: boolean = $state(false);
  /** True while a restore is in flight (per-row button reflects this). */
  restoring: string | null = $state(null);
  /** Last error message from a backup command, or `null`. */
  lastError: string | null = $state(null);

  /**
   * Refresh the in-memory list from Rust. Idempotent — call it after
   * any operation that might have changed the ledger (install,
   * update, restore).
   */
  async refresh(): Promise<BackupEntry[]> {
    this.loading = true;
    this.lastError = null;
    try {
      const list = await invoke<BackupEntry[]>("backup_list");
      this.entries = list;
      return list;
    } catch (e) {
      this.lastError = String(e);
      this.entries = [];
      return [];
    } finally {
      this.loading = false;
    }
  }

  /**
   * Restore a backup by `filename` (as listed in `entries`). The
   * command reads the bytes from `app_data/backups/{filename}` and
   * writes them back to the recorded `dest`, then drops the row
   * from the index. We refresh the local list afterwards so the
   * restored row disappears from the UI.
   */
  async restore(filename: string): Promise<boolean> {
    if (this.restoring) return false;
    this.restoring = filename;
    try {
      const dest = await invoke<string>("backup_restore", { filename });
      this.entries = this.entries.filter((e) => e.filename !== filename);
      toast.success(
        i18n.t("backups.restoredOk", { filename, dest }),
      );
      return true;
    } catch (e) {
      const error = String(e);
      this.lastError = error;
      toast.error(i18n.t("backups.restoreFailed", { error }));
      return false;
    } finally {
      this.restoring = null;
    }
  }

  /**
   * Human-readable "when" string for a backup row. Falls back to the
   * raw `created_at` if the browser can't parse it (e.g. an exotic
   * locale on a hand-edited index).
   */
  formatWhen(entry: BackupEntry): string {
    try {
      const d = new Date(entry.createdAt);
      if (Number.isNaN(d.getTime())) return entry.createdAt;
      return d.toLocaleString();
    } catch {
      return entry.createdAt;
    }
  }

  /** Human-readable byte size, e.g. "4.2 KB" / "1.0 MB". */
  formatSize(entry: BackupEntry): string {
    const n = Number(entry.size);
    if (!Number.isFinite(n) || n <= 0) return "—";
    const units = ["B", "KB", "MB", "GB"];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    const formatted = v < 10 && i > 0 ? v.toFixed(1) : Math.round(v).toString();
    return `${formatted} ${units[i]}`;
  }
}

export const backup = new BackupStore();
