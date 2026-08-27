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
  RenderableAgent,
} from "$lib/types";

/** Path users will most often want to see in the UI. */
const INSTALL_PATH_HINT = "~/.hermes/plugins/agency-agents-router/";

class HermesStore {
  /** Last `hermes_status` result, or `null` if never probed / probe failed. */
  status: HermesProbe | null = $state(null);
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
   * Install the `agency-agents-router` plugin into the canonical user
   * location (`~/.hermes/plugins/agency-agents-router/`). Refreshes the
   * install record cache and emits an activity event on success.
   */
  async install(agents: RenderableAgent[], catalogRef: string): Promise<HermesInstallResult | null> {
    if (this.busy) return null;
    this.busy = true;
    this.lastError = null;
    try {
      const result = await invoke<HermesInstallResult>("hermes_install", {
        request: { agents, catalogRef },
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

  /**
   * Stage the plugin into a user-picked directory (so they can run
   * `hermes plugin install <path>` themselves). The frontend uses
   * `@tauri-apps/plugin-dialog`'s `open({ directory: true })` to pick
   * the destination, then passes the path here.
   */
  async stage(
    agents: RenderableAgent[],
    catalogRef: string,
    dest: string,
  ): Promise<HermesInstallResult | null> {
    if (this.busy) return null;
    this.busy = true;
    this.lastError = null;
    try {
      const result = await invoke<HermesInstallResult>("hermes_stage", {
        request: { agents, catalogRef, dest },
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

  /** Remove the installed plugin directory. Idempotent. */
  async uninstall(): Promise<boolean> {
    if (this.busy) return false;
    this.busy = true;
    this.lastError = null;
    try {
      await invoke<void>("hermes_uninstall");
      this.lastInstall = null;
      activity.log({
        action: "uninstall",
        outcome: "ok",
        detail: i18n.t("hermes.uninstallSuccess"),
      });
      toast.success(i18n.t("hermes.uninstallSuccess"));
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
