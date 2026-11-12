import type { RawAccountQuotaSnapshot, ClassifiedQuota } from '../types/quota';

const FIVE_HOUR_MS = 300 * 60 * 1000;
const WEEKLY_MS = 10080 * 60 * 1000;

/**
 * Normalizes Unix timestamps that may arrive in either seconds (10-digit) or milliseconds (13-digit).
 */
function normalizeTimestampMs(ts?: number): number | undefined {
  if (typeof ts !== 'number' || !Number.isFinite(ts) || ts <= 0) {
    return undefined;
  }
  // Values under 10^10 are Unix timestamps in seconds (up to year 2286)
  return ts < 1e10 ? Math.round(ts * 1000) : Math.round(ts);
}

/**
 * Ensures usedPercent is always a finite number between 0 and infinity before arithmetic operations.
 */
function sanitizeUsedPercent(val?: number): number {
  if (typeof val !== 'number' || !Number.isFinite(val)) {
    return 0;
  }
  return val;
}

/**
 * Accurately classifies quota windows for OpenAI Codex accounts.
 * Fixes the critical bug where accounts with only a 7-day weekly limit
 * (e.g. primary.windowMinutes = 10080, secondary = 0) were falsely forced
 * into nonexistent 5-hour quota bars, and resets remaining headroom to 100%
 * once an expired rate-limit window passes.
 */
export function classifyQuota(
  entry?: RawAccountQuotaSnapshot | null,
  now = Date.now()
): ClassifiedQuota {
  if (!entry) {
    return {
      has5HourWindow: true,
      hasWeeklyWindow: false,
      isWeeklyOnly: false,
      fiveHourRemainingPercent: 100,
      weeklyRemainingPercent: undefined,
      isExhausted: false,
      isAvailable: true,
    };
  }

  // Normalize: allow either primary/secondary or fiveHour/weekly format
  let primary =
    entry.primary ||
    (entry.fiveHour
      ? { ...entry.fiveHour, windowMinutes: entry.fiveHour.windowMinutes || 300 }
      : undefined);
  let secondary =
    entry.secondary ||
    (entry.weekly
      ? { ...entry.weekly, windowMinutes: entry.weekly.windowMinutes || 10080 }
      : undefined);

  // Edge case: snapshot only populated `secondary` with a weekly window and omitted `primary`
  if (!primary && secondary && (secondary.windowMinutes ?? 0) >= 10080) {
    primary = secondary;
    secondary = undefined;
  }

  const updatedAtMs = normalizeTimestampMs(entry.updatedAt);

  // Case 1: Account has ONLY a weekly (7-day, 10080 minutes) quota window
  const isPrimaryWeekly = (primary?.windowMinutes ?? 0) >= 10080;
  const hasSecondary = Boolean(secondary && (secondary.windowMinutes ?? 0) > 0);

  if (isPrimaryWeekly && !hasSecondary) {
    const rawWeeklyUsed = sanitizeUsedPercent(primary?.usedPercent);
    const weeklyReset = normalizeTimestampMs(primary?.resetAtMs ?? primary?.resetAt);
    const isResetPassed = weeklyReset !== undefined && weeklyReset <= now;

    // Fallback expiration if status=429 or 100% arrived without an explicit reset timestamp
    const isFallbackActive =
      weeklyReset === undefined
        ? updatedAtMs !== undefined
          ? now - updatedAtMs < WEEKLY_MS
          : true
        : weeklyReset > now;

    const isExhausted = (entry.status === 429 || rawWeeklyUsed >= 100) && isFallbackActive;
    const effectiveWeeklyUsed = isResetPassed ? 0 : rawWeeklyUsed;

    return {
      has5HourWindow: false,
      hasWeeklyWindow: true,
      isWeeklyOnly: true,
      fiveHourRemainingPercent: undefined,
      fiveHourResetAtMs: undefined,
      weeklyRemainingPercent: Math.min(100, Math.max(0, 100 - Math.round(effectiveWeeklyUsed))),
      weeklyResetAtMs: weeklyReset,
      isExhausted,
      isAvailable: !isExhausted,
    };
  }

  // Case 2: Standard Dual-Window account (5h rolling primary + 7d weekly secondary)
  const rawFiveHourUsed = sanitizeUsedPercent(primary?.usedPercent);
  const fiveHourReset = normalizeTimestampMs(primary?.resetAtMs ?? primary?.resetAt);
  const is5hResetPassed = fiveHourReset !== undefined && fiveHourReset <= now;

  const rawWeeklyUsed =
    hasSecondary && secondary?.usedPercent !== undefined
      ? sanitizeUsedPercent(secondary.usedPercent)
      : undefined;
  const weeklyReset = hasSecondary
    ? normalizeTimestampMs(secondary?.resetAtMs ?? secondary?.resetAt)
    : undefined;
  const isWeeklyResetPassed = weeklyReset !== undefined && weeklyReset <= now;

  // Only attribute top-level status=429 to the 5h window if the weekly window isn't the one at 100%
  const status429AppliesTo5h =
    entry.status === 429 && (rawWeeklyUsed === undefined || rawWeeklyUsed < 100);

  const is5hWindowStillLocked =
    fiveHourReset !== undefined
      ? fiveHourReset > now
      : updatedAtMs !== undefined
      ? now - updatedAtMs < FIVE_HOUR_MS
      : true;

  const is5hExhausted = (status429AppliesTo5h || rawFiveHourUsed >= 100) && is5hWindowStillLocked;

  const isWeeklyWindowStillLocked =
    weeklyReset !== undefined
      ? weeklyReset > now
      : updatedAtMs !== undefined
      ? now - updatedAtMs < WEEKLY_MS
      : true;

  const isWeeklyExhausted =
    rawWeeklyUsed !== undefined && rawWeeklyUsed >= 100 && isWeeklyWindowStillLocked;

  const isExhausted = is5hExhausted || isWeeklyExhausted;

  const effectiveFiveHourUsed = is5hResetPassed ? 0 : rawFiveHourUsed;
  const effectiveWeeklyUsed =
    rawWeeklyUsed !== undefined ? (isWeeklyResetPassed ? 0 : rawWeeklyUsed) : undefined;

  return {
    has5HourWindow: true,
    hasWeeklyWindow: hasSecondary,
    isWeeklyOnly: false,
    fiveHourRemainingPercent: Math.min(100, Math.max(0, 100 - Math.round(effectiveFiveHourUsed))),
    fiveHourResetAtMs: fiveHourReset,
    weeklyRemainingPercent:
      effectiveWeeklyUsed !== undefined
        ? Math.min(100, Math.max(0, 100 - Math.round(effectiveWeeklyUsed)))
        : undefined,
    weeklyResetAtMs: weeklyReset,
    isExhausted,
    isAvailable: !isExhausted,
  };
}
