$ErrorActionPreference = 'Stop'
$version = '0.6.21'
$digest = '186e009c498477ec56661807e496337caa474f5fcde9b39922f8d95962307fc9'
$name = "CodexAutoResume-v$version-win-x64.zip"
$url = "https://github.com/songyb111-gachon/codex-auto-resume-windows/releases/download/v$version/$name"
$archive = Join-Path $env:CODEX_TOOL_INSTALL_TEMP $name
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'
Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $archive -TimeoutSec 180
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $digest) {
    throw 'The Auto Resume archive failed SHA-256 verification.'
}
$payload = Join-Path $env:CODEX_TOOL_INSTALL_TEMP 'payload'
Expand-Archive -LiteralPath $archive -DestinationPath $payload
& (Join-Path $payload 'install\install.ps1') -Silent -SkipStartup
exit $LASTEXITCODE
