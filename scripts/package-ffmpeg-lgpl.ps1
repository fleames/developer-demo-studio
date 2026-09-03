param(
  [Parameter(Mandatory = $true)]
  [string]$Destination
)

$ErrorActionPreference = "Stop"

$archiveName = "ffmpeg-n8.1.2-50-g1a748fe2cd-win64-lgpl-8.1.zip"
$archiveUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-02-13-13/$archiveName"
$expectedSha256 = "da423df4788dabc645dc19789bf3fb71736d51f0a79fd96f285a286000753a19"
$work = Join-Path ([System.IO.Path]::GetTempPath()) "developer-demo-studio-ffmpeg"
$archive = Join-Path $work $archiveName
$expanded = Join-Path $work "expanded"

Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $expanded, $Destination | Out-Null

Write-Host "Downloading pinned LGPL FFmpeg 8.1 build..."
Invoke-WebRequest -Uri $archiveUrl -OutFile $archive

$actualSha256 = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
  throw "FFmpeg archive checksum mismatch. Expected $expectedSha256, received $actualSha256."
}

Expand-Archive -Path $archive -DestinationPath $expanded
$ffmpeg = Get-ChildItem $expanded -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
$ffprobe = Get-ChildItem $expanded -Recurse -Filter "ffprobe.exe" | Select-Object -First 1
if (-not $ffmpeg -or -not $ffprobe) {
  throw "Verified FFmpeg archive did not contain ffmpeg.exe and ffprobe.exe."
}

Copy-Item $ffmpeg.FullName (Join-Path $Destination "ffmpeg.exe")
Copy-Item $ffprobe.FullName (Join-Path $Destination "ffprobe.exe")

$license = Get-ChildItem $expanded -Recurse -Filter "LICENSE.txt" | Select-Object -First 1
if ($license) {
  Copy-Item $license.FullName (Join-Path $Destination "FFMPEG-LICENSE.txt")
}

@"
FFmpeg 8.1 LGPL build
=====================

Developer Demo Studio invokes FFmpeg and FFprobe as separate executables.
These files are not part of Developer Demo Studio and remain licensed by
their respective copyright holders under GNU LGPL 2.1 or later.

Binary build: BtbN/FFmpeg-Builds, win64 LGPL
Build scripts and configuration: https://github.com/BtbN/FFmpeg-Builds
Corresponding FFmpeg source: https://ffmpeg.org/releases/ffmpeg-8.1.tar.xz
FFmpeg license information: https://ffmpeg.org/legal.html

Archive: $archiveName
Archive SHA-256: $expectedSha256

No GPL or nonfree FFmpeg build is bundled.
"@ | Set-Content (Join-Path $Destination "FFMPEG-NOTICE.txt")

& (Join-Path $Destination "ffmpeg.exe") -hide_banner -version
& (Join-Path $Destination "ffprobe.exe") -hide_banner -version
if ($LASTEXITCODE -ne 0) {
  throw "Bundled FFmpeg tools failed their executable smoke test."
}

Remove-Item $work -Recurse -Force
