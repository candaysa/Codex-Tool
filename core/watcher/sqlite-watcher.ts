import { DatabaseSync } from 'node:sqlite';
import { readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import os from 'node:os';

export interface TurnFailureRecord {
  threadId: string;
  turnId: string;
  rolloutOrdinal: number;
  status: string;
  errorJson: string | null;
  startedAt: number;
  completedAt: number;
}

export interface ThreadInfo {
  id: string;
  rolloutPath: string | null;
  title: string | null;
  name: string | null;
  cwd: string | null;
  model: string | null;
  updatedAtMs: number;
  createdAtMs: number;
  archived: boolean;
}

export function getDefaultCodexHome(): string {
  const envCodex = process.env.CODEX_HOME?.trim();
  if (envCodex) return envCodex;
  const userHome = process.env.USERPROFILE?.trim() || os.homedir();
  return join(userHome, '.codex');
}

/**
 * Discovers candidate state SQLite databases in ~/.codex (e.g. state_5.sqlite, state_4.sqlite)
 */
export function findStateDatabasePaths(codexHome?: string): string[] {
  const home = codexHome || getDefaultCodexHome();
  if (!existsSync(home)) return [];

  try {
    const entries = readdirSync(home);
    const thMatches: Array<{ path: string; gen: number }> = [];
    const stateMatches: Array<{ path: string; gen: number }> = [];

    for (const file of entries) {
      const thMatch = /^thread_history_(\d+)\.sqlite$/.exec(file);
      if (thMatch && thMatch[1]) {
        thMatches.push({
          path: join(home, file),
          gen: Number.parseInt(thMatch[1], 10),
        });
      }
      const match = /^state_(\d+)\.sqlite$/.exec(file);
      if (match && match[1]) {
        stateMatches.push({
          path: join(home, file),
          gen: Number.parseInt(match[1], 10),
        });
      }
    }

    thMatches.sort((a, b) => b.gen - a.gen);
    stateMatches.sort((a, b) => b.gen - a.gen);
    return [...thMatches.map(m => m.path), ...stateMatches.map(m => m.path)];
  } catch {
    return [];
  }
}

/**
 * Finds recently failed turns due to rate limit / usage limit across candidate state databases
 */
export function findRecentFailedTurns(
  sinceSeconds: number = 7200,
  codexHome?: string
): TurnFailureRecord[] {
  const dbPaths = findStateDatabasePaths(codexHome);
  const cutoff = Math.floor(Date.now() / 1000) - sinceSeconds;
  const results: TurnFailureRecord[] = [];

  for (const dbPath of dbPaths) {
    if (!existsSync(dbPath)) continue;
    try {
      const db = new DatabaseSync(dbPath, { open: true, readOnly: true });
      try {
        db.exec('PRAGMA busy_timeout = 5000;');
        const stmt = db.prepare(`
          SELECT t.thread_id, t.turn_id, t.rollout_ordinal, t.status, t.error_json, t.started_at, t.completed_at
          FROM thread_turns t
          WHERE t.status = 'failed' AND t.completed_at >= ?
          ORDER BY t.completed_at DESC
          LIMIT 20
        `);
        const rows = stmt.all(cutoff) as Array<{
          thread_id: string;
          turn_id: string;
          rollout_ordinal: number;
          status: string;
          error_json: string | null;
          started_at: number;
          completed_at: number;
        }>;

        for (const row of rows) {
          results.push({
            threadId: row.thread_id,
            turnId: row.turn_id,
            rolloutOrdinal: row.rollout_ordinal,
            status: row.status,
            errorJson: row.error_json,
            startedAt: row.started_at,
            completedAt: row.completed_at,
          });
        }
        if (results.length > 0) break; // Found in newest generation database
      } finally {
        db.close();
      }
    } catch {
      // Continue searching older generations
    }
  }

  return results;
}

/**
 * Reads thread metadata (title, model, etc.)
 */
export function getThreadInfo(threadId: string, codexHome?: string): ThreadInfo | null {
  const dbPaths = findStateDatabasePaths(codexHome);

  for (const dbPath of dbPaths) {
    if (!existsSync(dbPath)) continue;
    try {
      const db = new DatabaseSync(dbPath, { open: true, readOnly: true });
      try {
        db.exec('PRAGMA busy_timeout = 5000;');
        const stmt = db.prepare(`
          SELECT id, rollout_path, title, name, cwd, model, updated_at_ms, created_at_ms, archived
          FROM threads
          WHERE id = ?
          LIMIT 1
        `);
        const row = stmt.get(threadId) as
          | {
              id: string;
              rollout_path: string | null;
              title: string | null;
              name: string | null;
              cwd: string | null;
              model: string | null;
              updated_at_ms: number | null;
              created_at_ms: number | null;
              archived: number | null;
            }
          | undefined;

        if (row) {
          return {
            id: row.id,
            rolloutPath: row.rollout_path,
            title: row.title,
            name: row.name,
            cwd: row.cwd,
            model: row.model,
            updatedAtMs: row.updated_at_ms || 0,
            createdAtMs: row.created_at_ms || 0,
            archived: Boolean(row.archived),
          };
        }
      } finally {
        db.close();
      }
    } catch {
      // Continue
    }
  }

  return null;
}
