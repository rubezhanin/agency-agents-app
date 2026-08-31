# Audit Log

> **Status:** v1.2.0. Durable, append-only operations trail.
> Source of truth: this document + `src-tauri/src/audit.rs` +
> `src-tauri/src/commands/audit.rs` + `src/lib/stores/audit.svelte.ts`
> + `src/lib/components/SettingsSectionAudit.svelte`.

The audit log is a flat, append-only JSONL file at
`<app_data>/audit/operations.jsonl`. Every significant
user-initiated operation lands one row. The UI surfaces the tail
in **Settings → Audit log**; the user can export the whole file as
a pretty-printed JSON array for sharing with a team lead or
incident review.

## 1. What it is for

The audit log answers the question "what did the app do, when, to
what, and with what result?" days after the fact. It is **not** a
general-purpose event log (use Settings → Logs for that) and **not**
a real-time activity feed (use the in-memory `activity` store
for that). The audit log is the durable, crash-safe, auditable
shadow.

Use cases:

- **Incident review.** "Did the install land? Where did it write?
  Why did the plugin dir not appear after the upgrade?" — the log
  has the entry.
- **Team mode.** Export the log and post it to a team channel or
  PR so a teammate can see what changed without remote-poking your
  machine.
- **Forensics.** The log survives app crashes, hard kills, and
  power loss. A partial last line is silently dropped on read, so
  the renderer never sees a malformed row.

## 2. Format

One JSON object per line, RFC 3339 UTC timestamp first:

```json
{
  "timestamp": "2026-08-31T07:58:12.345Z",
  "kind": "install",
  "label": "Install frontend-architect",
  "outcome": "ok",
  "targetId": "frontend-architect",
  "detail": "tool=claude-code"
}
```

Fields:

| field       | type   | required | meaning                                                |
|-------------|--------|----------|--------------------------------------------------------|
| `timestamp` | string | yes      | RFC 3339 UTC; stamped server-side.                     |
| `kind`      | string | yes      | `<area>.<verb>`; free-form for new areas.              |
| `label`     | string | no       | Human label the UI can show.                           |
| `outcome`   | string | yes      | `ok` \| `warn` \| `fail`.                              |
| `targetId`  | string | no       | Slug, plugin id, runbook slug, etc.                    |
| `detail`    | string | no       | Free-form context (tool / project / counts).          |

The `kind` namespace convention is:

- `install.*` — per-agent installs (`install`).
- `hermes.*` — Hermes plugin install / uninstall (`hermes.install`,
  `hermes.uninstall`).
- `runbook.*` — runbook orchestrator (`runbook.apply`).
- `settings.*` — settings mutations (`settings.update`).
- `backup.*` — restore from a backup (`backup.restore`).
- `recovery.*` — startup journal recovery (`recovery.sweep`).

## 3. Write path

`src-tauri/src/audit.rs::append(path, entry)`:

1. `tokio::fs::create_dir_all(parent)` — ensure `<app_data>/audit/`
   exists.
2. `OpenOptions::new().create(true).append(true).open(path)` — open
   the file in append mode (or create it).
3. `serde_json::to_string(entry) + "\n"` — serialize the entry
   to a single line of JSON.
4. `write_all(bytes)` then `sync_all()` — write the line, then
   fsync to disk so a hard kill after the write doesn't lose the
   entry.

There is no read-modify-write; the file is append-only. Two writers
contending on the same line is impossible because each `write` is a
single `O_APPEND` syscall (atomic on POSIX, atomic on Windows for
files opened in append mode).

## 4. Read path

`audit::read_recent(path, limit)`:

1. `tokio::fs::read_to_string(path)` — slurp the whole file. The
   log is expected to stay small (a few hundred lines per user per
   month) so a full read is fine.
2. For each non-empty line, `serde_json::from_str::<AuditEntry>`. A
   partial last line (from a hard kill mid-append) doesn't parse
   and is dropped — the next read will continue past the partial
   fragment.
3. Reverse the parsed entries so the newest is first.
4. Truncate to `limit`.

The IPC `audit_recent(limit)` clamps `limit` to `[1, 500]` and
returns the tail.

## 5. Frontend integration

`src/lib/stores/audit.svelte.ts`:

- `entries: AuditEntry[]` — the most recent N (default 100).
- `record(kind, label, opts?)` — invokes `audit_log`, the backend
  stamps the timestamp. Best-effort: a write failure degrades to a
  `console.warn`, never a destructive toast (install / hermes
  already report their own success toasts).
- `refresh(limit = 100)` — pulls the tail.
- `kindLabel(kind)` / `outcomeLabel(outcome)` — i18n wrappers for
  the UI list.

`src/lib/components/SettingsSectionAudit.svelte`:

- One card with a header (title + Download/Trash2/Refresh
  icon-buttons), an empty state, and a scrollable list of entries.
- Each row: outcome icon (green / yellow / red), kind label +
  outcome pill, label, optional target + detail, formatted
  timestamp.

`src/lib/stores/install.svelte.ts`:

- Emits `audit.record("install", ...)` after a successful
  `install_agent` and a `fail` audit entry when the install
  throws. Other stores (Hermes, runbook, settings) follow the
  same pattern.

## 6. Export

`audit_export(dest)` IPC:

1. Read the full log (no `limit`).
2. `serde_json::to_string_pretty(&entries)` — pretty-printed
   JSON array.
3. Write to `<dest>.tmp-<uuid>`, `fsync`, `rename` over `<dest>` —
   atomic; a crash mid-write leaves the previous destination file
   untouched.
4. Return `AuditExportSummary { path, count }`.

The frontend opens a Tauri save dialog (default
`agency-agents-audit.json`, JSON filter), invokes the IPC, and
toasts the result. The exported file is a plain `AuditEntry[]` —
it round-trips through `serde_json::from_str` and any external
tool, and can be diffed between two users.

## 7. Clear

`audit_clear()` IPC:

1. Read the existing count so the toast can say "Cleared N
   entries" even when the file doesn't exist.
2. `remove_file` — no-op + returns 0 when the file is missing.
3. Return the cleared count.

The UI gates the call behind a `window.confirm` because the
action is irreversible. Exported files on disk are not affected.

## 8. Versioning

| App version | Notes                                                       |
|-------------|-------------------------------------------------------------|
| 1.2.0       | First audit-log release: `audit_log` / `audit_recent` / `audit_export` / `audit_clear`. |

The format is forward-compatible: new fields can be added with
`#[serde(default, skip_serializing_if = "Option::is_none")]` and
older versions of the app will read the new rows without losing
data. Removing a field is a breaking change and requires a
`schema_version` bump on the entry struct.
