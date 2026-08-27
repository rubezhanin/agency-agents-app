<script lang="ts">
  /**
   * SettingsSectionHermes.svelte — Hermes plugin installer (0.4.0)
   *
   * Three actions the user can take from this section:
   *
   * 1. **Install as Hermes plugin** — writes the entire catalog as a single
   *    `agency-agents-router` plugin directory to
   *    `~/.hermes/plugins/agency-agents-router/`. The plugin format is
   *    documented in `docs/HERMES-PLUGIN.md`. No `hermes` CLI required —
   *    the user can `hermes plugin install` afterwards from the terminal.
   *
   * 2. **Stage for `hermes plugin install`…** — same renderer, but the user
   *    picks a destination directory via a file dialog. Useful if the
   *    canonical install path is read-only, or the user wants to inspect
   *    the plugin before letting `hermes` see it.
   *
   * 3. **Uninstall** — removes the plugin directory. Idempotent.
   *
   * The status tile surfaces the local `hermes` CLI state (PATH lookup,
   * version, minimum-version check, kanban support, profile list). It is
   * informational only — the install/stage buttons do NOT shell out to
   * `hermes`.
   */

  import { onMount } from "svelte";
  import AlertTriangle from "@lucide/svelte/icons/triangle-alert";
  import CheckCircle from "@lucide/svelte/icons/check-circle-2";
  import XCircle from "@lucide/svelte/icons/x-circle";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import Download from "@lucide/svelte/icons/download";

  import { open as openDialog } from "@tauri-apps/plugin-dialog";

  import { hermes } from "$lib/stores/hermes.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import type { RenderableAgent } from "$lib/types";

  /**
   * Pick the agents we'll render. We send every parsed agent (the whole
   * catalog) — Hermes doesn't filter at the plugin level, it dispatches
   * at runtime based on the router skill. If the catalog is empty (e.g.
   * the user hasn't loaded a source yet), the buttons disable.
   */
  const renderableAgents = $derived<RenderableAgent[]>(
    corpus.agents.map((a) => ({
      slug: a.slug,
      name: a.name,
      description: a.description,
      category: a.category,
      body: a.body ?? "",
    })),
  );

  /** Frozen catalog ref for the manifest. We use a count-based pseudo-ref
      because the corpus store does not yet expose `meta.commit` to the
      frontend; the install will use whatever the renderer accepts. */
  const catalogRef = $derived<string>(
    `local-${corpus.agents.length}`,
  );

  onMount(() => {
    void hermes.refreshStatus();
  });

  async function handleInstall() {
    if (renderableAgents.length === 0) return;
    await hermes.install(renderableAgents, catalogRef);
  }

  async function handleStage() {
    if (renderableAgents.length === 0) return;
    const dest = await openDialog({
      directory: true,
      multiple: false,
      title: i18n.t("hermes.stageForCli"),
    });
    if (typeof dest !== "string") return;
    await hermes.stage(renderableAgents, catalogRef, dest);
  }

  async function handleUninstall() {
    if (!confirm(i18n.t("hermes.uninstallHint"))) return;
    await hermes.uninstall();
  }
</script>

