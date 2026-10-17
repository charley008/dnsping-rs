$ErrorActionPreference = 'Stop'

# Keep all attribution in the existing LICENSE.txt rather than shipping hundreds
# of dependency directories. Cargo's filtered resolve graph excludes other OSes.
$metadata = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Dependency inventory failed' }
$packages = @{}
foreach ($package in $metadata.packages) { $packages[$package.id] = $package }
$nodes = @{}
foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
$visited = [System.Collections.Generic.HashSet[string]]::new()
$queue = [System.Collections.Generic.Queue[string]]::new()
$queue.Enqueue($metadata.resolve.root)
while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    if (-not $visited.Add($id)) { continue }
    foreach ($dependency in $nodes[$id].deps) {
        if (@($dependency.dep_kinds | Where-Object kind -ne 'dev').Count -gt 0) {
            $queue.Enqueue($dependency.pkg)
        }
    }
}
$out = [System.Text.StringBuilder]::new()
$overrides = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'license-overrides.json') -Raw | ConvertFrom-Json -AsHashtable
[void]$out.AppendLine((Get-Content -LiteralPath LICENSE -Raw))
[void]$out.AppendLine("`nTHIRD-PARTY NOTICES`n")
foreach ($package in ($metadata.packages | Sort-Object name,version)) {
    if ($package.id -eq $metadata.resolve.root -or -not $visited.Contains($package.id)) { continue }
    $sourceDir = Split-Path -Parent $package.manifest_path
    $files = @(Get-ChildItem -LiteralPath $sourceDir -File | Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)([._-]|$)')
    if ($package.license_file) {
        $path = Join-Path $sourceDir $package.license_file
        if (Test-Path -LiteralPath $path -PathType Leaf) { $files += Get-Item -LiteralPath $path }
    }
    foreach ($folder in @('licenses', 'license')) {
        $path = Join-Path $sourceDir $folder
        if (Test-Path -LiteralPath $path -PathType Container) {
            $files += Get-ChildItem -LiteralPath $path -File -Recurse
        }
    }
    $files = @($files | Sort-Object FullName -Unique)
    [void]$out.AppendLine("`n===== $($package.name) $($package.version) [$($package.license)] =====")
    if ($files.Count -eq 0) {
        $key = "$($package.name)@$($package.version)"
        if (-not $overrides.ContainsKey($key)) { throw "Missing license text for $key" }
        foreach ($notice in $overrides[$key]) {
            [void]$out.AppendLine("Source: $($notice.source)")
            [void]$out.AppendLine($notice.text)
        }
    }
    foreach ($file in $files) {
        [void]$out.AppendLine("`n--- $($file.Name) ---")
        [void]$out.AppendLine((Get-Content -LiteralPath $file.FullName -Raw))
    }
}
foreach ($notice in $overrides['lucide-icons']) {
    [void]$out.AppendLine("`n===== Lucide icons (bundled in gpui-component-assets) =====")
    [void]$out.AppendLine("Source: $($notice.source)")
    [void]$out.AppendLine($notice.text)
}
# The statically linked Rust standard library is outside Cargo's package graph.
$sysroot = rustc --print sysroot
if ($LASTEXITCODE -ne 0) { throw 'Rust sysroot lookup failed' }
$rustDocs = Join-Path $sysroot 'share/doc/rust'
$copyright = Join-Path $rustDocs 'COPYRIGHT-library.html'
if (-not (Test-Path -LiteralPath $copyright)) { throw 'Rust standard library notices missing' }
$html = Get-Content -LiteralPath $copyright -Raw
[void]$out.AppendLine("`n===== Rust standard library =====")
[void]$out.AppendLine([System.Net.WebUtility]::HtmlDecode([regex]::Replace($html, '<[^>]*>', '')))
$licenseNames = [regex]::Matches($html, 'href="licenses/([A-Za-z0-9._-]+)') | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
foreach ($name in $licenseNames) {
    [void]$out.AppendLine("`n--- Rust: $name ---")
    [void]$out.AppendLine((Get-Content -LiteralPath (Join-Path $rustDocs "licenses/$name") -Raw))
}
$out.ToString() | Set-Content -LiteralPath dist/LICENSE.txt -Encoding utf8
