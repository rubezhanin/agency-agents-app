<!--
  DeployPreviewModal — Phase 3 follow-up UI for the `deploy_plan` IPC.

  Surfaces the structured plan (creates / overwrites / no-changes /
  refusals) as a colour-coded table so the user can see exactly what
  an install would do before committing. The modal is open-only:
  the parent component decides when to call `install.openPreview()`
  and when to wire the "Proceed" button to the actual install call.
-->
<script lang="ts">
  import AlertTriangle from "@lucide/svelte/icons/triangle-alert";
  import CheckCircle from "@lucide/svelte/icons/check-circle-2";
  import FilePlus from "@lucide/svelte/icons/file-plus";
  import FileEdit from "@lucide/svelte/icons/file-edit";
  import FileX from "@lucide/svelte/icons/file-x-2";
  import Info from "@lucide/svelte/icons/info";
  import X from "@lucide/svelte/icons/x";

  import { install } from "$lib/stores/install.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import type { PlanChange, PlanSummary } from "$lib/types";

  type Props = {
    open: boolean;
    onClose: () => void;
    /** Called when the user clicks "Proceed". The plan + summary are
     * passed so the parent can re-use the same targets for the actual
     * install call. */
    onProceed?: (changes: PlanChange[], summary: PlanSummary) => void;
    /** Optional title override (e.g. "Preview · 5 agents"). */
    title?: string;
  };

  let { open, onClose, onProceed, title }: Props = $props();

  /** Filter out the "noChange" rows by default; the user can expand
   * the section to see them when they want a full audit. */
  let showUnchanged = $state(false);

  const plan = $derived(install.previewPlan);
  const busy = $derived(install.previewBusy);
  const error = $derived(install.previewError);

  const creates = $derived(plan?.changes.filter((c) => c.kind === "create") ?? []);
  const overwrites = $derived(plan?.changes.filter((c) => c.kind === "overwrite") ?? []);
  const refused = $derived(plan?.changes.filter((c) => c.kind === "refused") ?? []);
  const unchanged = $derived(plan?.changes.filter((c) => c.kind === "noChange") ?? []);
  /** Destructiveness — overwriting or refusing any file makes the
   * plan unsafe to apply without explicit review. Computed in TS
   * because `is_destructive()` is a Rust method that doesn't make
   * it across the wire (ts-rs exports data, not behaviour). */
  const destructive = $derived(overwrites.length > 0 || refused.length > 0);
  /** Total file count. Same reasoning as `destructive` — the
   * Rust `total()` method is not in the wire DTO. */
  const totalCount = $derived(
    plan ? plan.summary.creates + plan.summary.overwrites + plan.summary.noChanges + plan.summary.refused : 0,
  );

  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  }

  function shortSha(sha: string | undefined | null): string {
    if (!sha) return "—";
    return sha.length >= 8 ? sha.slice(0, 8) : sha;
  }

  function handleProceed() {
    if (!plan) return;
    onProceed?.(plan.changes, plan.summary);
  }
</script>

