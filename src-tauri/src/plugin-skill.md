---
name: codex-tool-auto-resume
description: Check the Windows recovery watcher, show interrupted Codex conversations, and open recovery settings through Codex Tool.
---

Use the plugin's codex-tool-resume tools to manage the installed Windows recovery engine.

- For watcher status, call get_status.
- For pending recoveries, call list_pending. Use the exact interruption ID for subsequent actions.
- To open or configure recovery, call open_settings.
- Change state only when requested and report the tool result.
- If the recovery server is missing, direct the user to Codex Tool → Ayarlar → Codex Eklentisini Kur.
