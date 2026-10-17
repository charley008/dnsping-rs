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
    if ($files.Count -eq 0) { throw "Missing license text for $($package.name) $($package.version)" }
    [void]$out.AppendLine("`n===== $($package.name) $($package.version) [$($package.license)] =====")
    foreach ($file in $files) {
        [void]$out.AppendLine("`n--- $($file.Name) ---")
        [void]$out.AppendLine((Get-Content -LiteralPath $file.FullName -Raw))
    }
}
$out.ToString() | Set-Content -LiteralPath dist/LICENSE.txt -Encoding utf8
