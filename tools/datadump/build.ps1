# Compiles and runs DataDump against the vanilla 26.3 server jar.
# Usage: powershell -ExecutionPolicy Bypass -File build.ps1
$ErrorActionPreference = 'Stop'

$jdk      = 'C:\Program Files\Java\jdk-25.0.4\bin'
$javac    = Join-Path $jdk 'javac.exe'
$java     = Join-Path $jdk 'java.exe'

$here     = $PSScriptRoot
$vanilla  = 'E:\Projects\Rine-server\vanilla-data'
$serverJar = Join-Path $vanilla 'versions\26.3\server-26.3.jar'
$libsDir  = Join-Path $vanilla 'libraries'
$outDir   = Join-Path $here 'out'
$dumpDir  = 'E:\Projects\Rine-server\vanilla-data\dump'
$src      = Join-Path $here 'src\DataDump.java'

if (-not (Test-Path $serverJar)) { throw "Server jar not found: $serverJar" }

# Classpath: server jar + every jar under libraries (recursive).
$jars = @($serverJar) + @(Get-ChildItem -Path $libsDir -Recurse -Filter *.jar | ForEach-Object { $_.FullName })
$cp = $jars -join ';'
Write-Host "Classpath entries: $($jars.Count)"

# Use @argfile so the classpath does not hit the command-line length limit.
if (Test-Path $outDir) { Remove-Item -Recurse -Force $outDir }
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$argFile = Join-Path $outDir 'javac.args'
# Forward slashes avoid backslash-escape handling in argfiles; no spaces in these paths.
$cpFwd = $cp.Replace('\', '/')
$lines = @(
    '-encoding', 'UTF-8',
    '-proc:none',
    '-cp', $cpFwd,
    '-d', $outDir.Replace('\', '/'),
    $src.Replace('\', '/')
)
Set-Content -Path $argFile -Value $lines -Encoding ASCII

& $javac "@$argFile"
if ($LASTEXITCODE -ne 0) { throw "javac failed with exit code $LASTEXITCODE" }

New-Item -ItemType Directory -Force -Path $dumpDir | Out-Null
$runArgs = @(
    '-Xmx4G',
    '-cp', ($outDir + ';' + $cp),
    'DataDump',
    $serverJar,
    $dumpDir
)
& $java @runArgs
if ($LASTEXITCODE -ne 0) { throw "DataDump failed with exit code $LASTEXITCODE" }

Get-ChildItem -Path $dumpDir -Filter *.json | ForEach-Object {
    Write-Host ("{0}  {1:N0} bytes" -f $_.FullName, $_.Length)
}
