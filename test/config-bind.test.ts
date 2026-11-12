import { test, describe, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {
  bindCodexConfig,
  restoreCodexConfig,
  inspectConfigBind,
  autoSanitizeLegacyConfig,
  RUNTIME_PROXY_PROVIDER_ID,
  ORIGINAL_CONFIG_BACKUP_NAME,
} from '../core/config/config-bind-service';

describe('Codex Tool - Config Bind & Restore Service', () => {
  let tempCodexDir: string;
  let originalEnv: string | undefined;

  beforeEach(() => {
    tempCodexDir = fs.mkdtempSync(path.join(os.tmpdir(), 'codex-test-home-'));
    originalEnv = process.env.CODEX_HOME;
    process.env.CODEX_HOME = tempCodexDir;
  });

  afterEach(() => {
    process.env.CODEX_HOME = originalEnv;
    try {
      fs.rmSync(tempCodexDir, { recursive: true, force: true });
    } catch {}
  });

  test('Preserves original config as pristine backup and injects proxy', () => {
    const configPath = path.join(tempCodexDir, 'config.toml');
    const backupPath = path.join(tempCodexDir, ORIGINAL_CONFIG_BACKUP_NAME);

    // Initial user config
    const userOriginalContent = 'model = "gpt-5"\nmodel_provider = "openai"\n';
    fs.writeFileSync(configPath, userOriginalContent, 'utf8');

    // 1. Initial inspection: not bound, no backup
    const initialStatus = inspectConfigBind();
    assert.strictEqual(initialStatus.isBound, false);
    assert.strictEqual(initialStatus.hasOriginalBackup, false);

    // 2. Perform bind
    const bindResult = bindCodexConfig(8080);
    assert.strictEqual(bindResult.success, true);

    // Verify backup created with original content
    assert.strictEqual(fs.existsSync(backupPath), true);
    assert.strictEqual(fs.readFileSync(backupPath, 'utf8'), userOriginalContent);

    // Verify config is now bound
    const boundStatus = inspectConfigBind();
    assert.strictEqual(boundStatus.isBound, true);
    assert.strictEqual(boundStatus.activeProvider, RUNTIME_PROXY_PROVIDER_ID);

    const updatedConfig = fs.readFileSync(configPath, 'utf8');
    assert.match(updatedConfig, /model_provider = "codex-tool-proxy"/);
    assert.match(updatedConfig, /base_url = "http:\/\/127\.0\.0\.1:8080\/v1"/);
    assert.match(updatedConfig, /requires_openai_auth = false/);

    // 3. Restore config back to original
    const restoreResult = restoreCodexConfig();
    assert.strictEqual(restoreResult.success, true);

    const restoredStatus = inspectConfigBind();
    assert.strictEqual(restoredStatus.isBound, false);
    assert.strictEqual(fs.readFileSync(configPath, 'utf8'), userOriginalContent);
  });

  test('Sequential bind calls are strictly idempotent and never duplicate sections', () => {
    const configPath = path.join(tempCodexDir, 'config.toml');
    fs.writeFileSync(configPath, 'model = "gpt-5"\n', 'utf8');

    // Bind 3 times consecutively
    bindCodexConfig(8080);
    bindCodexConfig(8080);
    bindCodexConfig(8080);

    const content = fs.readFileSync(configPath, 'utf8');
    const count = (content.match(/experimental_bearer_token\s*=/g) || []).length;
    assert.strictEqual(count, 1, 'Local proxy credential must appear exactly once');
  });

  test('Preserves other third-party providers and custom comments', () => {
    const configPath = path.join(tempCodexDir, 'config.toml');
    const complexConfig = [
      '# My custom Codex settings',
      'model = "o3-mini"',
      'model_provider = "custom-gateway"',
      '',
      '[model_providers.custom-gateway]',
      'base_url = "https://my-proxy.company.internal/v1"',
      'wire_api = "responses"',
      '',
      '[editor]',
      'line_numbers = true',
    ].join('\n');

    fs.writeFileSync(configPath, complexConfig, 'utf8');

    // Bind
    bindCodexConfig(8080);

    // Verify third-party provider and editor section still intact
    let content = fs.readFileSync(configPath, 'utf8');
    assert.match(content, /\[model_providers\.custom-gateway\]/);
    assert.match(content, /line_numbers = true/);
    assert.match(content, /# My custom Codex settings/);

    // Restore
    restoreCodexConfig();
    content = fs.readFileSync(configPath, 'utf8');
    assert.strictEqual(content, complexConfig, 'Complete file must be restored byte-for-byte');
  });

  test('Handles non-existent initial config file safely', () => {
    // No config.toml exists
    const status = inspectConfigBind();
    assert.strictEqual(status.exists, false);
    assert.strictEqual(status.isBound, false);

    // Bind should create it
    const bindRes = bindCodexConfig(9090);
    assert.strictEqual(bindRes.success, true);

    const postStatus = inspectConfigBind();
    assert.strictEqual(postStatus.exists, true);
    assert.strictEqual(postStatus.isBound, true);

    // Unbind cleanly removes proxy block
    const unbindRes = restoreCodexConfig();
    assert.strictEqual(unbindRes.success, true);
  });

  test('autoSanitizeLegacyConfig upgrades the legacy binding to local bearer authentication', () => {
    const configPath = path.join(tempCodexDir, 'config.toml');
    const legacy = [
      'model = "gpt-6-luna"',
      'model_provider = "codex-tool-proxy"',
      '',
      '[model_providers.codex-tool-proxy]',
      'base_url = "http://127.0.0.1:58641/v1"',
      'wire_api = "responses"',
      '',
      '[editor]',
      'line_numbers = true',
    ].join('\n');
    fs.writeFileSync(configPath, legacy, 'utf8');

    const migrated = autoSanitizeLegacyConfig();
    assert.strictEqual(migrated, true);

    const status = inspectConfigBind();
    assert.strictEqual(status.isBound, true);
    assert.strictEqual(status.activeProvider, RUNTIME_PROXY_PROVIDER_ID);

    const updated = fs.readFileSync(configPath, 'utf8');
    assert.match(updated, /model_provider = "codex-tool-proxy"/);
    assert.match(updated, /base_url = "http:\/\/127\.0\.0\.1:58641\/v1"/);
    assert.match(updated, /requires_openai_auth = false/);
    assert.match(updated, /line_numbers = true/);

    // Idempotent: second call returns false
    const secondCall = autoSanitizeLegacyConfig();
    assert.strictEqual(secondCall, false);
  });
});
