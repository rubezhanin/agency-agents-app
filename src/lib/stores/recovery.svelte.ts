/**
 * Recovery store — listens for the `journal_recovery` event from
 * the Rust startup sweep (`lib.rs::setup` calls
 * `install::recovery::recover_unfinished`, which emits the event
 * when it finds unfinished operations left behind by a
 * crashed process). The store exposes the most recent report and
 * a small accessor for the UI.
 *
 * The event is *one-shot at startup* — the app only ever emits it
 * once, when the boot sweep runs. Subsequent re-renders, route
 * changes, etc. don't refire it. We therefore cache the report in
 * the store and never `unlisten` until the app exits.
 *
 * Mirrors `src-tauri/src/install/recovery.rs` (the backend side
 * that does the actual sweep).
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { toast } from "$lib/stores/toast.svelte";
import { i18n } from "$lib/stores/i18n.svelte";
import { ui } from "$lib/stores/ui.svelte";
import type { RecoveryReport } from "$lib/types";

class RecoveryStore {
  /**
   * The most recent recovery report from the startup sweep.
   * `null` until the first event arrives (or forever, if the
   * backend never fires it — e.g. on a fresh install with no
   * journal). UI should treat `null` and "empty report" the
   * same way.
   */
  report: RecoveryReport | null = $state(null);

  /**
   * `true` once at least one `journal_recovery` event has been
   * received in this session. Stays `true` for the lifetime of
   * the store even if `report` is later cleared.
   */
  seen: boolean = $state(false);

  /**
   * Subscribe to the `journal_recovery` Tauri event. Call from a
   * top-level `onMount` (typically `+layout.svelte`) so it
   * stays alive for the app's lifetime. Returns the unlisten
   * function for hot-reload friendliness, but the caller rarely
   * needs to invoke it.
   */
  async start(): Promise<UnlistenFn> {
    const unlisten = await listen<RecoveryReport>("journal_recovery", (e) => {
      this.report = e.payload;
      this.seen = true;
      this.surface(e.payload);
    });
    return unlisten;
  }

  /**
   * Convenience accessors for the UI.
   */
  hasUnfinished(): boolean {
    return !!this.report && this.report.recoveredCount > 0;
  }

  /** All affected dests across all actions, deduped, sorted. */
  affectedDests(): string[] {
    if (!this.report) return [];
    const out = new Set<string>();
    for (const a of this.report.actions) {
      for (const t of a.targets) out.add(t);
    }
    return Array.from(out).sort();
  }

  /** Clear the cached report (UI dismissal). Does not refire. */
  dismiss(): void {
    this.report = null;
  }

  /**
   * Show a one-time toast per recovery event. The user-facing
   * banner (Settings → Backups, or a startup banner in the
   * sidebar) is wired separately; the toast is the immediate,
   * global, hard-to-miss signal that something needs attention.
   */
  private surface(report: RecoveryReport): void {
    if (report.recoveredCount === 0) {
      // Backend emits even on a clean journal so the listener
      // doesn't have to poll; treat that as a no-op for the
      // user.
      return;
    }
    const n = report.recoveredCount;
    toast.warning(
      i18n.t("recovery.bannerTitle", { count: n }),
      i18n.t("recovery.bannerBody"),
      {
        label: i18n.t("recovery.bannerAction"),
        onClick: () => {
          // The natural destination is the Backups section —
          // the user can see which files are affected and roll
          // any of them back. Open Settings and route there.
          // (The cross-store wiring to `ui` is one-way:
          // `recovery` → `ui` for action, never the other way.
          // No cycle.)
          ui.openSettings("backups");
        },
      },
    );
  }
}

export const recovery = new RecoveryStore();