{#if open}
  <div
    class="modal-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="deploy-preview-title"
  >
    <div class="modal">
      <header class="head">
        <h2 id="deploy-preview-title">
          {title ?? i18n.t("deployPreview.title")}
        </h2>
        <button class="iconbtn" type="button" aria-label={i18n.t("deployPreview.close")} onclick={onClose}>
          <X size={16} />
        </button>
      </header>

      {#if busy && !plan}
        <div class="loading">
          <Info size={16} />
          <span>{i18n.t("deployPreview.loading")}</span>
        </div>
      {:else if error}
        <div class="error">
          <AlertTriangle size={16} />
          <div>
            <strong>{i18n.t("deployPreview.errorTitle")}</strong>
            <p class="mono">{error}</p>
          </div>
        </div>
      {:else if plan}
        <!-- Summary cards: creates / overwrites / unchanged / refused -->
        <div class="summary">
          <div class="card" data-kind="create">
            <FilePlus size={16} />
            <div class="card-num">{plan.summary.creates}</div>
            <div class="card-label">{i18n.t("deployPreview.creates")}</div>
          </div>
          <div class="card" data-kind="overwrite">
            <FileEdit size={16} />
            <div class="card-num">{plan.summary.overwrites}</div>
            <div class="card-label">{i18n.t("deployPreview.overwrites")}</div>
          </div>
          <div class="card" data-kind="unchanged">
            <CheckCircle size={16} />
            <div class="card-num">{plan.summary.noChanges}</div>
            <div class="card-label">{i18n.t("deployPreview.unchanged")}</div>
          </div>
          <div class="card" data-kind="refused">
            <FileX size={16} />
            <div class="card-num">{plan.summary.refused}</div>
            <div class="card-label">{i18n.t("deployPreview.refused")}</div>
          </div>
        </div>

        {#if destructive}
          <div class="warn">
            <AlertTriangle size={16} />
            <span>{i18n.t("deployPreview.destructiveWarning")}</span>
          </div>
        {/if}

        <!-- Detailed change list, grouped by kind. -->
        <div class="changes">
          {#if creates.length > 0}
            <section>
              <h3>
                <FilePlus size={14} /> {i18n.t("deployPreview.sectionCreates", { count: creates.length })}
              </h3>
              <ul>
                {#each creates as c, i (i)}
                  <li data-kind="create">
                    <span class="path mono">{c.dest}</span>
                    <span class="meta">{formatBytes(Number(c.size))}</span>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          {#if overwrites.length > 0}
            <section>
              <h3>
                <FileEdit size={14} /> {i18n.t("deployPreview.sectionOverwrites", { count: overwrites.length })}
              </h3>
              <ul>
                {#each overwrites as c, i (i)}
                  <li data-kind="overwrite">
                    <span class="path mono">{c.dest}</span>
                    <span class="meta">
                      {shortSha(c.before_sha)} → {shortSha(c.after_sha)}
                      · {i18n.t("deployPreview.backup", { name: c.backup_filename })}
                    </span>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          {#if refused.length > 0}
            <section>
              <h3>
                <FileX size={14} /> {i18n.t("deployPreview.sectionRefused", { count: refused.length })}
              </h3>
              <ul>
                {#each refused as c, i (i)}
                  <li data-kind="refused">
                    <span class="path mono">{c.dest}</span>
                    <span class="meta">{c.reason}</span>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          {#if unchanged.length > 0}
            <section>
              <button class="toggle" type="button" onclick={() => (showUnchanged = !showUnchanged)}>
                {showUnchanged ? i18n.t("deployPreview.hideUnchanged") : i18n.t("deployPreview.showUnchanged", { count: unchanged.length })}
              </button>
              {#if showUnchanged}
                <ul>
                  {#each unchanged as c, i (i)}
                    <li data-kind="unchanged">
                      <span class="path mono">{c.dest}</span>
                      <span class="meta">{shortSha(c.sha)}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
          {/if}
        </div>
      {/if}

      <footer class="foot">
        <button class="ghost" type="button" onclick={onClose}>
          {i18n.t("deployPreview.cancel")}
        </button>
        <button
          class="primary"
          type="button"
          disabled={!plan || totalCount === 0}
          onclick={handleProceed}
        >
          {destructive ? i18n.t("deployPreview.proceedAnyway") : i18n.t("deployPreview.proceed")}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, black 50%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
    padding: var(--space-4);
  }
  .modal {
    background: var(--color-surface);
    color: var(--color-text-primary);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-border);
    width: 100%;
    max-width: 720px;
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4);
    border-bottom: 1px solid var(--color-border);
  }
  .head h2 {
    margin: 0;
    font-size: var(--text-h2);
    font-weight: var(--fw-semibold);
  }
  .iconbtn {
    background: transparent;
    border: 0;
    color: var(--color-text-muted);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm);
  }
  .iconbtn:hover {
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
  }
  .loading,
  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-4);
    color: var(--color-text-secondary);
  }
  .error { color: var(--color-danger); }
  .error p { margin: 4px 0 0 0; font-size: var(--text-caption); }
  .summary {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-border);
  }
  .card-num { font-size: var(--text-h2); font-weight: var(--fw-semibold); }
  .card-label { font-size: var(--text-caption); color: var(--color-text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .card[data-kind="create"] { color: var(--color-brand); }
  .card[data-kind="overwrite"] { color: var(--color-warning); }
  .card[data-kind="unchanged"] { color: var(--color-text-muted); }
  .card[data-kind="refused"] { color: var(--color-danger); }
  .warn {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    margin: 0 var(--space-4);
    background: color-mix(in srgb, var(--color-warning) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-warning) 30%, transparent);
    border-radius: var(--radius-sm);
    color: var(--color-warning);
    font-size: var(--text-body-sm);
  }
  .changes {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-3) var(--space-4) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .changes section h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 4px 0;
    font-size: var(--text-body-sm);
    font-weight: var(--fw-semibold);
    color: var(--color-text-secondary);
  }
  .changes ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .changes li {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    background: var(--color-surface-sunken);
    font-size: var(--text-body-sm);
  }
  .changes li[data-kind="overwrite"] {
    background: color-mix(in srgb, var(--color-warning) 8%, transparent);
  }
  .changes li[data-kind="refused"] {
    background: color-mix(in srgb, var(--color-danger) 8%, transparent);
  }
  .changes li[data-kind="unchanged"] {
    opacity: 0.7;
  }
  .path { flex: 1; min-width: 0; word-break: break-all; }
  .meta { color: var(--color-text-muted); font-size: var(--text-caption); white-space: nowrap; }
  .toggle {
    background: transparent;
    border: 0;
    color: var(--color-text-muted);
    cursor: pointer;
    font-size: var(--text-caption);
    padding: 4px 0;
  }
  .toggle:hover { color: var(--color-text-primary); }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--color-border);
  }
  .ghost {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    border: 1px solid var(--color-border);
    height: 30px;
    padding: 0 12px;
    border-radius: var(--radius-sm);
    font-size: var(--text-body-sm);
    cursor: pointer;
  }
  .primary {
    background: var(--color-brand);
    color: var(--color-text-on-brand, white);
    border: 1px solid var(--color-brand);
    height: 30px;
    padding: 0 14px;
    border-radius: var(--radius-sm);
    font-size: var(--text-body-sm);
    font-weight: var(--fw-medium);
    cursor: pointer;
  }
  .primary:disabled { opacity: 0.5; cursor: default; }
  .mono { font-family: var(--font-mono); font-size: var(--text-mono); }
</style>
