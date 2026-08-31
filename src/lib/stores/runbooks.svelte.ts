/**
 * Runbooks store — the NEXUS scenario runbooks from the catalog's
 * `strategy/runbooks.json` (catalog PR #664). Each runbook names its roster BY
 * SLUG; the UI resolves those against the loaded corpus to deploy the set.
 *
 * `strategy/` only ships in a synced catalog (not the bundled snapshot), so an
 * empty list is the normal "not synced yet" state — the UI shows a
 * "sync to unlock" nudge rather than an error. Backend-not-ready posture matches
 * the corpus/install stores: a failed invoke degrades to empty.
 *
 * Phase 5 follow-up — `applyRunbook(slug, tool)` shells out to the
 * `runbook_apply` IPC for a structured one-shot install: the backend
 * resolves the runbook to a deduplicated slug list and the frontend
 * then walks that list through the existing `install` store, which
 * keeps the per-install audit + recovery wiring uniform.
 *
 * Singleton: import `runbooks` everywhere.
 */
import { invoke } from "@tauri-apps/api/core";

import { activity } from "$lib/stores/activity.svelte";
import { audit } from "$lib/stores/audit.svelte";
import { i18n } from "$lib/stores/i18n.svelte";
import { toast } from "$lib/stores/toast.svelte";
import type { Runbook, RunbookApplySummary, Tool } from "$lib/types";

class RunbooksStore {
  /** The scenario runbooks, in manifest order. Empty until loaded / when unsynced. */
  list: Runbook[] = $state([]);
  /** True once the first load resolves (so "empty" ≠ "not fetched yet"). */
  loaded: boolean = $state(false);
  /** True while a load is in flight. */
  loading: boolean = $state(false);
  /** True while `applyRunbook` is in flight. */
  applying: boolean = $state(false);
  /** Last apply summary, or `null` when none has been run yet. */
  lastSummary: RunbookApplySummary | null = $state(null);

  /** Load the manifest from the active catalog. Safe to call on mount. */
  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    try {
      this.list = await invoke<Runbook[]>("runbooks_list");
    } catch {
      this.list = []; // backend not ready / no manifest → empty
    } finally {
      this.loaded = true;
      this.loading = false;
    }
  }

  /**
   * One-shot apply: ask the backend for the runbook's resolved slug
   * list, then install each slug through the existing `install`
   * store. The backend emits the structured summary; the frontend
   * reports it as a toast and surfaces it in `lastSummary` for the
   * UI to render an audit table.
   */
  async applyRunbook(
    runbookSlug: string,
    _tool: Tool = "claude-code",
    _projectPath: string | null = null,
  ): Promise<RunbookApplySummary | null> {
    if (this.applying) return null;
    this.applying = true;
    this.lastSummary = null;
    try {
      const summary = await invoke<RunbookApplySummary>("runbook_apply", {
        request: {
          runbookSlug,
          tool: _tool,
          projectPath: _projectPath ?? null,
        },
      });
      this.lastSummary = summary;

      // Drive the per-slug activity entries so the user sees the
      // per-row outcome in the in-memory activity log. Installed
      // rows are already in the install store's audit log; we
      // only need to surface skips + failures here.
      for (const outcome of summary.outcomes) {
        if (outcome.status === "installed") continue;
        activity.log({
          action: "install",
          agentSlug: outcome.slug,
          tool: _tool,
          scope: _projectPath ? "project" : "user",
          projectPath: _projectPath ?? undefined,
          outcome: outcome.status === "skipped" ? "error" : "error",
          detail: outcome.detail,
        });
      }

      // Audit the rollup — the install store's per-slug audits
      // have already landed, this is the headline the user sees
      // in the Settings → Audit log list.
      const headline = i18n.t("runbooks.applySummary", {
        slug: summary.runbookSlug,
        installed: summary.installed,
        total: summary.total,
      });
      void audit.record("runbook.apply", headline, {
        targetId: summary.runbookSlug,
        detail: `${summary.installed}/${summary.total} installed, ${summary.skipped} skipped, ${summary.failed} failed`,
      });

      // Toast the outcome.
      if (summary.failed === 0 && summary.skipped === 0) {
        toast.success(headline);
      } else if (summary.installed === 0) {
        toast.error(i18n.t("runbooks.applyFailedAll", { slug: summary.runbookSlug }));
      } else {
        toast.success(
          i18n.t("runbooks.applyPartial", {
            installed: summary.installed,
            total: summary.total,
          }),
        );
      }

      return summary;
    } catch (e) {
      toast.error(i18n.t("runbooks.applyFailed", { message: String(e) }));
      return null;
    } finally {
      this.applying = false;
    }
  }
}

export const runbooks = new RunbooksStore();
