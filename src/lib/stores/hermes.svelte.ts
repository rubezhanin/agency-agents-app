/**
 * Hermes plugin store — drives the install/uninstall flow for the
 * `agency-agents-router` Hermes plugin and probes the local `hermes` CLI.
 *
 * Mirrors `src-tauri/src/commands/hermes.rs` and `src-tauri/src/hermes/probe.rs`.
 * The plugin format itself is documented in `docs/HERMES-PLUGIN.md`.
 *
 * Like the other stores, every invoke is wrapped so a backend-not-ready
 * build degrades to empty state rather than throwing.
 */

import { invoke } from "@tauri-apps/api/core";

import { activity } from "$lib/stores/activity.svelte";
import { i18n } from "$lib/stores/i18n.svelte";
import { toast } from "$lib/stores/toast.svelte";
import type {
  HermesProbe,
  HermesInstallResult,
  HermesPreflight,
  HermesHealthSnapshot,
  HermesHealthStatus,
  HermesInstalledPlugin,
  PreflightCheck,
  PreflightStatus,
  RenderableAgent,
} from "$lib/types";

/** Path users will most often want to see in the UI. */
const INSTALL_PATH_HINT = "~/.hermes/plugins/agency-agents-router/";

class HermesStore {
  /** Last `hermes_status` result, or `null` if never probed / probe failed. */
  status: HermesProbe | null = $state(null);
  /** Last `hermes_preflight` result, or `null` if never run / failed. */
  preflight: HermesPreflight | null = $state(null);
  /** True while a preflight check is in flight. */
  preflighting: boolean = $state(false);
  /** Every installed plugin under `~/.hermes/plugins/`. Cached after
   * each refresh; `null` until the first scan. */
  installedPlugins: HermesInstalledPlugin[] = $state([]);
  /** True while the installed-plugins scan is in flight. */
  listingPlugins: boolean = $state(false);
  /** Aggregated health snapshot (Phase 4c) — probe + preflight +
   * installed plugins in a single round-trip. The frontend polls
   * this on a 60s timer to keep the Hermes settings tile fresh. */
  health: HermesHealthSnapshot | null = $state(null);
  /** True while a health poll is in flight. */
  healthLoading: boolean = $state(false);
  /** Polling handle returned by `startHealthPoll`. `null` when no
   * poll is running. */
  healthPollHandle: number | null = $state(null);
  /** Most recent install result, or `null`. Persisted in memory only. */
  lastInstall: HermesInstallResult | null = $state(null);
  /** True while a status probe is in flight. */
  probing: boolean = $state(false);
  /** True while an install/uninstall/stage is in flight. */
  busy: boolean = $state(false);
  /** Last error message from a Hermes command, or `null`. */
  lastError: string | null = $state(null);

  /**
   * Probe the local `hermes` CLI: PATH → scan-beyond-path → version.
   * Caches the result in `this.status` for the UI to render.
   */
  async refreshStatus(): Promise<HermesProbe | null> {
    if (this.probing) return this.status;
    this.probing = true;
    this.lastError = null;
    try {
      const probe = await invoke<HermesProbe>("hermes_status");
      this.status = probe;
      return probe;
    } catch (e) {
      this.lastError = String(e);
      this.status = null;
      return null;
    } finally {
      this.probing = false;
    }
  }

  /**
   * Run the Hermes pre-flight readiness check (CLI, kanban, Node runtime,
   * home writable, install target). Caches the structured checklist in
   * `this.preflight` so the UI can render a colour-coded status block.
   */
  async refreshPreflight(): Promise<HermesPreflight | null> {
    if (this.preflighting) return this.preflight;
    this.preflighting = true;
    this.lastError = null;
    try {
      const pf = await invoke<HermesPreflight>("hermes_preflight");
      this.preflight = pf;
      return pf;
    } catch (e) {
      this.lastError = String(e);
      this.preflight = null;
      return null;
    } finally {
      this.preflighting = false;
    }
  }

