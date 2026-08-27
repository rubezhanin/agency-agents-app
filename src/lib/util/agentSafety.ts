/**
 * Pre-flight agent safety scanner.
 *
 * Catalog personas are full Markdown files that get rendered into the user's
 * AI-tool agents directory (Claude Code, Codex, Hermes, etc.). A bad persona
 * could exfiltrate secrets, run shell, or smuggle instructions aimed at the
 * downstream agent. The scanner flags anything suspicious so the UI can
 * require an explicit "I trust this" acknowledgement before install.
 *
 * Scope is intentionally narrow. We do **not** try to prove the persona is
 * safe; we surface things that are clearly out of place for a persona
 * ("design a frontend", "review a PR") and let the human decide. The
 * scanner is a safety net, not a sandbox.
 *
 * Three severity tiers:
 *
 *  - `critical` — almost certainly malicious. Block install until the
 *    user explicitly acknowledges. Examples: an exfiltration pattern
 *    (`cat ~/.ssh/id_rsa`), an exec pattern (`curl … | sh`).
 *  - `high`     — strongly discouraged in a persona. Examples: `rm -rf`,
 *    `sudo`, reading `.env`, `eval(`, hidden / zero-width characters.
 *  - `medium`   — suspicious. Could be legitimate (e.g. "do not run rm
 *    -rf" inside an example) but worth a glance.
 *
 * The matcher list is intentionally small and hand-curated; false
 * positives are easier to debug than a missed exfil.
 */

export type SafetyLevel = "ok" | "medium" | "high" | "critical";

export interface SafetyFinding {
  /** Stable id so the UI can suppress duplicates across scans. */
  id: string;
  /** Severity tier — drives UI treatment. */
  level: Exclude<SafetyLevel, "ok">;
  /** Human-readable pattern name, e.g. "shell pipe-to-sh". */
  name: string;
  /** The matched substring (single line, trimmed). */
  match: string;
  /** 1-based line number in the source the match was found on. */
  line: number;
}

interface Rule {
  id: string;
  name: string;
  level: SafetyFinding["level"];
  /** Compiled regex. Flags `i` for case-insensitive, `g` is added at scan time. */
  pattern: RegExp;
}

