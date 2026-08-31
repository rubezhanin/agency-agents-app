<!--
  SettingsSectionAudit — Phase 5 Trustworthy Core.

  Surfaces the durable operations.jsonl log the backend maintains.
  Read-only by design: the user can refresh but never edit the trail.
  Each row is one audit entry: a kind, a human label, a timestamp,
  and an outcome pill (ok / warn / fail). Older entries are
  intentionally kept in the file for forensic value; the UI only
  shows the most recent N (default 100, see `audit.refresh(100)`).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import CheckCircle from "@lucide/svelte/icons/check-circle-2";
  import AlertTriangle from "@lucide/svelte/icons/triangle-alert";
  import XCircle from "@lucide/svelte/icons/x-circle";
  import Info from "@lucide/svelte/icons/info";

  import { audit } from "$lib/stores/audit.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";

  onMount(() => {
    void audit.refresh();
  });
</script>

<div class="section">
  <h2>{i18n.t("audit.title")}</h2>
  <p class="lead">{i18n.t("audit.subtitle")}</p>

  <div class="card">
    <div class="card-head">
      <h3>{i18n.t("audit.title")}</h3>
      <button
        type="button"
        class="iconbtn"
        title={i18n.t("audit.refresh")}
        disabled={audit.loading}
        onclick={() => void audit.refresh()}
      >
        <RefreshCw size={14} class={audit.loading ? "spin" : ""} />
      </button>
    </div>

    {#if audit.entries.length === 0}
      <div class="status-row missing">
        <Info size={16} />
        <span>{i18n.t("audit.empty")}</span>
      </div>
    {:else}
      <ul class="entries">
        {#each audit.entries as e (e.timestamp + e.kind)}
          <li class="entry" data-outcome={e.outcome}>
            <div class="entry-icon">
              {#if e.outcome === "ok"}
                <CheckCircle size={14} />
              {:else if e.outcome === "warn"}
                <AlertTriangle size={14} />
              {:else}
                <XCircle size={14} />
              {/if}
            </div>
            <div class="entry-body">
              <div class="entry-head">
                <span class="entry-kind">{audit.kindLabel(e.kind)}</span>
                <span class="entry-pill" data-outcome={e.outcome}>
                  {audit.outcomeLabel(e.outcome)}
                </span>
              </div>
              {#if e.label}
                <p class="entry-label">{e.label}</p>
              {/if}
              {#if e.detail || e.targetId}
                <p class="entry-meta mono">
                  {#if e.targetId}<span class="entry-target">{e.targetId}</span>{/if}
                  {#if e.targetId && e.detail}<span class="sep">·</span>{/if}
                  {#if e.detail}<span class="entry-detail">{e.detail}</span>{/if}
                </p>
              {/if}
            </div>
            <div class="entry-when mono">{audit.formatTimestamp(e.timestamp)}</div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .section { display: flex; flex-direction: column; gap: var(--space-4); max-width: 720px; }
  h2 {
    font-size: var(--text-h1);
    font-weight: var(--fw-semibold);
    color: var(--color-text-primary);
  }
  .lead {
    font-size: var(--text-body);
    color: var(--color-text-secondary);
    line-height: var(--lh-normal);
    margin: 0;
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
  .card-head h3 {
    font-size: var(--text-h2);
    font-weight: var(--fw-semibold);
    color: var(--color-text-primary);
    margin: 0;
  }
  .status-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-body-sm);
    color: var(--color-text-muted);
  }
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-top: 1px solid var(--color-border);
    padding-top: var(--space-2);
    max-height: 480px;
    overflow-y: auto;
  }
  .entry {
    display: grid;
    grid-template-columns: 20px 1fr auto;
    gap: var(--space-2);
    align-items: start;
    padding: 8px;
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
  }
  .entry[data-outcome="fail"] {
    background: color-mix(in srgb, var(--color-danger) 6%, transparent);
  }
  .entry[data-outcome="warn"] {
    background: color-mix(in srgb, var(--color-warning) 6%, transparent);
  }
  .entry-icon { display: flex; align-items: center; padding-top: 1px; }
  .entry[data-outcome="ok"]   .entry-icon { color: var(--color-success); }
  .entry[data-outcome="warn"] .entry-icon { color: var(--color-warning); }
  .entry[data-outcome="fail"] .entry-icon { color: var(--color-danger); }
  .entry-body { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .entry-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .entry-kind {
    font-size: var(--text-body-sm);
    font-weight: var(--fw-medium);
    color: var(--color-text-primary);
  }
  .entry-pill {
    font-size: var(--text-caption);
    font-weight: var(--fw-semibold);
    padding: 1px 8px;
    border-radius: 999px;
  }
  .entry-pill[data-outcome="ok"] {
    background: color-mix(in srgb, var(--color-success) 18%, transparent);
    color: var(--color-success);
  }
  .entry-pill[data-outcome="warn"] {
    background: color-mix(in srgb, var(--color-warning) 18%, transparent);
    color: var(--color-warning);
  }
  .entry-pill[data-outcome="fail"] {
    background: color-mix(in srgb, var(--color-danger) 18%, transparent);
    color: var(--color-danger);
  }
  .entry-label {
    font-size: var(--text-caption);
    color: var(--color-text-secondary);
    margin: 0;
    word-break: break-word;
  }
  .entry-meta {
    font-size: var(--text-caption);
    color: var(--color-text-muted);
    margin: 0;
    word-break: break-all;
  }
  .entry-meta .sep { margin: 0 6px; opacity: 0.6; }
  .entry-when {
    font-size: var(--text-caption);
    color: var(--color-text-muted);
    white-space: nowrap;
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
    border: 0;
  }
  .iconbtn:hover:not(:disabled) {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }
  .iconbtn:disabled { cursor: default; opacity: 0.5; }
  .spin { animation: spin 1s linear infinite; }
  @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
  .mono { font-family: var(--font-mono); font-size: var(--text-mono); }
</style>
