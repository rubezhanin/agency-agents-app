/**
 * Structured frontend logger.
 *
 * The app is local-first and has no telemetry. We do, however, need a
 * place to surface errors during development and beta builds. Every
 * empty-catch block should be replaced with a call to logError() with a
 * stable event name.
 *
 * The logger:
 *   1. Writes a structured JSON line to console.error (devs see it).
 *   2. Appends a row to an in-memory ring buffer the Settings About page
 *      can render, so power users can see what their app has been
 *      complaining about.
 *   3. Never throws. The whole point is that it never breaks the call site.
 *
 * Intentionally NOT sent anywhere. No fetch, no analytics, no Sentry, no
 * log file. The buffer lives only in memory and is lost when the page
 * reloads.
 */

export type LogLevel = "debug" | "info" | "warn" | "error";

export interface LogEntry {
  /** Epoch millis. */
  ts: number;
  /** Log level. */
  level: LogLevel;
  /** Stable event name, e.g. "tool.install.failed", "corpus.parse.skip". */
  event: string;
  /** Free-form structured context. Never log secrets, PII, or full
      persona bodies -- slugs and counts only. */
  context?: Record<string, unknown>;
  /** Optional attached error. We keep .name and .message only. */
  error?: { name: string; message: string };
}

const RING_SIZE = 200;
const ring: LogEntry[] = [];

/** Returns a copy of the in-memory log ring, newest-first. */
export function recentLogs(): readonly LogEntry[] {
  return ring.slice().reverse();
}

/** Clear the in-memory log ring. Used by Settings About "Clear logs". */
export function clearLogs(): void {
  ring.length = 0;
}

function push(entry: LogEntry): void {
  ring.push(entry);
  if (ring.length > RING_SIZE) {
    ring.splice(0, ring.length - RING_SIZE);
  }
  // Best-effort console output. Single-line JSON so it grep's well in
  // DevTools and in any future log-file exporter.
  const line = JSON.stringify({
    level: entry.level,
    event: entry.event,
    ts: entry.ts,
    context: entry.context,
    error: entry.error,
  });
  if (entry.level === "error") {
    console.error(line);
  } else if (entry.level === "warn") {
    console.warn(line);
  } else if (entry.level === "info") {
    console.info(line);
  } else {
    console.debug(line);
  }
}

function normaliseError(e: unknown): { name: string; message: string } | undefined {
  if (e === undefined || e === null) return undefined;
  if (e instanceof Error) {
    return { name: e.name, message: e.message };
  }
  return { name: "NonError", message: String(e) };
}

export function logDebug(event: string, context?: Record<string, unknown>): void {
  push({ ts: Date.now(), level: "debug", event, context });
}

export function logInfo(event: string, context?: Record<string, unknown>): void {
  push({ ts: Date.now(), level: "info", event, context });
}

export function logWarn(event: string, context?: Record<string, unknown>, error?: unknown): void {
  push({ ts: Date.now(), level: "warn", event, context, error: normaliseError(error) });
}

export function logError(event: string, context?: Record<string, unknown>, error?: unknown): void {
  push({ ts: Date.now(), level: "error", event, context, error: normaliseError(error) });
}

/**
 * Wrap a function so that any throw becomes a structured logError entry
 * with a configurable event name. The original error is NOT swallowed --
 * we re-throw it after logging. This is for the "I want a record of the
 * failure before it propagates" use case.
 */
export async function withLogging<T>(
  event: string,
  fn: () => Promise<T>,
  context?: Record<string, unknown>,
): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    logError(event, context, e);
    throw e;
  }
}
