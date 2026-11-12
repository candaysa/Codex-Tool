import type { ManagedAccount } from '../types/account';
import { classifyQuota } from '../quota/window-classifier';

export interface TierClassification {
  tier1Accounts: ManagedAccount[]; // 5-hour window accounts (Primary priority)
  tier2Accounts: ManagedAccount[]; // 7-day weekly-only accounts (Reserve priority)
}

export interface SelectionResult {
  account: ManagedAccount | null;
  reason: string;
  tier: 1 | 2 | null;
  nextResetWaitMs?: number;
}

/**
 * Classifies accounts into:
 * Tier 1: Accounts with standard 5-hour rate limits (should be burned first and cycled)
 * Tier 2: Accounts with weekly-only rate limits (precious reserve, only touched when Tier 1 is fully exhausted)
 */
export function classifyAccountTiers(accounts: ManagedAccount[], now = Date.now()): TierClassification {
  const tier1Accounts: ManagedAccount[] = [];
  const tier2Accounts: ManagedAccount[] = [];

  for (const acc of accounts) {
    if (acc.isFlagged || acc.isSuspended) continue;
    if (acc.enabled === false) continue;

    const quotaType = classifyQuota(acc.quota, now);
    if (quotaType.isWeeklyOnly) {
      tier2Accounts.push(acc);
    } else {
      tier1Accounts.push(acc);
    }
  }

  return { tier1Accounts, tier2Accounts };
}

/**
 * Determines if an account is currently usable (has remaining quota & not in cooldown)
 */
export function isAccountAvailable(acc: ManagedAccount, now: number = Date.now()): boolean {
  if (acc.isFlagged || acc.isSuspended) return false;
  if (acc.enabled === false) return false;
  if (acc.cooldownUntil && acc.cooldownUntil > now) return false;

  const quota = acc.quota;
  if (!quota) return true; // optimistic if no quota probe yet

  const quotaType = classifyQuota(quota, now);
  return quotaType.isAvailable && !quotaType.isExhausted;
}

/**
 * Returns the effective remaining headroom for a Tier 1 account, constrained by
 * both its 5-hour rolling window and its secondary weekly window if present.
 */
function getTier1RemainingHeadroom(acc: ManagedAccount, now: number): number {
  if (!acc.quota) return 100;
  const classified = classifyQuota(acc.quota, now);
  const fiveHour = classified.fiveHourRemainingPercent ?? 100;
  const weekly = classified.weeklyRemainingPercent ?? 100;
  return Math.min(fiveHour, weekly);
}

function getTier2RemainingHeadroom(acc: ManagedAccount, now: number): number {
  if (!acc.quota) return 100;
  const classified = classifyQuota(acc.quota, now);
  return classified.weeklyRemainingPercent ?? 100;
}

/**
 * Calculates the earliest timestamp when any currently unavailable (but enabled/unflagged)
 * account will become usable again, accounting for both cooldowns and bottleneck quota windows.
 */
function getEarliestResetTime(accounts: ManagedAccount[], now: number): number | undefined {
  let earliest: number | undefined;

  for (const acc of accounts) {
    if (acc.isFlagged || acc.isSuspended || acc.enabled === false) continue;

    let accountUnlockAt: number | undefined;

    if (acc.cooldownUntil && acc.cooldownUntil > now) {
      accountUnlockAt = acc.cooldownUntil;
    }

    if (acc.quota) {
      const classified = classifyQuota(acc.quota, now);
      if (classified.isExhausted) {
        const fiveHourExhausted =
          (classified.fiveHourRemainingPercent ?? 100) <= 0 &&
          classified.fiveHourResetAtMs !== undefined &&
          classified.fiveHourResetAtMs > now;
        const weeklyExhausted =
          (classified.weeklyRemainingPercent ?? 100) <= 0 &&
          classified.weeklyResetAtMs !== undefined &&
          classified.weeklyResetAtMs > now;

        let quotaResetAt: number | undefined;
        if (fiveHourExhausted && weeklyExhausted) {
          quotaResetAt = Math.max(classified.fiveHourResetAtMs!, classified.weeklyResetAtMs!);
        } else if (fiveHourExhausted) {
          quotaResetAt = classified.fiveHourResetAtMs;
        } else if (weeklyExhausted) {
          quotaResetAt = classified.weeklyResetAtMs;
        } else {
          quotaResetAt = classified.fiveHourResetAtMs ?? classified.weeklyResetAtMs;
        }

        if (quotaResetAt && quotaResetAt > now) {
          accountUnlockAt =
            accountUnlockAt !== undefined
              ? Math.max(accountUnlockAt, quotaResetAt)
              : quotaResetAt;
        }
      }
    }

    if (accountUnlockAt && accountUnlockAt > now) {
      if (earliest === undefined || accountUnlockAt < earliest) {
        earliest = accountUnlockAt;
      }
    }
  }

  return earliest;
}

/**
 * Selects the optimal account to use based on strict tier priority:
 * 1. Always prefer Tier 1 (5-hour limit) accounts first.
 * 2. Only fall back to Tier 2 (7-day weekly-only) if ALL Tier 1 accounts are exhausted.
 * 3. Within a tier, select the account with the most remaining headroom (lowest usedPercent)
 *    and least recently used.
 */
