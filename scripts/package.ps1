$ErrorActionPreference = 'Stop'
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    cargo build --locked --release --bin dnsping
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    Copy-Item -LiteralPath target/release/dnsping.exe -Destination dist/DNSPing.exe
    & ./scripts/notices.ps1
    Copy-Item -LiteralPath settings.example.json -Destination dist/settings.json
    $hash = Get-FileHash -LiteralPath dist/DNSPing.exe -Algorithm SHA256
    ('{0}  DNSPing.exe' -f $hash.Hash.ToLowerInvariant()) | Set-Content -LiteralPath dist/SHA256SUMS.txt -Encoding ascii
    Compress-Archive -LiteralPath dist/DNSPing.exe,dist/LICENSE.txt,dist/SHA256SUMS.txt,dist/settings.json -DestinationPath dist/DNSPing-windows-x64.zip -Force
    $hash = Get-FileHash -LiteralPath dist/DNSPing-windows-x64.zip -Algorithm SHA256
    ('{0}  DNSPing-windows-x64.zip' -f $hash.Hash.ToLowerInvariant()) | Set-Content -LiteralPath dist/SHA256SUMS.txt -Encoding ascii
    Get-Item -LiteralPath dist/DNSPing-windows-x64.zip,dist/SHA256SUMS.txt | Select-Object FullName,Length
} finally {
    Pop-Location
}
