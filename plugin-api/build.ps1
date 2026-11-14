# Builds the Rine plugin API jar and the example plugin.
# Outputs:
#   plugin-api/rine-api.jar        embedded into crates/rine-plugin with include_bytes!
#   plugin-api/examples/hello.jar  example Bukkit-style plugin used by the integration test
#
# The JDK is taken from $env:RINE_JDK, then the default install, then PATH.
# Run this before `cargo build -p rine-plugin` whenever a Java source changes.

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot

$jdk = if ($env:RINE_JDK) { $env:RINE_JDK } else { 'C:\Program Files\Java\jdk-25.0.4' }
$javac = Join-Path $jdk 'bin\javac.exe'
$jar = Join-Path $jdk 'bin\jar.exe'
if (-not (Test-Path $javac)) {
    $javac = 'javac'
    $jar = 'jar'
}

function Invoke-Checked([string]$exe, [string[]]$arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { throw "$exe failed with exit code $LASTEXITCODE" }
}

# 1. rine-api: compile src/ into out/, then pack rine-api.jar
$out = Join-Path $root 'out'
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $out | Out-Null

$sources = @(Get-ChildItem -Path (Join-Path $root 'src') -Recurse -Filter *.java | ForEach-Object { $_.FullName })
Invoke-Checked $javac (@('-encoding', 'UTF-8', '--release', '21', '-Xlint:-options', '-d', $out) + $sources)

$apiJar = Join-Path $root 'rine-api.jar'
if (Test-Path $apiJar) { Remove-Item -Force $apiJar }
Invoke-Checked $jar @('--create', '--file', $apiJar, '-C', $out, '.')
Write-Host "built $apiJar"

# 2. example plugin: compile against rine-api.jar, copy plugin.yml, pack hello.jar
$helloDir = Join-Path $root 'examples\hello'
$helloOut = Join-Path $helloDir 'out'
if (Test-Path $helloOut) { Remove-Item -Recurse -Force $helloOut }
New-Item -ItemType Directory -Force $helloOut | Out-Null

$helloSources = @(Get-ChildItem -Path (Join-Path $helloDir 'src') -Recurse -Filter *.java | ForEach-Object { $_.FullName })
Invoke-Checked $javac (@('-encoding', 'UTF-8', '--release', '21', '-cp', $apiJar, '-d', $helloOut) + $helloSources)
Copy-Item (Join-Path $helloDir 'plugin.yml') $helloOut

$helloJar = Join-Path $root 'examples\hello.jar'
if (Test-Path $helloJar) { Remove-Item -Force $helloJar }
Invoke-Checked $jar @('--create', '--file', $helloJar, '-C', $helloOut, '.')
Write-Host "built $helloJar"
