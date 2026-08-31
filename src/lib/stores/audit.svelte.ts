/**
 * Audit log store — Phase 5 Trustworthy Core.
 *
 * Reads from the durable `operations.jsonl` log the backend
 * maintains. The frontend never writes entries directly: it calls
 * `record(kind, label, ...)` after a successful user-initiated
 * operation, which sends the entry to the backend, where the
 * `audit_log` IPC stamps the timestamp and appends the line.
 *
 * The store is read-mostly: a single `refresh()` call loads the
 * tail of the log; `record()` triggers an optimistic append + a
 * background refresh so the UI sees the new row without a round
 * trip's delay.
 */

import { invoke } from "@tauri-apps/api/core";

import { i18n } from "$lib/stores/i18n.svelte";
import { toast } from "$lib/stores/toast.svelte";
import type { AuditEntry, AuditOutcome } from "$lib/types";

class AuditStore {
  /** Most recent N entries, newest first. `null` until first refresh. */
  entries: AuditEntry[] = $state([]);
  /** True while a refresh is in flight. */
  loading: boolean = $state(false);
  /** Last error string, surfaced as a toast on `record()` failure. */
  lastError: string | null = $state(null);

  /** Pull the most recent entries from the backend log. */
  async refresh(limit = 100): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    try {
      this.entries = await invoke<AuditEntry[]>("audit_recent", { limit });
      this.lastError = null;
    } catch (e) {
      this.lastError = String(e);
      // Backend not ready → degrade to empty, no toast.
      this.entries = [];
    } finally {
      this.loading = false;
    }
  }

  /**
   * Append a new entry. Server-side stamps the timestamp; the
   * frontend sends `kind` + `label` + optional `targetId`/`detail`
   * and a default `outcome` of `ok`. Returns the freshly-stored
   * entry on success.
   */
  async record(
    kind: string,
    label: string,
    opts: {
      outcome?: AuditOutcome;
      targetId?: string;
      detail?: string;
    } = {},
  ): Promise<AuditEntry | null> {
    const entry: AuditEntry = {
      timestamp: "", // backend stamps
      kind,
      label,
      outcome: opts.outcome ?? "ok",
      targetId: opts.targetId ?? null,
      detail: opts.detail ?? null,
    };
    try {
      await invoke<void>("audit_log", { request: { entry } });
      // Optimistic refresh so the UI sees the new row without
      // waiting for the user to click anything.
      void this.refresh();
      return { ...entry, timestamp: new Date().toISOString() };
    } catch (e) {
      this.lastError = String(e);
      // Audit is best-effort: a write failure should not surface as
      // a destructive toast. The install / hermes flows already
      // show their own success / failure toasts; audit is a
      // passive trail.
      console.warn("audit.record failed:", e);
      return null;
    }
  }

  /**
   * Human label for a `kind` like "install.commit". Falls back to
   * the raw kind when no i18n key matches. Cast through `unknown`
   * because `i18n.t` is typed against a strict union of registered
   * keys; we synthesize the lookup string here.
   */
  kindLabel(kind: string): string {
    const key = `audit.kind.${kind}`;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const translated = (i18n.t as (k: string) => string)(key);
    if (translated === key) return kind;
    return translated;
  }

  /** Human label for an outcome. */
  outcomeLabel(outcome: AuditOutcome): string {
    return i18n.t(`audit.outcome.${outcome}`);
  }

  /** Render a timestamp as a short, locale-aware string. The audit
   * log is RFC 3339 UTC; we strip the timezone for compactness. */
  formatTimestamp(ts: string): string {
    if (!ts) return "—";
    // ts may be empty (optimistic local entry) or full RFC 3339.
    const d = new Date(ts);
    if (Number.isNaN(d.getTime())) return ts;
    return d.toLocaleString();
  }

  /** Notify the user that a recording failed — only on user-initiated
   * flows (not the background poll). Wired to a toast; the user
   * doesn't usually see this. */
  showErrorOnce(): void {
    if (this.lastError) {
      toast.error(this.lastError);
      this.lastError = null;
    }
  }
}

export const audit = new AuditStore();
