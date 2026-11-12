import { test, describe, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { DatabaseSync } from 'node:sqlite';
import {
  findStateDatabasePaths,
  findRecentFailedTurns,
  getThreadInfo,
} from '../core/watcher/sqlite-watcher';
import {
  findGoalDatabasePaths,
  listWaitingGoals,
  reactivateGoal,
} from '../core/watcher/goal-manager';

describe('Codex Tool - SQLite Watcher & Goal Manager', () => {
  let tempCodexHome: string;
  let originalEnv: string | undefined;

  beforeEach(() => {
    tempCodexHome = fs.mkdtempSync(path.join(os.tmpdir(), 'codex-watcher-test-'));
    originalEnv = process.env.CODEX_HOME;
    process.env.CODEX_HOME = tempCodexHome;
  });

  afterEach(() => {
    process.env.CODEX_HOME = originalEnv;
    try {
      fs.rmSync(tempCodexHome, { recursive: true, force: true });
    } catch {}
  });

  test('Finds candidate state and goal databases sorted descending by generation', () => {
    fs.writeFileSync(path.join(tempCodexHome, 'state_1.sqlite'), '');
    fs.writeFileSync(path.join(tempCodexHome, 'state_5.sqlite'), '');
    fs.writeFileSync(path.join(tempCodexHome, 'goals_2.sqlite'), '');
    fs.writeFileSync(path.join(tempCodexHome, 'goals_9.sqlite'), '');
    fs.writeFileSync(path.join(tempCodexHome, 'random.db'), '');

    const statePaths = findStateDatabasePaths(tempCodexHome);
    assert.strictEqual(statePaths.length, 2);
    assert.match(statePaths[0], /state_5\.sqlite$/, 'State db must be sorted descending');
    assert.match(statePaths[1], /state_1\.sqlite$/);

    const goalPaths = findGoalDatabasePaths(tempCodexHome);
    assert.strictEqual(goalPaths.length, 4); // 2 goals + 2 state fallback
    assert.match(goalPaths[0], /goals_9\.sqlite$/, 'Goals db must be sorted descending');
  });

  test('Reads thread info and failed turns from state database', () => {
    const dbPath = path.join(tempCodexHome, 'state_5.sqlite');
    const db = new DatabaseSync(dbPath);

    db.exec(`
      CREATE TABLE threads (
        id TEXT PRIMARY KEY,
        rollout_path TEXT,
        title TEXT,
        name TEXT,
        cwd TEXT,
        model TEXT,
        updated_at_ms INTEGER,
        created_at_ms INTEGER,
        archived INTEGER
      );
      CREATE TABLE thread_turns (
        thread_id TEXT,
        turn_id TEXT,
        rollout_ordinal INTEGER,
        status TEXT,
        error_json TEXT,
        started_at INTEGER,
        completed_at INTEGER
      );
    `);

    const now = Math.floor(Date.now() / 1000);
    db.exec(`
      INSERT INTO threads (id, title, model, updated_at_ms, created_at_ms, archived)
      VALUES ('th_abc123', 'Optimize SQL Indexes', 'gpt-5-codex', 1700000000, 1699990000, 0);

      INSERT INTO thread_turns (thread_id, turn_id, rollout_ordinal, status, error_json, started_at, completed_at)
      VALUES ('th_abc123', 'turn_1', 1, 'failed', '{"error":"usage_limit_reached"}', ${now - 60}, ${now - 10});
    `);
    db.close();

    // Query thread info
    const thread = getThreadInfo('th_abc123', tempCodexHome);
    assert.notStrictEqual(thread, null);
    assert.strictEqual(thread?.title, 'Optimize SQL Indexes');
    assert.strictEqual(thread?.model, 'gpt-5-codex');

    // Query failed turns
    const failedTurns = findRecentFailedTurns(3600, tempCodexHome);
    assert.strictEqual(failedTurns.length, 1);
    assert.strictEqual(failedTurns[0].threadId, 'th_abc123');
    assert.strictEqual(failedTurns[0].status, 'failed');
  });

  test('Lists waiting thread goals and reactivates them with atomic update', () => {
    const dbPath = path.join(tempCodexHome, 'goals_1.sqlite');
    const db = new DatabaseSync(dbPath);

    db.exec(`
      CREATE TABLE thread_goals (
        thread_id TEXT PRIMARY KEY,
        goal_id TEXT NOT NULL,
        objective TEXT NOT NULL,
        status TEXT NOT NULL,
        token_budget INTEGER,
        tokens_used INTEGER,
        time_used_seconds INTEGER,
        updated_at_ms INTEGER
      );
      CREATE TABLE thread_goal_continuation_deferrals (
        thread_id TEXT PRIMARY KEY
      );
    `);

    db.exec(`
      INSERT INTO thread_goals (thread_id, goal_id, objective, status, tokens_used, updated_at_ms)
      VALUES ('th_goal_999', 'g_999', 'Araştır: Ebedi kapıda ilerlemiyor', 'usage_limited', 12500, 1700000000);

      INSERT INTO thread_goal_continuation_deferrals (thread_id)
      VALUES ('th_goal_999');
    `);
    db.close();

    // 1. List waiting goals
    const waiting = listWaitingGoals(tempCodexHome);
    assert.strictEqual(waiting.length, 1);
    assert.strictEqual(waiting[0].threadId, 'th_goal_999');
    assert.strictEqual(waiting[0].status, 'usage_limited');
    assert.strictEqual(waiting[0].title, 'Araştır: Ebedi kapıda ilerlemiyor');

    // 2. Reactivate goal
    const reactivateRes = reactivateGoal('th_goal_999', tempCodexHome);
    assert.strictEqual(reactivateRes.success, true);

    // Verify database row updated to 'active' and deferral removed
    const verifyDb = new DatabaseSync(dbPath, { readOnly: true });
    const stmt = verifyDb.prepare('SELECT status FROM thread_goals WHERE thread_id = ?');
    const row = stmt.get('th_goal_999') as { status: string };
    assert.strictEqual(row.status, 'active');

    const deferralStmt = verifyDb.prepare('SELECT COUNT(*) as count FROM thread_goal_continuation_deferrals WHERE thread_id = ?');
    const deferralRow = deferralStmt.get('th_goal_999') as { count: number };
    assert.strictEqual(deferralRow.count, 0, 'Deferral row must be cleared');
    verifyDb.close();
  });
});
