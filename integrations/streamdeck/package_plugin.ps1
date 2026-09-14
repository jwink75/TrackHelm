$ErrorActionPreference = "Stop"

$Dir = Split-Path -Parent $MyInvocation.MyCommand.Path
$OutDir = Join-Path $Dir "..\..\dist-streamdeck"

if (-not (Test-Path $OutDir)) {
    New-Item -ItemType Directory -Path $OutDir | Out-Null
}

Write-Host "1. Generating action icons..."
python "$Dir\generate_icons.py"

Write-Host "2. Packaging .streamDeckPlugin archive..."
$PluginDir = Join-Path $Dir "com.trackhelm.controller.sdPlugin"
$ZipOut = Join-Path $OutDir "com.trackhelm.controller.streamDeckPlugin.zip"
$FinalOut = Join-Path $OutDir "com.trackhelm.controller.streamDeckPlugin"
$AliasOut = Join-Path $OutDir "TrackHelm.streamDeckPlugin"

if (Test-Path $ZipOut) { Remove-Item $ZipOut -Force }
if (Test-Path $FinalOut) { Remove-Item $FinalOut -Force }
if (Test-Path $AliasOut) { Remove-Item $AliasOut -Force }

Compress-Archive -Path "$PluginDir\*" -DestinationPath $ZipOut
Move-Item -Path $ZipOut -Destination $FinalOut -Force
Copy-Item -Path $FinalOut -Destination $AliasOut -Force

Write-Host "✓ Created: dist-streamdeck\TrackHelm.streamDeckPlugin"

# 3. Direct install to Stream Deck Plugins directory on Windows
$AppData = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::ApplicationData)
$StreamDeckPlugins = Join-Path $AppData "Elgato\StreamDeck\Plugins"

if (Test-Path $StreamDeckPlugins) {
    $TargetDir = Join-Path $StreamDeckPlugins "com.trackhelm.controller.sdPlugin"
    if (Test-Path $TargetDir) { Remove-Item $TargetDir -Recurse -Force }
    Copy-Item -Path $PluginDir -Destination $TargetDir -Recurse
    Write-Host "✓ Installed directly to: $TargetDir"
}
