import { DatabaseSync } from 'node:sqlite';
import { readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { getDefaultCodexHome, getThreadInfo } from './sqlite-watcher';

export type GoalStatus =
  | 'active'
  | 'paused'
  | 'blocked'
  | 'usage_limited'
  | 'budget_limited'
  | 'complete';

export interface ThreadGoal {
  threadId: string;
  goalId: string;
  objective: string;
  status: GoalStatus;
  tokenBudget: number | null;
  tokensUsed: number;
  timeUsedSeconds: number;
  updatedAtMs: number;
  title?: string;
  autoResumeEnabled: boolean;
  timeUntilResetFormatted?: string;
}

const GOALS_DB_PATTERN = /^goals_(\d+)\.sqlite$/;
const STATE_DB_PATTERN = /^state_(\d+)\.sqlite$/;

export function findGoalDatabasePaths(codexHome?: string): string[] {
  const home = codexHome || getDefaultCodexHome();
  if (!existsSync(home)) return [];

  try {
    const entries = readdirSync(home);
    const goalDbs: Array<{ path: string; gen: number }> = [];
    const stateDbs: Array<{ path: string; gen: number }> = [];

    for (const entry of entries) {
      const gMatch = GOALS_DB_PATTERN.exec(entry);
      if (gMatch && gMatch[1]) {
        goalDbs.push({ path: join(home, entry), gen: Number.parseInt(gMatch[1], 10) });
      }
      const sMatch = STATE_DB_PATTERN.exec(entry);
      if (sMatch && sMatch[1]) {
        stateDbs.push({ path: join(home, entry), gen: Number.parseInt(sMatch[1], 10) });
      }
    }

    goalDbs.sort((a, b) => b.gen - a.gen);
    stateDbs.sort((a, b) => b.gen - a.gen);

    return [...goalDbs.map(d => d.path), ...stateDbs.map(d => d.path)];
  } catch {
    return [];
  }
}

function hasTable(db: DatabaseSync, tableName: string): boolean {
  try {
    const stmt = db.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?");
    return stmt.get(tableName) !== undefined;
  } catch {
    return false;
  }
}

/**
 * Lists all thread goals awaiting quota or paused
 */
export function listWaitingGoals(codexHome?: string): ThreadGoal[] {
  const dbPaths = findGoalDatabasePaths(codexHome);
  const seenThreads = new Set<string>();
  const results: ThreadGoal[] = [];

  for (const dbPath of dbPaths) {
    if (!existsSync(dbPath)) continue;
    try {
      const db = new DatabaseSync(dbPath, { open: true, readOnly: true });
      try {
        db.exec('PRAGMA busy_timeout = 5000;');
        if (!hasTable(db, 'thread_goals')) continue;

        const stmt = db.prepare(`
          SELECT thread_id, goal_id, objective, status, token_budget, tokens_used, time_used_seconds, updated_at_ms
          FROM thread_goals
          WHERE status IN ('usage_limited', 'blocked', 'paused')
          ORDER BY updated_at_ms DESC
        `);

        const rows = stmt.all() as Array<{
          thread_id: string;
          goal_id: string;
          objective: string;
          status: string;
          token_budget: number | null;
          tokens_used: number | null;
          time_used_seconds: number | null;
          updated_at_ms: number | null;
        }>;

        for (const r of rows) {
          if (seenThreads.has(r.thread_id)) continue;
          seenThreads.add(r.thread_id);

          const threadMeta = getThreadInfo(r.thread_id, codexHome);
          const displayTitle =
            threadMeta?.title || r.objective || `Thread ${r.thread_id.slice(0, 8)}`;

          results.push({
            threadId: r.thread_id,
            goalId: r.goal_id,
            objective: r.objective,
            status: r.status as GoalStatus,
            tokenBudget: r.token_budget,
            tokensUsed: r.tokens_used || 0,
            timeUsedSeconds: r.time_used_seconds || 0,
            updatedAtMs: r.updated_at_ms || Date.now(),
            title: displayTitle,
            autoResumeEnabled: true,
          });
        }
      } finally {
        db.close();
      }
    } catch {
      // Continue next db
    }
  }

  return results;
}

/**
 * Reactivates a paused/usage-limited goal by setting status to 'active'
 */
export function reactivateGoal(
  threadId: string,
  codexHome?: string
): { success: boolean; message: string } {
  const dbPaths = findGoalDatabasePaths(codexHome);
  let updatedAny = false;
  const now = Date.now();

  for (const dbPath of dbPaths) {
    if (!existsSync(dbPath)) continue;
    try {
      const db = new DatabaseSync(dbPath);
      try {
        db.exec('PRAGMA busy_timeout = 5000;');
        if (!hasTable(db, 'thread_goals')) continue;

        db.exec('BEGIN IMMEDIATE');
        const updateStmt = db.prepare(`
          UPDATE thread_goals
          SET status = 'active', updated_at_ms = ?
          WHERE thread_id = ?
        `);
        const res = updateStmt.run(now, threadId);

        if (hasTable(db, 'thread_goal_continuation_deferrals')) {
          const delStmt = db.prepare(`
            DELETE FROM thread_goal_continuation_deferrals
            WHERE thread_id = ?
          `);
          delStmt.run(threadId);
        }

        db.exec('COMMIT');
        if (res.changes > 0) {
          updatedAny = true;
        }
      } catch {
        try {
          db.exec('ROLLBACK');
        } catch {}
      } finally {
        db.close();
      }
    } catch {
      // Continue next db
    }
  }

  if (updatedAny) {
    return { success: true, message: `Reactivated goal for thread ${threadId}` };
  }
  return { success: false, message: `Could not find or update goal for thread ${threadId}` };
}