  /**
   * Scan `~/.hermes/plugins/` and return every installed plugin.
   * Used by the multi-plugin UI (Phase 4b) to render the per-plugin
   * table; the canonical `agency-agents-router` is listed alongside
   * any custom division plugins.
   */
  async listInstalledPlugins(): Promise<HermesInstalledPlugin[]> {
    if (this.listingPlugins) return this.installedPlugins;
    this.listingPlugins = true;
    this.lastError = null;
    try {
      const plugins = await invoke<HermesInstalledPlugin[]>("hermes_list_plugins");
      this.installedPlugins = plugins;
      return plugins;
    } catch (e) {
      this.lastError = String(e);
      return this.installedPlugins;
    } finally {
      this.listingPlugins = false;
    }
  }

  /**
   * Run the aggregated `hermes_health` IPC. Bundles probe + preflight
   * + installed-plugins into a single round-trip and a single
   * timestamp so the UI can render an atomic snapshot. Phase 4c.
   */
  async refreshHealth(): Promise<HermesHealthSnapshot | null> {
    if (this.healthLoading) return this.health;
    this.healthLoading = true;
    this.lastError = null;
    try {
      const snap = await invoke<HermesHealthSnapshot>("hermes_health");
      this.health = snap;
      // Mirror the bundled fields into the per-channel state so
      // existing UI bits that read `status` / `preflight` /
      // `installedPlugins` keep working without rewiring.
      this.status = snap.probe;
      this.preflight = snap.preflight;
      this.installedPlugins = snap.plugins;
      return snap;
    } catch (e) {
      this.lastError = String(e);
      return this.health;
    } finally {
      this.healthLoading = false;
    }
  }

  /**
   * Start a `setInterval` poll that re-fetches `hermes_health` every
   * `intervalMs` (default 60s). Returns the interval handle so the
   * caller can clear it. Phase 4c.
   *
   * Idempotent — calling it again with a new interval replaces the
   * existing handle.
   */
  startHealthPoll(intervalMs = 60_000): number {
    if (typeof window === "undefined") return 0;
    this.stopHealthPoll();
    // Fire one immediate refresh so the UI never shows stale state.
    void this.refreshHealth();
    const handle = window.setInterval(() => {
      void this.refreshHealth();
    }, intervalMs);
    this.healthPollHandle = handle;
    return handle;
  }

  /** Cancel the health poll started by `startHealthPoll`. No-op when
   * the poll isn't running. */
  stopHealthPoll(): void {
    if (this.healthPollHandle !== null && typeof window !== "undefined") {
      window.clearInterval(this.healthPollHandle);
      this.healthPollHandle = null;
    }
  }

  /** Human label for an overall health status. Used in the badge
   * on the Hermes settings tile (Phase 4c). */
  healthLabel(status: HermesHealthStatus | null | undefined): string {
    if (!status) return i18n.t("hermes.healthUnknown");
    switch (status) {
      case "ok":
        return i18n.t("hermes.healthOk");
      case "degraded":
        return i18n.t("hermes.healthDegraded");
      case "down":
        return i18n.t("hermes.healthDown");
    }
  }

  /**
   * Pick the first failing check (status === "fail"). Used by the UI to
   * highlight the most urgent remediation in a banner.
   */
  firstFailure(): PreflightCheck | null {
    if (!this.preflight) return null;
    return (
      this.preflight.checks.find((c) => c.status === "fail") ?? null
    );
  }

  /**
   * Count of checks whose status is not "ok". The UI uses this for the
   * "{n} issues found" line in the banner.
   */
  preflightIssueCount(): number {
    if (!this.preflight) return 0;
    return this.preflight.checks.filter((c) => c.status !== "ok").length;
  }

  /**
   * Map a `PreflightStatus` to an i18n key suffix so the UI can pick a
   * human label without sprinkling ternaries across the template.
   */
  statusLabel(status: PreflightStatus): string {
    switch (status) {
      case "ok":
        return i18n.t("hermes.preflightOk");
      case "warn":
        return i18n.t("hermes.preflightWarn");
      case "fail":
        return i18n.t("hermes.preflightFail");
    }
  }