<div class="section">
  <h2>{i18n.t("hermes.pluginName")}</h2>
  <p class="lead">{i18n.t("hermes.pluginDescription")}</p>

  <!-- CLI status tile -->
  <div class="card">
    <div class="card-head">
      <h3>{i18n.t("hermes.cliStatus")}</h3>
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("hermes.refreshStatus")}
        disabled={hermes.probing}
        onclick={() => hermes.refreshStatus()}
      >
        <RefreshCw size={14} class={hermes.probing ? "spin" : ""} />
      </button>
    </div>

    {#if !hermes.status}
      <div class="status-row missing">
        <XCircle size={16} />
        <span>{hermes.describeStatus()}</span>
      </div>
    {:else if !hermes.status.found}
      <div class="status-row missing">
        <XCircle size={16} />
        <span>{hermes.describeStatus()}</span>
      </div>
    {:else}
      <div class="status-row ok">
        <CheckCircle size={16} />
        <span>{hermes.describeStatus()}</span>
      </div>
      <dl class="meta">
        <div class="row">
          <dt>version</dt>
          <dd class="mono">{hermes.status.version ?? "—"}</dd>
        </div>
        <div class="row">
          <dt>minimum</dt>
          <dd class="mono">{hermes.status.minimum}</dd>
        </div>
        <div class="row">
          <dt>compatibility</dt>
          <dd>
            {#if hermes.status.meetsMinimum}
              <span class="ok-tag">{i18n.t("hermes.cliMeetsMinimum", { minimum: hermes.status.minimum })}</span>
            {:else}
              <span class="warn-tag">{i18n.t("hermes.cliOutdated", { minimum: hermes.status.minimum })}</span>
            {/if}
          </dd>
        </div>
        <div class="row">
          <dt>kanban</dt>
          <dd>
            {hermes.status.kanbanAvailable
              ? i18n.t("hermes.cliKanbanAvailable")
              : i18n.t("hermes.cliKanbanMissing")}
          </dd>
        </div>
        {#if hermes.status.profiles.length > 0}
          <div class="row">
            <dt>profiles</dt>
            <dd>{i18n.t("hermes.cliProfiles", { count: hermes.status.profiles.length })}</dd>
          </div>
        {/if}
      </dl>
      {#if hermes.status.configPath}
        <p class="hint mono">{hermes.status.configPath}</p>
      {/if}
    {/if}
  </div>

  <!-- Action buttons -->
  <div class="card">
    <h3>Plugin</h3>
    {#if renderableAgents.length === 0}
      <div class="callout" role="status">
        <AlertTriangle size={16} />
        <span>No agents loaded — pick a catalog source first.</span>
      </div>
    {:else}
      <p class="hint">{i18n.t("hermes.installAsPluginHint")}</p>
      <p class="catalog-line">
        <span class="mono">{renderableAgents.length}</span> agents
        ·
        <span class="mono">{catalogRef}</span>
      </p>
      <div class="actions">
        <button
          type="button"
          class="primary"
          disabled={hermes.busy}
          onclick={handleInstall}
        >
          <Download size={14} />
          <span>{hermes.busy ? i18n.t("hermes.installing") : i18n.t("hermes.installAsPlugin")}</span>
        </button>
        <button
          type="button"
          class="secondary"
          disabled={hermes.busy}
          onclick={handleStage}
        >
          <FolderOpen size={14} />
          <span>{i18n.t("hermes.stageForCli")}</span>
        </button>
        <button
          type="button"
          class="danger"
          disabled={hermes.busy}
          onclick={handleUninstall}
        >
          <Trash2 size={14} />
          <span>{hermes.busy ? i18n.t("hermes.uninstalling") : i18n.t("hermes.uninstall")}</span>
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .section { display: flex; flex-direction: column; gap: var(--space-4); max-width: 640px; }
  h2 {
    font-size: var(--text-h1);
    font-weight: var(--fw-semibold);
    color: var(--color-text-primary);
  }
  h3 {
    font-size: var(--text-h2);
    font-weight: var(--fw-semibold);
    color: var(--color-text-primary);
    margin: 0;
  }
  .lead {
    font-size: var(--text-body);
    color: var(--color-text-secondary);
    line-height: var(--lh-normal);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .status-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-body-sm);
    color: var(--color-text-primary);
  }
  .status-row.missing { color: var(--color-text-muted); }
  .status-row.ok     { color: var(--color-success); }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding-top: var(--space-2);
    border-top: 1px solid var(--color-border);
  }
  .row {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: var(--space-3);
    align-items: baseline;
    padding: 2px 0;
  }
  .row dt {
    font-size: var(--text-caption);
    color: var(--color-text-muted);
    font-weight: var(--fw-medium);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .row dd { font-size: var(--text-body-sm); color: var(--color-text-primary); margin: 0; }
  .mono { font-family: var(--font-mono); font-size: var(--text-mono); }
  .hint {
    font-size: var(--text-body-sm);
    color: var(--color-text-muted);
    line-height: var(--lh-normal);
    margin: 0;
  }
  .catalog-line {
    font-size: var(--text-body-sm);
    color: var(--color-text-secondary);
    margin: 0;
  }
  .ok-tag {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-success) 18%, transparent);
    color: var(--color-success);
    font-size: var(--text-caption);
    font-weight: var(--fw-semibold);
  }
  .warn-tag {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-warning) 18%, transparent);
    color: var(--color-warning);
    font-size: var(--text-caption);
    font-weight: var(--fw-semibold);
  }
  .callout {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: var(--space-3);
    background: color-mix(in srgb, var(--color-warning) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-warning) 30%, transparent);
    border-radius: var(--radius-md);
    color: var(--color-text-secondary);
    font-size: var(--text-body-sm);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .actions button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border-radius: var(--radius-sm);
    font-size: var(--text-body-sm);
    font-weight: var(--fw-medium);
    cursor: pointer;
    transition: background-color var(--motion-duration-fast) var(--motion-ease-out);
  }
  .actions button:disabled { cursor: default; opacity: 0.5; }
  .actions .primary {
    background: var(--color-brand);
    color: var(--color-text-on-brand, white);
    border: 1px solid var(--color-brand);
  }
  .actions .primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-brand) 88%, black);
  }
  .actions .secondary {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    border: 1px solid var(--color-border);
  }
  .actions .secondary:hover:not(:disabled) { background: var(--color-surface-sunken); }
  .actions .danger {
    background: transparent;
    color: var(--color-danger);
    border: 1px solid color-mix(in srgb, var(--color-danger) 30%, transparent);
  }
  .actions .danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-danger) 10%, transparent);
  }
  .iconbtn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    cursor: pointer;
    background: transparent;
  }
  .iconbtn:hover:not(:disabled) {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }
  .iconbtn:disabled { cursor: default; opacity: 0.5; }
  .spin { animation: spin 1s linear infinite; }
  @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
</style>
