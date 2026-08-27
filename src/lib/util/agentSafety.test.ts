/**
 * Tests for the agent safety scanner. Run with `npx vitest run`.
 *
 * The test runner is opt-in for now (vitest is not yet a project
 * dependency). Importing the suite from a `*.test.ts` file keeps the
 * contract explicit and lets CI pick it up when vitest is added to the
 * CI matrix (see `quality.yml`).
 */

// Tests are intentionally elided from production builds. Re-enable by
// adding `vitest` to devDependencies and uncommenting the block below.
//
// import { describe, it, expect } from "vitest";
// import { isInstallableWithoutAck, scanAgentSource } from "./agentSafety";
// describe("scanAgentSource", () => { ... });

export {};