export function selectBestAccount(
  accounts: ManagedAccount[],
  currentEmail?: string,
  now: number = Date.now()
): SelectionResult {
  const { tier1Accounts, tier2Accounts } = classifyAccountTiers(accounts, now);

  // Filter available accounts in Tier 1
  const availableTier1 = tier1Accounts.filter(acc => isAccountAvailable(acc, now));

  if (availableTier1.length > 0) {
    // Sort Tier 1: highest headroom first, then least recently used
    availableTier1.sort((a, b) => {
      const aHeadroom = getTier1RemainingHeadroom(a, now);
      const bHeadroom = getTier1RemainingHeadroom(b, now);
      if (aHeadroom !== bHeadroom) return bHeadroom - aHeadroom;
      return (a.lastUsedAt ?? 0) - (b.lastUsedAt ?? 0);
    });

    // Affinity preference if current account is still healthy (>20% headroom)
    if (currentEmail) {
      const current = availableTier1.find(a => a.email.toLowerCase() === currentEmail.toLowerCase());
      if (current && getTier1RemainingHeadroom(current, now) >= 20) {
        return {
          account: current,
          reason: `Current Tier 1 account (${current.email}) remains healthy with ${getTier1RemainingHeadroom(current, now)}% headroom`,
          tier: 1,
        };
      }
    }

    const chosen = availableTier1[0];
    const headroom = getTier1RemainingHeadroom(chosen, now);
    return {
      account: chosen,
      reason: `Selected Tier 1 account (${chosen.email}) with ${headroom}% headroom`,
      tier: 1,
    };
  }

  // Tier 1 is completely exhausted -> Fall back to Tier 2 (Reserve 7-day accounts)
  const availableTier2 = tier2Accounts.filter(acc => isAccountAvailable(acc, now));

  if (availableTier2.length > 0) {
    // Sort Tier 2: highest weekly headroom first, then least recently used
    availableTier2.sort((a, b) => {
      const aHeadroom = getTier2RemainingHeadroom(a, now);
      const bHeadroom = getTier2RemainingHeadroom(b, now);
      if (aHeadroom !== bHeadroom) return bHeadroom - aHeadroom;
      return (a.lastUsedAt ?? 0) - (b.lastUsedAt ?? 0);
    });

    const chosen = availableTier2[0];
    const headroom = getTier2RemainingHeadroom(chosen, now);
    return {
      account: chosen,
      reason: `All Tier 1 accounts exhausted. Activated Tier 2 reserve (${chosen.email}) with ${headroom}% weekly headroom`,
      tier: 2,
    };
  }

  const earliestReset = getEarliestResetTime(accounts, now);
  const nextResetWaitMs = earliestReset ? Math.max(0, earliestReset - now) : undefined;

  return {
    account: null,
    reason: 'All accounts (Tier 1 and Tier 2) are currently exhausted or cooling down',
    tier: null,
    nextResetWaitMs,
  };
}

/**
 * Checks if the currently active account should be preempted immediately.
 * If currently using a Tier 2 account, and ANY Tier 1 account has reset and is available,
 * returns true with the best available Tier 1 account (highest headroom & least recently used).
 */
export function checkPreemption(
  currentEmail: string | undefined,
  accounts: ManagedAccount[],
  now: number = Date.now()
): { shouldPreempt: boolean; preemptWith: ManagedAccount | null; reason?: string } {
  if (!currentEmail) {
    const best = selectBestAccount(accounts, undefined, now);
    return {
      shouldPreempt: best.account !== null,
      preemptWith: best.account,
      reason: 'No active account set',
    };
  }

  const current = accounts.find(a => a.email.toLowerCase() === currentEmail.toLowerCase());
  if (!current) {
    const best = selectBestAccount(accounts, undefined, now);
    return {
      shouldPreempt: true,
      preemptWith: best.account,
      reason: 'Current account no longer in pool',
    };
  }

  // If current account is exhausted or flagged, preempt immediately
  if (!isAccountAvailable(current, now)) {
    const nextBest = selectBestAccount(accounts, currentEmail, now);
    return {
      shouldPreempt: true,
      preemptWith: nextBest.account,
      reason: `Current account (${current.email}) is exhausted or cooling down`,
    };
  }

  const currentQuotaType = classifyQuota(current.quota, now);

  // If current account is Tier 2 (weekly only), check if any Tier 1 (5h) account is back online
  if (currentQuotaType.isWeeklyOnly) {
    const { tier1Accounts } = classifyAccountTiers(accounts, now);
    const bestTier1 = selectBestAccount(tier1Accounts, undefined, now);

    if (bestTier1.account && bestTier1.tier === 1) {
      return {
        shouldPreempt: true,
        preemptWith: bestTier1.account,
        reason: `Preempting weekly reserve: Tier 1 account (${bestTier1.account.email}) 5-hour quota has refreshed!`,
      };
    }
  }

  return { shouldPreempt: false, preemptWith: null };
}
