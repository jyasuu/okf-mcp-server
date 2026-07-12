#!/usr/bin/env pwsh
param(
    [string]$Version = "latest",
    [string]$InstallDir = "$env:USERPROFILE\.local\bin"
)

$ErrorActionPreference = "Stop"
$Repo = "jyasuu/okf-mcp-server"
$Binary = "okf-mcp-server"

function Get-Platform {
    $arch = [System.Environment]::Is64BitOperatingSystem
    $os = if ($IsWindows -or $env:OS -eq "Windows_NT") { "windows" }
          elseif ($IsLinux) { "linux" }
          elseif ($IsMacOS) { "darwin" }
          else { throw "Unsupported OS" }

    $cpuArch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture
    return switch ("$os-$cpuArch") {
        "windows-X64"   { "x86_64-pc-windows-msvc" }
        "linux-X64"     { "x86_64-unknown-linux-gnu" }
        "linux-Arm64"   { "aarch64-unknown-linux-gnu" }
        "darwin-Arm64"  { "aarch64-apple-darwin" }
        default         { throw "Unsupported platform: $os-$cpuArch" }
    }
}

function Get-Version {
    param([string]$Version)
    if ($Version -eq "latest" -or [string]::IsNullOrEmpty($Version)) {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
        return $release.tag_name
    }
    return $Version
}

$platform = Get-Platform
$version = Get-Version -Version $Version

$ext = if ($platform -like "*windows*") { "zip" } else { "tar.gz" }
$binaryName = if ($platform -like "*windows*") { "$Binary.exe" } else { $Binary }
$archive = "$Binary-$version-$platform.$ext"
$url = "https://github.com/$Repo/releases/download/$version/$archive"

Write-Host "Installing $Binary $version for $platform..."
Write-Host "  Download: $url"

$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

try {
    Invoke-WebRequest -Uri $url -OutFile "$tmpDir\$archive"

    if ($ext -eq "tar.gz") {
        tar xzf "$tmpDir\$archive" -C $tmpDir
    } else {
        Expand-Archive -Path "$tmpDir\$archive" -DestinationPath $tmpDir -Force
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item "$tmpDir\$binaryName" "$InstallDir\$Binary" -Force

    Write-Host ""
    Write-Host "Installed $Binary to $InstallDir\$Binary"
    Write-Host ""
    Write-Host "Add to PATH if not already:"
    Write-Host "  `$env:PATH = `"$InstallDir`" + `$env:PATH`"
} finally {
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
}
