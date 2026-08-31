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

  import { onMount, onDestroy } from "svelte";
  import AlertTriangle from "@lucide/svelte/icons/triangle-alert";
  import CheckCircle from "@lucide/svelte/icons/check-circle-2";
  import XCircle from "@lucide/svelte/icons/x-circle";
  import AlertCircle from "@lucide/svelte/icons/circle-alert";
  import Plus from "@lucide/svelte/icons/plus";
  import Package from "@lucide/svelte/icons/package";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import Download from "@lucide/svelte/icons/download";

  import { open as openDialog } from "@tauri-apps/plugin-dialog";

  import { hermes } from "$lib/stores/hermes.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { toast } from "$lib/stores/toast.svelte";
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
    void hermes.refreshPreflight();
    void hermes.listInstalledPlugins();
    // Phase 4c — start a 60s poll that re-fetches the aggregated
    // `hermes_health` snapshot. Cancel on unmount so the timer
    // doesn't outlive the Settings tile.
    hermes.startHealthPoll(60_000);
  });

  onDestroy(() => {
    hermes.stopHealthPoll();
  });

  async function handleInstall() {
    if (renderableAgents.length === 0) return;
    await hermes.install(renderableAgents, catalogRef);
  }

  async function handleInstallCustom() {
    if (renderableAgents.length === 0) return;
    const pluginId = (
      window.prompt(i18n.t("hermes.customPluginIdPrompt"), "engineering-team") ?? ""
    ).trim();
    if (!pluginId) return;
    if (!/^[a-z0-9-]+$/.test(pluginId)) {
      toast.error(i18n.t("hermes.customPluginIdInvalid"));
      return;
    }
    const pluginLabel = (
      window.prompt(i18n.t("hermes.customPluginLabelPrompt"), "Engineering Team") ?? ""
    ).trim();
    if (!pluginLabel) return;
    await hermes.install(renderableAgents, catalogRef, pluginId, pluginLabel);
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

  async function handleUninstallPlugin(pluginId: string) {
    if (!confirm(i18n.t("hermes.uninstallPluginHint", { id: pluginId }))) return;
    await hermes.uninstall(pluginId);
  }
</script>

<div class="section">
  <h2>{i18n.t("hermes.pluginName")}</h2>
  <p class="lead">{i18n.t("hermes.pluginDescription")}</p>

  <!-- Phase 4c — overall health badge + last-checked timestamp.
       Driven by the 60s `hermes_health` poll started in onMount. -->
  {#if hermes.health}
    <div class="health-row">
      <span class="health-badge" data-status={hermes.health.overall}>
        <span class="dot" aria-hidden="true"></span>
        {hermes.healthLabel(hermes.health.overall)}
      </span>
      <span class="health-when mono">
        {i18n.t("hermes.healthCheckedAt", { when: hermes.health.checkedAt })}
      </span>
    </div>
  {/if}

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

  <!-- Readiness checklist (Phase 4a) -->
  <div class="card">
    <div class="card-head">
      <h3>{i18n.t("hermes.preflightTitle")}</h3>
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("hermes.refreshStatus")}
        disabled={hermes.preflighting}
        onclick={() => hermes.refreshPreflight()}
      >
        <RefreshCw size={14} class={hermes.preflighting ? "spin" : ""} />
      </button>
    </div>

    {#if !hermes.preflight}
      <div class="status-row missing">
        <AlertCircle size={16} />
        <span>{i18n.t("hermes.preflightEmpty")}</span>
      </div>
    {:else}
      <div
        class="status-row"
        class:ok={hermes.preflight.ready}
        class:warn={!hermes.preflight.ready}
      >
        {#if hermes.preflight.ready}
          <CheckCircle size={16} />
          <span>{i18n.t("hermes.preflightReady")}</span>
        {:else}
          <AlertTriangle size={16} />
          <span>{i18n.t("hermes.preflightNotReady", { count: hermes.preflightIssueCount() })}</span>
        {/if}
      </div>

      <ul class="checks">
        {#each hermes.preflight.checks as check (check.id)}
          <li class="check" data-status={check.status}>
            <span class="check-icon" aria-hidden="true">
              {#if check.status === "ok"}
                <CheckCircle size={14} />
              {:else if check.status === "warn"}
                <AlertTriangle size={14} />
              {:else}
                <XCircle size={14} />
              {/if}
            </span>
            <div class="check-body">
              <div class="check-head">
                <span class="check-label">{check.label}</span>
                <span class="check-status" data-status={check.status}>
                  {hermes.statusLabel(check.status)}
                </span>
              </div>
              {#if check.detail}
                <p class="check-detail mono">{check.detail}</p>
              {/if}
              {#if check.remediation}
                <p class="check-fix">{check.remediation}</p>
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      <p class="hint mono">checked: {hermes.preflight.checkedAt}</p>
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
          onclick={handleInstallCustom}
          title={i18n.t("hermes.installCustomHint")}
        >
          <Plus size={14} />
          <span>{i18n.t("hermes.installCustom")}</span>
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

  <!-- Installed plugins (Phase 4b — multi-plugin routing) -->
  <div class="card">
    <div class="card-head">
      <h3>{i18n.t("hermes.installedPluginsTitle")}</h3>
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("hermes.refreshPlugins")}
        disabled={hermes.listingPlugins}
        onclick={() => hermes.listInstalledPlugins()}
      >
        <RefreshCw size={14} class={hermes.listingPlugins ? "spin" : ""} />
      </button>
    </div>

    {#if hermes.installedPlugins.length === 0}
      <div class="status-row missing">
        <Package size={16} />
        <span>{i18n.t("hermes.installedPluginsEmpty")}</span>
      </div>
    {:else}
      <ul class="plugins">
        {#each hermes.installedPlugins as p (p.pluginId)}
          <li class="plugin" data-canonical={p.isCanonical}>
            <div class="plugin-main">
              <div class="plugin-id">
                <Package size={14} />
                <span class="mono">{p.pluginId}</span>
                {#if p.isCanonical}
                  <span class="canonical-tag">{i18n.t("hermes.canonical")}</span>
                {/if}
              </div>
              {#if p.label && p.label !== p.pluginId}
                <p class="plugin-label">{p.label}</p>
              {/if}
              <p class="plugin-meta mono">
                <span>{p.agentCount} agents</span>
                <span class="sep">·</span>
                <span class="path">{p.path}</span>
              </p>
            </div>
            <button
              type="button"
              class="iconbtn danger"
              title={i18n.t("hermes.uninstallPlugin", { id: p.pluginId })}
              disabled={hermes.busy}
              onclick={() => handleUninstallPlugin(p.pluginId)}
            >
              <Trash2 size={14} />
            </button>
          </li>
        {/each}
      </ul>
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

  /* Readiness checklist (Phase 4a) */
  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    border-top: 1px solid var(--color-border);
    padding-top: var(--space-2);
  }
  .check {
    display: grid;
    grid-template-columns: 20px 1fr;
    gap: var(--space-2);
    align-items: start;
    padding: 4px 0;
  }
  .check-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding-top: 1px;
  }
  .check[data-status="ok"]   .check-icon { color: var(--color-success); }
  .check[data-status="warn"] .check-icon { color: var(--color-warning); }
  .check[data-status="fail"] .check-icon { color: var(--color-danger); }
  .check-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .check-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .check-label {
    font-size: var(--text-body-sm);
    color: var(--color-text-primary);
    font-weight: var(--fw-medium);
  }
  .check-status {
    font-size: var(--text-caption);
    font-weight: var(--fw-semibold);
    padding: 1px 8px;
    border-radius: 999px;
  }
  .check-status[data-status="ok"] {
    background: color-mix(in srgb, var(--color-success) 18%, transparent);
    color: var(--color-success);
  }
  .check-status[data-status="warn"] {
    background: color-mix(in srgb, var(--color-warning) 18%, transparent);
    color: var(--color-warning);
  }
  .check-status[data-status="fail"] {
    background: color-mix(in srgb, var(--color-danger) 18%, transparent);
    color: var(--color-danger);
  }
  .check-detail {
    font-size: var(--text-caption);
    color: var(--color-text-secondary);
    margin: 0;
    word-break: break-all;
  }
  .check-fix {
    font-size: var(--text-caption);
    color: var(--color-text-muted);
    margin: 0;
    line-height: var(--lh-normal);
  }

  /* Installed plugins (Phase 4b) */
  .plugins {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    border-top: 1px solid var(--color-border);
    padding-top: var(--space-2);
  }
  .plugin {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-2);
    align-items: center;
    padding: 8px 10px;
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }
  .plugin[data-canonical="true"] {
    border-color: color-mix(in srgb, var(--color-brand) 35%, transparent);
  }
  .plugin-main { min-width: 0; }
  .plugin-id {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-body-sm);
    color: var(--color-text-primary);
    font-weight: var(--fw-medium);
  }
  .canonical-tag {
    display: inline-block;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-brand) 18%, transparent);
    color: var(--color-brand);
    font-size: var(--text-caption);
    font-weight: var(--fw-semibold);
  }
  .plugin-label {
    font-size: var(--text-caption);
    color: var(--color-text-secondary);
    margin: 2px 0 0 0;
  }
  .plugin-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    font-size: var(--text-caption);
    color: var(--color-text-muted);
    margin: 2px 0 0 0;
    word-break: break-all;
  }
  .plugin-meta .sep { opacity: 0.6; }
  .plugin-meta .path { opacity: 0.85; }
  .iconbtn.danger { color: var(--color-danger); }
  .iconbtn.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-danger) 12%, transparent);
    color: var(--color-danger);
  }

  /* Phase 4c — overall health badge */
  .health-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .health-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px;
    border-radius: 999px;
    font-size: var(--text-caption);
    font-weight: var(--fw-semibold);
    border: 1px solid var(--color-border);
  }
  .health-badge .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .health-badge[data-status="ok"] {
    background: color-mix(in srgb, var(--color-success) 12%, transparent);
    color: var(--color-success);
    border-color: color-mix(in srgb, var(--color-success) 30%, transparent);
  }
  .health-badge[data-status="ok"] .dot { background: var(--color-success); }
  .health-badge[data-status="degraded"] {
    background: color-mix(in srgb, var(--color-warning) 12%, transparent);
    color: var(--color-warning);
    border-color: color-mix(in srgb, var(--color-warning) 30%, transparent);
  }
  .health-badge[data-status="degraded"] .dot { background: var(--color-warning); }
  .health-badge[data-status="down"] {
    background: color-mix(in srgb, var(--color-danger) 12%, transparent);
    color: var(--color-danger);
    border-color: color-mix(in srgb, var(--color-danger) 30%, transparent);
  }
  .health-badge[data-status="down"] .dot { background: var(--color-danger); }
  .health-when {
    font-size: var(--text-caption);
    color: var(--color-text-muted);
  }
</style>
