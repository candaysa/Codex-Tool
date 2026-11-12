# Codex Tool

Codex Tool is a desktop companion for managing Codex accounts, viewing usage limits, and monitoring interrupted conversations.

## Features

- Add ChatGPT accounts through browser sign-in, copy the sign-in link, and choose the account Codex uses.
- Read quota windows from ChatGPT. A weekly-only account shows a single quota bar. The window length and time until reset are displayed separately.
- On startup, configure Codex to route requests through the local proxy with the Codex Tool provider and a local connection credential. Codex receives a local bearer credential and reports no ChatGPT account for this provider. The app attempts a live configuration reload when the binding changes. If the Codex control socket is unavailable, reopen Codex once to load the new address. Keep Codex Tool open while using the proxy.
- **Ayarlar → Profili Gizleyerek Bağlan** uses the local model catalog from Codex's cache when available. **Eski OpenAI Sohbetlerini Aç** retains the OpenAI provider for existing chats and keeps proxy account routing enabled. The Codex profile is visible in this mode. **Codex’i Fabrika Ayarlarına Döndür** restores the previous provider settings.
- The proxy prefers the selected usable account and tries another account if a Responses request receives an authorization or quota rejection before output begins. Account changes apply to new HTTP requests and WebSocket connections while upstream ChatGPT credentials stay in the account pool. The proxy selects an account for each WebSocket turn and rebuilds completed conversation context when the next turn uses another account. Voice calls keep the same account for their control channel.
- The proxy accepts Responses WebSockets, dictation, voice calls, and image generation and editing requests routed to its address. Availability depends on the Codex client and ChatGPT account. **Ayarlar → Codex’i Fabrika Ayarlarına Döndür** restores the previous provider settings.
- On Windows, check for the **Codex Tool** plugin at startup and install it if missing. If needed, the app downloads and verifies the recovery engine before installing it on Windows x64. Restart Codex to load a newly installed plugin. Edit the recovery message under **Ayarlar → Otomatik devam mesajı**.

## Build from source

Use Node.js 22.13 or later, a current Rust toolchain, and the Tauri prerequisites for your platform. Windows requires Visual Studio C++ Build Tools and WebView2. Recovery and plugin installation are Windows features.

```sh
npm ci
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run tauri:dev
```

Build the Windows x64 executable on a Windows x64 machine:

```sh
npm run tauri:build -- --no-bundle
```

Run or distribute `src-tauri/target/release/codex-tool.exe`.

## Local data

The app reads Codex settings and stores account sessions under `CODEX_HOME`, which defaults to `~/.codex`. Exported backups can contain sign-in tokens and private Codex settings. Keep them private.

## License

[MIT](LICENSE)