  /**
   * Install the canonical `agency-agents-router` plugin (or a custom
   * plugin when `pluginId` is set). Refreshes the installed-plugins
   * list on success so the multi-plugin table updates immediately.
   */
  async install(
    agents: RenderableAgent[],
    catalogRef: string,
    pluginId?: string,
    pluginLabel?: string,
  ): Promise<HermesInstallResult | null> {
    if (this.busy) return null;
    this.busy = true;
    this.lastError = null;
    try {
      const result = await invoke<HermesInstallResult>("hermes_install", {
        request: {
          agents,
          catalogRef,
          pluginId: pluginId ?? null,
          pluginLabel: pluginLabel ?? null,
        },
      });
      this.lastInstall = result;
      activity.log({
        action: "install",
        outcome: "ok",
        detail: i18n.t("hermes.installSuccess", { count: result.agentCount })
          + " — " + result.installRoot,
      });
      toast.success(i18n.t("hermes.installSuccess", { count: result.agentCount }));
      void this.listInstalledPlugins();
      return result;
    } catch (e) {
      this.lastError = String(e);
      toast.error(i18n.t("hermes.installFailed", { message: String(e) }));
      return null;
    } finally {
      this.busy = false;
    }
  }

  /**
   * Stage the plugin into a user-picked directory (so they can run
   * `hermes plugin install <path>` themselves). The frontend uses
   * `@tauri-apps/plugin-dialog`'s `open({ directory: true })` to pick
   * the destination, then passes the path here. When `pluginId` is
   * set, the staged manifest is labelled accordingly.
   */
  async stage(
    agents: RenderableAgent[],
    catalogRef: string,
    dest: string,
    pluginId?: string,
    pluginLabel?: string,
  ): Promise<HermesInstallResult | null> {
    if (this.busy) return null;
    this.busy = true;
    this.lastError = null;
    try {
      const result = await invoke<HermesInstallResult>("hermes_stage", {
        request: {
          agents,
          catalogRef,
          dest,
          pluginId: pluginId ?? null,
          pluginLabel: pluginLabel ?? null,
        },
      });
      this.lastInstall = result;
      activity.log({
        action: "install",
        outcome: "ok",
        detail: i18n.t("hermes.installSuccess", { count: result.agentCount })
          + " — " + result.installRoot,
      });
      toast.success(i18n.t("hermes.installSuccess", { count: result.agentCount }));
      return result;
    } catch (e) {
      this.lastError = String(e);
      toast.error(i18n.t("hermes.installFailed", { message: String(e) }));
      return null;
    } finally {
      this.busy = false;
    }
  }

  /** Remove an installed plugin. `pluginId` defaults to the canonical
   * `agency-agents-router` when omitted. Refreshes the installed-
   * plugins list on success. Idempotent. */
  async uninstall(pluginId?: string): Promise<boolean> {
    if (this.busy) return false;
    this.busy = true;
    this.lastError = null;
    try {
      await invoke<void>("hermes_uninstall", {
        request: pluginId ? { pluginId } : null,
      });
      this.lastInstall = null;
      activity.log({
        action: "uninstall",
        outcome: "ok",
        detail: i18n.t("hermes.uninstallSuccess"),
      });
      toast.success(i18n.t("hermes.uninstallSuccess"));
      void this.listInstalledPlugins();
      return true;
    } catch (e) {
      this.lastError = String(e);
      toast.error(i18n.t("hermes.uninstallFailed", { message: String(e) }));
      return false;
    } finally {
      this.busy = false;
    }
  }

  /**
   * Format a status line for the UI. Falls back to a "not found" string
   * when no probe has been run yet.
   */
  describeStatus(): string {
    if (!this.status) {
      return i18n.t("hermes.cliMissing", {
        url: "https://hermes-agent.dev/install",
      });
    }
    if (!this.status.found || !this.status.version || !this.status.path) {
      return i18n.t("hermes.cliMissing", {
        url: "https://hermes-agent.dev/install",
      });
    }
    return i18n.t("hermes.cliFound", {
      version: this.status.version,
      path: this.status.path,
    });
  }

  /** Stable hint shown next to the install button. */
  installHint(): string {
    return `${INSTALL_PATH_HINT}`;
  }
}

export const hermes = new HermesStore();
