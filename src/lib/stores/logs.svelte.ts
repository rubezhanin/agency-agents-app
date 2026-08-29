/**
 * Logs store — surfaces the per-app `logs/app.YYYY-MM-DD.json` files
 * written by the Rust `tracing-appender` rolling file layer (set up
 * in `lib.rs::setup`).
 *
 * Mirrors `src-tauri/src/commands/logs.rs`. Every invoke is wrapped
 * so a backend-not-ready build degrades to empty state rather than
 * throwing.
 */

import { invoke } from "@tauri-apps/api/core";

import { i18n } from "$lib/stores/i18n.svelte";
import { toast } from "$lib/stores/toast.svelte";
import type { LogFile } from "$lib/types";

class LogsStore {
  /** The most recent `logs_list` result, newest first. */
  files: LogFile[] = $state([]);
  /** True while a refresh is in flight. */
  loading: boolean = $state(false);
  /** Name of the file whose tail is currently loaded into `current`. */
  openFile: string | null = $state(null);
  /** Tail of the open file, or empty string if none / load failed. */
  current: string = $state("");
  /** True while a tail read is in flight. */
  reading: boolean = $state(false);
  /** True while a clear is in flight. */
  clearing: boolean = $state(false);
  /** Last error message from a logs command, or `null`. */
  lastError: string | null = $state(null);

  /** Refresh the file list from Rust. */
  async refresh(): Promise<LogFile[]> {
    this.loading = true;
    this.lastError = null;
    try {
      const list = await invoke<LogFile[]>("logs_list");
      this.files = list;
      // If the previously open file vanished, drop it from the view.
      if (this.openFile && !list.find((f) => f.name === this.openFile)) {
        this.openFile = null;
        this.current = "";
      }
      return list;
    } catch (e) {
      this.lastError = String(e);
      this.files = [];
      return [];
    } finally {
      this.loading = false;
    }
  }

  /** Read the tail of `name` and stash it in `current`. */
  async open(name: string): Promise<void> {
    if (this.reading) return;
    this.reading = true;
    this.lastError = null;
    try {
      const tail = await invoke<string>("logs_read", { name });
      this.openFile = name;
      this.current = tail;
    } catch (e) {
      this.lastError = String(e);
      this.openFile = null;
      this.current = "";
    } finally {
      this.reading = false;
    }
  }

  /** Close the open tail view. */
  close(): void {
    this.openFile = null;
    this.current = "";
  }

  /** Wipe every log file. Confirms via the caller (the button) first. */
  async clear(): Promise<boolean> {
    if (this.clearing) return false;
    this.clearing = true;
    try {
      const removed = await invoke<number>("logs_clear");
      this.files = [];
      this.openFile = null;
      this.current = "";
      toast.success(i18n.t("logs.clearedOk", { count: removed }));
      return true;
    } catch (e) {
      const error = String(e);
      this.lastError = error;
      toast.error(i18n.t("logs.clearFailed", { error }));
      return false;
    } finally {
      this.clearing = false;
    }
  }

  /** Human-readable "when" string for a log file. */
  formatWhen(file: LogFile): string {
    try {
      const d = new Date(file.createdAt);
      if (Number.isNaN(d.getTime())) return file.createdAt;
      return d.toLocaleString();
    } catch {
      return file.createdAt;
    }
  }

  /** Human-readable byte size, e.g. "4.2 KB" / "1.0 MB". */
  formatSize(file: LogFile): string {
    const n = Number(file.size);
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

export const logs = new LogsStore();
