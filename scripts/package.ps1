$ErrorActionPreference = 'Stop'
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    cargo build --locked --release --bin dnsping
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    Copy-Item -LiteralPath target/release/dnsping.exe -Destination dist/DNSPing.exe
    Copy-Item -LiteralPath README.md -Destination dist/使用说明.md
    Copy-Item -LiteralPath LICENSE -Destination dist/LICENSE.txt
    Copy-Item -LiteralPath settings.example.json -Destination dist/settings.example.json
    $metadata = cargo metadata --locked --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Dependency inventory failed' }
    $lines = @('# Third-party dependency inventory', '', 'Direct and transitive Cargo dependencies; see each package for its license text.', '')
    foreach ($package in ($metadata.packages | Sort-Object name,version)) {
        if ($package.name -eq 'dnsping') { continue }
        $lines += ('- {0} {1} — {2} — https://crates.io/crates/{0}/{1}' -f $package.name,$package.version,$package.license)
        $licenseDir = Join-Path 'dist/licenses' ('{0}-{1}' -f $package.name,$package.version)
        $sourceDir = Split-Path -Parent $package.manifest_path
        $notices = Get-ChildItem -LiteralPath $sourceDir -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([._-]|$)' }
        if ($notices) {
            New-Item -ItemType Directory -Path $licenseDir -Force | Out-Null
            foreach ($notice in $notices) {
                Copy-Item -LiteralPath $notice.FullName -Destination $licenseDir
                $copiedNotice = Get-Item -LiteralPath (Join-Path $licenseDir $notice.Name)
                if ($copiedNotice.LastWriteTime.Year -lt 1980) { $copiedNotice.LastWriteTime = [datetime]'2000-01-01' }
            }
        }
    }
    $lines | Set-Content -LiteralPath dist/THIRD-PARTY.md -Encoding utf8
    $hash = Get-FileHash -LiteralPath dist/DNSPing.exe -Algorithm SHA256
    ('{0}  DNSPing.exe' -f $hash.Hash.ToLowerInvariant()) | Set-Content -LiteralPath dist/SHA256SUMS.txt -Encoding ascii
    Compress-Archive -LiteralPath dist/DNSPing.exe,dist/使用说明.md,dist/LICENSE.txt,dist/THIRD-PARTY.md,dist/SHA256SUMS.txt,dist/settings.example.json,dist/licenses -DestinationPath dist/DNSPing-windows-x64.zip -Force
    Compress-Archive -LiteralPath src,assets,docs,scripts,.cargo,.github,Cargo.toml,Cargo.lock,build.rs,README.md,LICENSE,settings.example.json,.gitignore,.gitattributes -DestinationPath dist/DNSPing-source.zip -Force
    Get-Item -LiteralPath dist/DNSPing.exe,dist/DNSPing-windows-x64.zip,dist/DNSPing-source.zip | Select-Object FullName,Length
} finally {
    Pop-Location
}