const RULES: Rule[] = [
  // ----- CRITICAL: almost certainly malicious --------------------------------
  {
    id: "shell-pipe-to-sh",
    name: "shell pipe-to-sh",
    level: "critical",
    pattern: /\b(curl|wget|fetch)\b[^\n]{0,200}\|\s*(sh|bash|zsh|ksh|dash)\b/i,
  },
  {
    id: "ssh-secret-exfil",
    name: "SSH / secret-key exfiltration",
    level: "critical",
    pattern: /\b(cat|head|tail|less|more|xxd|base64)\b[^\n]{0,200}(\.ssh\/(id_rsa|id_ed25519|known_hosts)|\.aws\/credentials|\.npmrc|\.pypirc|\.netrc|\.git\/config)/i,
  },
  {
    id: "env-var-exfil",
    name: "environment-variable exfiltration",
    level: "critical",
    pattern: /\b(printenv|env)\b[^\n]{0,40}\|?\s*(curl|wget|nc|bash|sh)\b/i,
  },
  {
    id: "reverse-shell",
    name: "reverse shell",
    level: "critical",
    pattern: /\bbash\s+-i\s+>&\s*\/dev\/tcp\/|\bnc\s+-e\s|\bmkfifo\b.*\b(sh|bash)\b/i,
  },

  // ----- HIGH: strongly discouraged in a persona ------------------------------
  {
    id: "destructive-rm-rf",
    name: "destructive rm -rf",
    level: "high",
    pattern: /\brm\s+(-[a-zA-Z]*r[a-zA-Z]*f|-[a-zA-Z]*f[a-zA-Z]*r|-rf|-fr)\b[^\n]{0,80}(~|\\|\/)/i,
  },
  {
    id: "sudo",
    name: "sudo invocation",
    level: "high",
    pattern: /\bsudo\b[^\n]{0,80}(rm|mv|chmod|chown|kill|apt|brew|pacman|systemctl)\b/i,
  },
  {
    id: "dotenv-read",
    name: "read of .env file",
    level: "high",
    pattern: /\b(cat|head|less|more|source|export)\b[^\n]{0,80}\.env(\b|\.)/i,
  },
  {
    id: "eval-exec",
    name: "eval/exec at runtime",
    level: "high",
    pattern: /\b(eval|exec|Function\s*)\s*\(\s*[`"']/i,
  },
  {
    id: "hidden-chars",
    name: "hidden / zero-width characters",
    level: "high",
    pattern: /[\u200B-\u200F\u2028-\u202F\u2060\uFEFF]/,
  },
  {
    id: "prompt-injection",
    name: "prompt-injection style instruction",
    level: "high",
    pattern: /\bignore\s+(all\s+)?previous\s+instructions\b|\bdisregard\s+(all\s+)?prior\b|\byou\s+are\s+now\s+(a|an)\s+/i,
  },
  {
    id: "outbound-http",
    name: "outbound HTTP from a persona",
    level: "medium",
    pattern: /\bhttps?:\/\/(?!github\.com\/rubezhanin\/)[a-z0-9.-]+\.[a-z]{2,}\b/i,
  },

  // ----- MEDIUM: worth a glance -----------------------------------------------
  {
    id: "shell-pipe",
    name: "shell pipe (any kind)",
    level: "medium",
    pattern: /\|/,
  },
  {
    id: "backticks",
    name: "backtick code spans in the body",
    level: "medium",
    pattern: /`[^`\n]{0,200}`/,
  },
  {
    id: "html-script",
    name: "HTML <script> in the body",
    level: "medium",
    pattern: /<\s*script\b/i,
  },
  {
    id: "iframe",
    name: "HTML <iframe> in the body",
    level: "medium",
    pattern: /<\s*iframe\b/i,
  },
];

export interface ScanResult {
  /** Worst severity found, `ok` if nothing matched. */
  level: SafetyLevel;
  /** All findings, sorted by line number. */
  findings: SafetyFinding[];
  /** Count of findings at each non-ok level. */
  counts: { medium: number; high: number; critical: number };
}

/**
 * Scan an agent source. Returns the worst severity, all findings, and
 * counts. Findings are de-duplicated per `(ruleId, line, match)`.
 */
export function scanAgentSource(source: string): ScanResult {
  const lines = source.split(/\r?\n/);
  const findings: SafetyFinding[] = [];
  const seen = new Set<string>();

  for (const rule of RULES) {
    // Per-line so we can report line numbers and so a long body doesn't
    // produce one huge match that confuses the UI.
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i] ?? "";
      rule.pattern.lastIndex = 0;
      const m = rule.pattern.exec(line);
      if (!m) continue;
      const snippet = m[0].trim().slice(0, 120);
      const key = `${rule.id}|${i + 1}|${snippet}`;
      if (seen.has(key)) continue;
      seen.add(key);
      findings.push({
        id: rule.id,
        level: rule.level,
        name: rule.name,
        match: snippet,
        line: i + 1,
      });
    }
  }

  findings.sort((a, b) => a.line - b.line);

  const counts = findings.reduce(
    (acc, f) => {
      acc[f.level] += 1;
      return acc;
    },
    { medium: 0, high: 0, critical: 0 },
  );

  const level: SafetyLevel =
    counts.critical > 0
      ? "critical"
      : counts.high > 0
        ? "high"
        : counts.medium > 0
          ? "medium"
          : "ok";

  return { level, findings, counts };
}

/**
 * True when a persona is safe to install without a confirmation prompt.
 * Mirrors the install modal's gating: `ok` and `medium` install freely;
 * `high` and `critical` require an explicit user acknowledgement.
 */
export function isInstallableWithoutAck(result: ScanResult): boolean {
  return result.level === "ok" || result.level === "medium";
}
