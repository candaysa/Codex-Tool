import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { randomBytes } from 'node:crypto';

export const RUNTIME_PROXY_PROVIDER_ID = 'codex-tool-proxy';
export const ORIGINAL_CONFIG_BACKUP_NAME = 'config.toml.original.bak';

export interface ConfigBindStatus {
  configPath: string;
  backupPath: string;
  exists: boolean;
  isBound: boolean;
  hasOriginalBackup: boolean;
  activeProvider: string | null;
}

function resolveCodexHome(): string {
  const envHome = process.env.CODEX_HOME?.trim();
  if (envHome) return envHome;
  const userHome = process.env.USERPROFILE?.trim() || os.homedir();
  return path.join(userHome, '.codex');
}

/**
 * Returns the resolved path to ~/.codex/config.toml
 */
export function getCodexConfigPath(): string {
  return path.join(resolveCodexHome(), 'config.toml');
}

/**
 * Returns the resolved path to pristine backup config.toml.original.bak
 */
export function getCodexBackupPath(): string {
  return path.join(resolveCodexHome(), ORIGINAL_CONFIG_BACKUP_NAME);
}

/**
 * Reads and inspects the current config.toml to check if Codex is bound to the proxy router
 */
export function inspectConfigBind(): ConfigBindStatus {
  const configPath = getCodexConfigPath();
  const backupPath = getCodexBackupPath();

  const exists = fs.existsSync(configPath);
  const hasOriginalBackup = fs.existsSync(backupPath);

  if (!exists) {
    return {
      configPath,
      backupPath,
      exists: false,
      isBound: false,
      hasOriginalBackup,
      activeProvider: null,
    };
  }

  try {
    const rawContent = fs.readFileSync(configPath, 'utf8');
    const match = /^\s*model_provider\s*=\s*["']([^"']+)["']/m.exec(rawContent);
    const activeProvider = match ? match[1] : null;

    const hasProxyBaseUrl = /^\s*openai_base_url\s*=\s*["']http:\/\/127\.0\.0\.1:\d+(?:\/v1)?["']/m.test(rawContent);
    const hasLegacyProvider =
      activeProvider === RUNTIME_PROXY_PROVIDER_ID &&
      rawContent.includes(`[model_providers.${RUNTIME_PROXY_PROVIDER_ID}]`);

    const isBound = hasProxyBaseUrl || hasLegacyProvider;

    return {
      configPath,
      backupPath,
      exists: true,
      isBound,
      hasOriginalBackup,
      activeProvider,
    };
  } catch {
    return {
      configPath,
      backupPath,
      exists: true,
      isBound: false,
      hasOriginalBackup,
      activeProvider: null,
    };
  }
}

function writeAtomicFile(targetPath: string, content: string): void {
  const tempPath = `${targetPath}.tmp.${process.pid}.${Date.now()}`;
  try {
    fs.writeFileSync(tempPath, content, 'utf8');
    try {
      fs.renameSync(tempPath, targetPath);
    } catch {
      // Windows sharing lock fallback
      fs.writeFileSync(targetPath, content, 'utf8');
      try {
        fs.unlinkSync(tempPath);
      } catch {}
    }
  } catch (err) {
    try {
      if (fs.existsSync(tempPath)) fs.unlinkSync(tempPath);
    } catch {}
    throw err;
  }
}

/**
 * Binds ~/.codex/config.toml to the local proxy router using the Codex Tool provider:
 * 1. Creates pristine backup config.toml.original.bak if it does not already exist.
 * 2. Selects the Codex Tool provider.
 * 3. Selects local bearer authentication and disables native OpenAI sign-in for this provider.
 * 4. Replaces the existing Codex Tool provider section.
 */
export function bindCodexConfig(port: number = 8080): { success: boolean; message: string } {
  const configPath = getCodexConfigPath();
  const backupPath = getCodexBackupPath();

  const codexDir = path.dirname(configPath);
  if (!fs.existsSync(codexDir)) {
    fs.mkdirSync(codexDir, { recursive: true });
  }

  // 1. Ensure pristine original backup
  if (fs.existsSync(configPath) && !fs.existsSync(backupPath)) {
    try {
      fs.copyFileSync(configPath, backupPath);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return { success: false, message: `Failed to create original config backup: ${msg}` };
    }
  }

  let raw = '';
  if (fs.existsSync(configPath)) {
    raw = fs.readFileSync(configPath, 'utf8');
  }

  // Remove existing proxy block and proxy base_url before rewriting
  const cleaned = removeProxyBlock(raw);

  // Filter out any existing model_provider and openai_base_url in root scope
  const lines = cleaned.split(/\r?\n/);
  let root = true;
  const filteredLines: string[] = [];
  for (const line of lines) {
    if (/^\s*\[/.test(line)) {
      root = false;
    }
    if (root && /^\s*(model_provider|openai_base_url|experimental_realtime_ws_base_url)\s*=/m.test(line)) {
      continue;
    }
    filteredLines.push(line);
  }

  const accountDirectory = path.join(codexDir, 'accounts');
  fs.mkdirSync(accountDirectory, { recursive: true });
  const keyPath = path.join(accountDirectory, 'proxy-key');
  if (!fs.existsSync(keyPath)) {
    try {
      fs.writeFileSync(keyPath, randomBytes(32).toString('hex'), { flag: 'wx', mode: 0o600 });
    } catch (error) {
      if (!fs.existsSync(keyPath)) throw error;
    }
  }
  const localKey = fs.readFileSync(keyPath, 'utf8');
  if (!/^[a-f0-9]{64}$/i.test(localKey)) {
    return { success: false, message: 'Local proxy credential is invalid' };
  }
  const proxyLines = [
    `model_provider = "${RUNTIME_PROXY_PROVIDER_ID}"`,
    `openai_base_url = "http://127.0.0.1:${port}/v1"`,
    `experimental_realtime_ws_base_url = "ws://127.0.0.1:${port}/v1"`,
  ];
  const providerLines = [
    `[model_providers.${RUNTIME_PROXY_PROVIDER_ID}]`,
    'name = "Codex Tool"',
    `base_url = "http://127.0.0.1:${port}/v1"`,
    'wire_api = "responses"',
    'requires_openai_auth = false',
    `experimental_bearer_token = "${localKey}"`,
    'supports_websockets = false',
  ];
  const updated = [...proxyLines, ...filteredLines, '', ...providerLines].join('\n').trimEnd() + '\n';

  try {
    writeAtomicFile(configPath, updated);
    return { success: true, message: `Successfully bound Codex config to proxy on port ${port}` };
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    return { success: false, message: `Failed to write bound config: ${msg}` };
  }
}

/**
 * Reverts ~/.codex/config.toml back to its original state.
 * Uses config.toml.original.bak if available; otherwise strips the proxy provider block and openai_base_url.
 */
export function restoreCodexConfig(): { success: boolean; message: string } {
  const configPath = getCodexConfigPath();
  const backupPath = getCodexBackupPath();

  if (fs.existsSync(backupPath)) {
    try {
      fs.copyFileSync(backupPath, configPath);
      // Ensure restored config does not carry legacy codex-tool-proxy
      let restoredRaw = fs.readFileSync(configPath, 'utf8');
      if (
        restoredRaw.includes(`[model_providers.${RUNTIME_PROXY_PROVIDER_ID}]`) ||
        /^\s*model_provider\s*=\s*["']codex-tool-proxy["']/m.test(restoredRaw)
      ) {
        restoredRaw = removeProxyBlock(restoredRaw);
        restoredRaw = restoredRaw.replace(/^\s*model_provider\s*=\s*.*$/m, 'model_provider = "openai"');
        writeAtomicFile(configPath, restoredRaw);
      }
      return { success: true, message: 'Restored original config.toml from pristine backup' };
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return { success: false, message: `Failed to restore from backup: ${msg}` };
    }
  }

  // If no backup exists, cleanly sanitize config.toml
  if (fs.existsSync(configPath)) {
    try {
      const raw = fs.readFileSync(configPath, 'utf8');
      let cleaned = removeProxyBlock(raw);

      // Reset model_provider if it pointed to our legacy proxy or ensure openai
      if (/^\s*model_provider\s*=\s*["']codex-tool-proxy["']/m.test(cleaned)) {
        cleaned = cleaned.replace(/^\s*model_provider\s*=\s*.*$/m, 'model_provider = "openai"');
      } else if (!/^\s*model_provider\s*=/m.test(cleaned)) {
        cleaned = 'model_provider = "openai"\n' + cleaned;
      }

      writeAtomicFile(configPath, cleaned);
      return { success: true, message: 'Removed proxy configuration from config.toml' };
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return { success: false, message: `Failed to sanitize config: ${msg}` };
    }
  }

  return { success: true, message: 'No config.toml existed' };
}

export function removeProxyBlock(raw: string): string {
  const lines = raw.split(/\r?\n/);
  const out: string[] = [];
  let skipping = false;
  let inRoot = true;
  const targetHeader = `[model_providers.${RUNTIME_PROXY_PROVIDER_ID}]`;
  const sectionHeaderRegex = /^\s*\[[^\]]+\]\s*(?:#.*)?$/;

  for (const line of lines) {
    const trimmed = line.trim();
    if (sectionHeaderRegex.test(trimmed)) {
      inRoot = false;
    }
    const headerWithoutComment = trimmed.replace(/\s*#.*$/, '');
    if (headerWithoutComment === targetHeader) {
      skipping = true;
      continue;
    }
    if (skipping && sectionHeaderRegex.test(trimmed)) {
      skipping = false;
    }
    if (
      inRoot &&
      /^\s*(?:openai_base_url|experimental_realtime_ws_base_url)\s*=\s*["'](?:http|ws):\/\/(?:127\.0\.0\.1|localhost):\d+(?:\/v1)?\/?["']/.test(trimmed)
    ) {
      continue;
    }
    if (!skipping) {
      out.push(line);
    }
  }

  return out.join('\n');
}

export function autoSanitizeLegacyConfig(): boolean {
  const configPath = getCodexConfigPath();
  const backupPath = getCodexBackupPath();

  if (fs.existsSync(backupPath)) {
    try {
      const bakRaw = fs.readFileSync(backupPath, 'utf8');
      const targetHeader = `[model_providers.${RUNTIME_PROXY_PROVIDER_ID}]`;
      if (
        bakRaw.includes(targetHeader) ||
        /^\s*model_provider\s*=\s*["']codex-tool-proxy["']/m.test(bakRaw)
      ) {
        let cleanedBak = removeProxyBlock(bakRaw);
        if (cleanedBak.includes(`model_provider = "${RUNTIME_PROXY_PROVIDER_ID}"`)) {
          cleanedBak = cleanedBak.replace(/^\s*model_provider\s*=\s*.*$/m, 'model_provider = "openai"');
        }
        writeAtomicFile(backupPath, cleanedBak);
      }
    } catch {}
  }

  if (!fs.existsSync(configPath)) return false;
  try {
    const raw = fs.readFileSync(configPath, 'utf8');
    if (raw.includes('requires_openai_auth = false') &&
        raw.includes('experimental_bearer_token = "') &&
        raw.includes('model_provider = "codex-tool-proxy"')) {
      return false;
    }
    const targetHeader = `[model_providers.${RUNTIME_PROXY_PROVIDER_ID}]`;
    const hasLegacySection = raw.includes(targetHeader);
    const hasLegacyProvider = /^\s*model_provider\s*=\s*["']codex-tool-proxy["']/m.test(raw);
    if (!hasLegacySection && !hasLegacyProvider) {
      return false;
    }
    let port = 8080;
    const portMatch = /base_url\s*=\s*["']http:\/\/(?:127\.0\.0\.1|localhost):(\d+)(?:\/v1)?\/?["']/.exec(raw);
    if (portMatch) {
      port = parseInt(portMatch[1], 10);
    }
    bindCodexConfig(port);
    return true;
  } catch {
    return false;
  }
}
