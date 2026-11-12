import { test, describe } from 'node:test';
import assert from 'node:assert';
import { classifyQuota } from '../core/quota/window-classifier';
import {
  classifyAccountTiers,
  selectBestAccount,
  checkPreemption,
} from '../core/accounts/tier-scheduler';
import type { ManagedAccount } from '../core/types/account';

describe('Codex Tool - Core Quota & Tier Scheduler', () => {
  test('Classifies 7-day weekly-only accounts accurately (no false 5h window)', () => {
    // A weekly-only account can report its quota as the primary window.
    const weeklyAccountQuota = {
      updatedAt: Date.now(),
      status: 200,
      primary: {
        windowMinutes: 10080, // 7 days in minutes
        usedPercent: 16,
        resetAtMs: Date.now() + 500000000,
      },
      secondary: {
        windowMinutes: 0,
        usedPercent: 0,
      },
    };

    const classified = classifyQuota(weeklyAccountQuota);
    assert.strictEqual(classified.has5HourWindow, false, 'Weekly account must NOT report a 5h window');
    assert.strictEqual(classified.isWeeklyOnly, true, 'Account must be flagged as weekly-only');
    assert.strictEqual(classified.weeklyRemainingPercent, 84, 'Weekly remaining must be 84%');
    assert.strictEqual(classified.fiveHourRemainingPercent, undefined, '5h remaining must be undefined');
  });

  test('Classifies standard dual-window accounts (5h rolling + weekly secondary)', () => {
    const dualAccountQuota = {
      updatedAt: Date.now(),
      status: 200,
      primary: {
        windowMinutes: 300, // 5 hours in minutes
        usedPercent: 25,
        resetAtMs: Date.now() + 10000000,
      },
      secondary: {
        windowMinutes: 10080,
        usedPercent: 10,
        resetAtMs: Date.now() + 500000000,
      },
    };

    const classified = classifyQuota(dualAccountQuota);
    assert.strictEqual(classified.has5HourWindow, true, 'Dual account must have a 5h window');
    assert.strictEqual(classified.isWeeklyOnly, false, 'Dual account is NOT weekly only');
    assert.strictEqual(classified.fiveHourRemainingPercent, 75);
    assert.strictEqual(classified.weeklyRemainingPercent, 90);
  });

  test('Tier Priority: Prefers 5h limit accounts over weekly-only reserve accounts', () => {
    const accounts: ManagedAccount[] = [
      {
        index: 0,
        email: 'weekly-reserve@example.com',
        quota: {
          updatedAt: Date.now(),
          primary: { windowMinutes: 10080, usedPercent: 5 }, // 95% available
          secondary: { windowMinutes: 0, usedPercent: 0 },
        },
      },
      {
        index: 1,
        email: 'standard-5h@example.com',
        quota: {
          updatedAt: Date.now(),
          primary: { windowMinutes: 300, usedPercent: 40 }, // 60% available
          secondary: { windowMinutes: 10080, usedPercent: 15 },
        },
      },
    ];

    const { tier1Accounts, tier2Accounts } = classifyAccountTiers(accounts);
    assert.strictEqual(tier1Accounts.length, 1);
    assert.strictEqual(tier1Accounts[0].email, 'standard-5h@example.com');
    assert.strictEqual(tier2Accounts.length, 1);
    assert.strictEqual(tier2Accounts[0].email, 'weekly-reserve@example.com');

    const selection = selectBestAccount(accounts);
    assert.strictEqual(selection.tier, 1, 'Should choose Tier 1 account first');
    assert.strictEqual(selection.account?.email, 'standard-5h@example.com');
  });

  test('Tier Fallback: Uses weekly-only account when all 5h accounts are exhausted', () => {
    const now = Date.now();
    const accounts: ManagedAccount[] = [
      {
        index: 0,
        email: 'weekly-reserve@example.com',
        quota: {
          updatedAt: now,
          primary: { windowMinutes: 10080, usedPercent: 20 },
          secondary: { windowMinutes: 0, usedPercent: 0 },
        },
      },
      {
        index: 1,
        email: 'standard-5h-exhausted@example.com',
        quota: {
          updatedAt: now,
          fiveHour: { windowMinutes: 300, usedPercent: 100, resetAt: now + 3600000 },
        },
      },
    ];

    const selection = selectBestAccount(accounts, undefined, now);
    assert.strictEqual(selection.tier, 2, 'Must fallback to Tier 2 reserve');
    assert.strictEqual(selection.account?.email, 'weekly-reserve@example.com');
  });

  test('Preemption: Switches back from weekly-only reserve to 5h account when 5h resets', () => {
    const now = Date.now();
    const accounts: ManagedAccount[] = [
      {
        index: 0,
        email: 'weekly-reserve@example.com',
        quota: {
          updatedAt: now,
          primary: { windowMinutes: 10080, usedPercent: 30 },
          secondary: { windowMinutes: 0, usedPercent: 0 },
        },
      },
      {
        index: 1,
        email: 'standard-5h-reset@example.com',
        quota: {
          updatedAt: now,
          primary: { windowMinutes: 300, usedPercent: 0, resetAtMs: now - 1000 }, // Reset just arrived!
          secondary: { windowMinutes: 10080, usedPercent: 20 },
        },
      },
    ];

    // Currently on the weekly-reserve account
    const preemption = checkPreemption('weekly-reserve@example.com', accounts, now);
    assert.strictEqual(preemption.shouldPreempt, true, 'Must preempt the weekly account');
    assert.strictEqual(preemption.preemptWith?.email, 'standard-5h-reset@example.com');
  });
});
