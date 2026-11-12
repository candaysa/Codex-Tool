export interface QuotaWindow {
  usedPercent: number;
  windowMinutes: number;
  resetAtMs?: number;
  resetAt?: number;
}

export interface RawAccountQuotaSnapshot {
  updatedAt: number;
  status?: number;
  model?: string;
  planType?: string;
  primary?: QuotaWindow;
  secondary?: QuotaWindow;
  fiveHour?: QuotaWindow;
  weekly?: QuotaWindow;
}

export type AccountQuotaSnapshot = RawAccountQuotaSnapshot;

export interface ClassifiedQuota {
  /** True if the account has a 5-hour rolling quota window */
  has5HourWindow: boolean;
  /** True if the account has a weekly quota window */
  hasWeeklyWindow: boolean;
  /** True if the account ONLY has a 7-day quota (no 5h rolling window) */
  isWeeklyOnly: boolean;
  /** Five-hour window remaining percentage (0-100) */
  fiveHourRemainingPercent?: number;
  /** Five-hour window reset timestamp in ms */
  fiveHourResetAtMs?: number;
  /** Weekly window remaining percentage (0-100) */
  weeklyRemainingPercent?: number;
  /** Weekly window reset timestamp in ms */
  weeklyResetAtMs?: number;
  /** True if quota is fully exhausted / 429 */
  isExhausted: boolean;
  /** True if ready for inference right now */
  isAvailable: boolean;
}
