import type { AccountQuotaSnapshot } from './quota';

export type PriorityTier = 1 | 2; // Tier 1: 5-hour rolling pool, Tier 2: Weekly reserve pool

export interface ManagedAccount {
  index: number;
  email: string;
  shortName?: string;
  accountId?: string;
  enabled?: boolean;
  isCurrent?: boolean;
  tier?: PriorityTier;
  quota?: AccountQuotaSnapshot;
  cooldownUntil?: number;
  lastUsedAt?: number;
  isFlagged?: boolean;
  isSuspended?: boolean;
  status?: 'READY' | '429 LIMIT' | 'EXHAUSTED' | 'DISABLED' | 'PROBING';
}

export interface StoredAccount extends ManagedAccount {
  status: 'READY' | '429 LIMIT' | 'EXHAUSTED' | 'DISABLED' | 'PROBING';
}

export interface AccountPoolStatus {
  totalCount: number;
  readyCount: number;
  limitedCount: number;
  tier1AvailableCount: number;
  tier2AvailableCount: number;
  earliestResetWaitMs?: number;
  earliestResetAccountEmail?: string;
}
