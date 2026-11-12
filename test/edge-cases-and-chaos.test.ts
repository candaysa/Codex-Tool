import { test, describe } from 'node:test';
import assert from 'node:assert';
import { classifyQuota } from '../core/quota/window-classifier';
import {
  classifyAccountTiers,
  isAccountAvailable,
  selectBestAccount,
  checkPreemption,
} from '../core/accounts/tier-scheduler';
import type { ManagedAccount } from '../core/types/account';

describe('Codex Tool - Deep Edge Cases & Chaos Tests', () => {
  describe('classifyQuota Edge Cases', () => {
    test('Handles undefined or null quota gracefully', () => {
      const resNull = classifyQuota(null);
      assert.strictEqual(resNull.has5HourWindow, true);
      assert.strictEqual(resNull.isWeeklyOnly, false);
      assert.strictEqual(resNull.isAvailable, true);

      const resUndefined = classifyQuota(undefined);
      assert.strictEqual(resUndefined.has5HourWindow, true);
      assert.strictEqual(resUndefined.isAvailable, true);
    });

    test('Expired 429 rate limit is recognized as AVAILABLE and resets remaining headroom to 100%', () => {
      const now = 1_700_000_000_000;
      const expired429 = {
        updatedAt: now - 3600000,
        status: 429,
        primary: {
          windowMinutes: 300,
          usedPercent: 100,
          resetAtMs: now - 1000, // Reset was 1 second ago!
        },
      };

      const res = classifyQuota(expired429, now);
      assert.strictEqual(res.isExhausted, false, 'Expired reset must NOT remain exhausted');
      assert.strictEqual(res.isAvailable, true);
      assert.strictEqual(res.fiveHourRemainingPercent, 100, 'Expired window must restore 100% headroom');
    });

    test('Normalizes 10-digit Unix seconds resetAt into milliseconds accurately', () => {
      const nowMs = 1_760_000_000_000;
      const resetInSeconds = Math.floor((nowMs + 120_000) / 1000); // 2 minutes in the future (in seconds)
      const snap = {
        updatedAt: nowMs,
        status: 429,
        primary: {
          windowMinutes: 300,
          usedPercent: 100,
          resetAt: resetInSeconds,
        },
      };

      const res = classifyQuota(snap, nowMs);
      assert.strictEqual(res.isExhausted, true, '10-digit future seconds timestamp must be recognized as future');
      assert.strictEqual(res.fiveHourResetAtMs, resetInSeconds * 1000);
    });

    test('Handles NaN usedPercent without propagating NaN', () => {
      const broken = {
        updatedAt: Date.now(),
        primary: { windowMinutes: 300, usedPercent: Number.NaN },
      };
      const res = classifyQuota(broken);
      assert.strictEqual(Number.isNaN(res.fiveHourRemainingPercent), false);
      assert.strictEqual(res.fiveHourRemainingPercent, 100);
    });

    test('Weekly-only account with future reset and normal usage (16%) is AVAILABLE, not exhausted', () => {
      const now = 1_700_000_000_000;
      const weeklyHealthy = {
        updatedAt: now,
        status: 200,
        primary: {
          windowMinutes: 10080,
          usedPercent: 16,
          resetAtMs: now + 500000000,
        },
        secondary: {
          windowMinutes: 0,
          usedPercent: 0,
        },
      };

      const res = classifyQuota(weeklyHealthy, now);
      assert.strictEqual(res.isWeeklyOnly, true);
      assert.strictEqual(res.has5HourWindow, false);
      assert.strictEqual(res.isExhausted, false);
      assert.strictEqual(res.isAvailable, true);
      assert.strictEqual(res.weeklyRemainingPercent, 84);
    });

    test('Clamps remaining percentage bounds between 0 and 100', () => {
      const overused = {
        updatedAt: Date.now(),
        primary: { windowMinutes: 300, usedPercent: 145 },
      };
      const res = classifyQuota(overused);
      assert.strictEqual(res.fiveHourRemainingPercent, 0, 'Cannot drop below 0');

      const negativeUsed = {
        updatedAt: Date.now(),
        primary: { windowMinutes: 300, usedPercent: -10 },
      };
      const res2 = classifyQuota(negativeUsed);
      assert.strictEqual(res2.fiveHourRemainingPercent, 100, 'Cannot exceed 100');
    });

    test('Normalizes both primary/secondary and fiveHour/weekly object formats', () => {
      const legacyFormat = {
        updatedAt: Date.now(),
        fiveHour: { windowMinutes: 300, usedPercent: 20 },
        weekly: { windowMinutes: 10080, usedPercent: 10 },
      };

      const res = classifyQuota(legacyFormat);
      assert.strictEqual(res.has5HourWindow, true);
      assert.strictEqual(res.fiveHourRemainingPercent, 80);
      assert.strictEqual(res.weeklyRemainingPercent, 90);
    });
  });

  describe('tier-scheduler Edge Cases & Resilience', () => {
    test('Filters out disabled, flagged, and suspended accounts from tier classification', () => {
      const accounts: ManagedAccount[] = [
        { index: 0, email: 'flagged@test.com', isFlagged: true },
        { index: 1, email: 'suspended@test.com', isSuspended: true },
        { index: 2, email: 'disabled@test.com', enabled: false },
        { index: 3, email: 'valid@test.com', enabled: true },
      ];

      const { tier1Accounts, tier2Accounts } = classifyAccountTiers(accounts);
      assert.strictEqual(tier1Accounts.length, 1);
      assert.strictEqual(tier1Accounts[0].email, 'valid@test.com');
      assert.strictEqual(tier2Accounts.length, 0);
    });

    test('Respects cooldownUntil timestamp', () => {
      const now = 2000000;
      const coolingAccount: ManagedAccount = {
        index: 0,
        email: 'cooling@test.com',
        cooldownUntil: now + 60000,
      };

      assert.strictEqual(isAccountAvailable(coolingAccount, now), false);
      assert.strictEqual(isAccountAvailable(coolingAccount, now + 60001), true);
    });

    test('Tie-breaking: selects least recently used when headroom is identical', () => {
      const now = Date.now();
      const accounts: ManagedAccount[] = [
        {
          index: 0,
          email: 'recent@test.com',
          lastUsedAt: now - 1000,
          quota: { updatedAt: now, primary: { windowMinutes: 300, usedPercent: 50 } },
        },
        {
          index: 1,
          email: 'oldest@test.com',
          lastUsedAt: now - 3600000,
          quota: { updatedAt: now, primary: { windowMinutes: 300, usedPercent: 50 } },
        },
      ];

      const selection = selectBestAccount(accounts, undefined, now);
      assert.strictEqual(selection.account?.email, 'oldest@test.com');
    });

    test('Returns nextResetWaitMs when all accounts are exhausted', () => {
      const now = 1_700_000_000_000;
      const accounts: ManagedAccount[] = [
        {
          index: 0,
          email: 'exhausted-1@test.com',
          quota: {
            updatedAt: now,
            primary: { windowMinutes: 300, usedPercent: 100, resetAtMs: now + 30000 },
          },
        },
        {
          index: 1,
          email: 'exhausted-2@test.com',
          quota: {
            updatedAt: now,
            primary: { windowMinutes: 300, usedPercent: 100, resetAtMs: now + 15000 },
          },
        },
      ];

      const selection = selectBestAccount(accounts, undefined, now);
      assert.strictEqual(selection.account, null);
      assert.strictEqual(selection.nextResetWaitMs, 15000);
    });

    test('Preemption selects the highest-headroom Tier 1 account instead of first array element', () => {
      const now = 1_700_000_000_000;
      const accounts: ManagedAccount[] = [
        {
          index: 0,
          email: 'weekly-reserve@test.com',
          quota: {
            updatedAt: now,
            primary: { windowMinutes: 10080, usedPercent: 20 },
          },
        },
        {
          index: 1,
          email: 'tier1-low-headroom@test.com',
          quota: {
            updatedAt: now,
            primary: { windowMinutes: 300, usedPercent: 95, resetAtMs: now + 60000 },
          },
        },
        {
          index: 2,
          email: 'tier1-fresh@test.com',
          quota: {
            updatedAt: now,
            primary: { windowMinutes: 300, usedPercent: 5, resetAtMs: now + 60000 },
          },
        },
      ];

      const preemption = checkPreemption('weekly-reserve@test.com', accounts, now);
      assert.strictEqual(preemption.shouldPreempt, true);
      assert.strictEqual(
        preemption.preemptWith?.email,
        'tier1-fresh@test.com',
        'Must pick Tier 1 account with 95% headroom over 5% headroom'
      );
    });

    test('Tier 1 effective headroom considers secondary weekly bottleneck', () => {
      const now = 1_700_000_000_000;
      const accounts: ManagedAccount[] = [
        {
          index: 0,
          email: 'tier1-weekly-almost-empty@test.com',
          quota: {
            updatedAt: now,
            primary: { windowMinutes: 300, usedPercent: 10 }, // 90% 5h left
            secondary: { windowMinutes: 10080, usedPercent: 98 }, // only 2% weekly left!
          },
        },
        {
          index: 1,
          email: 'tier1-balanced@test.com',
          quota: {
            updatedAt: now,
            primary: { windowMinutes: 300, usedPercent: 25 }, // 75% 5h left
            secondary: { windowMinutes: 10080, usedPercent: 20 }, // 80% weekly left
          },
        },
      ];

      const best = selectBestAccount(accounts, undefined, now);
      assert.strictEqual(best.account?.email, 'tier1-balanced@test.com');
    });
  });
});
