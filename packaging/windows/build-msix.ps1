<#
.SYNOPSIS
    Builds and packages Downmark as a native Windows MSIX package.

.DESCRIPTION
    Stages Downmark.exe, manifest, and visual assets, runs MakeAppx.exe from
    the Windows SDK to produce a .msix package, and optionally signs it (with a
    self-signed certificate if requested for development or testing).

.PARAMETER Version
    The package version in 3-part (0.1.0) or 4-part (0.1.0.0) format.

.PARAMETER SourceExe
    Path to the compiled Downmark.exe binary.

.PARAMETER OutputDir
    Destination directory for the generated .msix package.

.PARAMETER PackageName
    The package identity name (from Partner Center or local default).

.PARAMETER Publisher
    Subject DN of the publisher (must match the code-signing certificate).

.PARAMETER PublisherDisplayName
    Friendly display name of the publisher.

.PARAMETER SignSelfSigned
    When specified, generates a temporary self-signed test certificate, signs
    the MSIX, and exports Downmark-TestCert.cer for sideloading.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$SourceExe,

    [string]$OutputDir = ".",

    [string]$PackageName = "",

    [string]$Publisher = "",

    [string]$PublisherDisplayName = "",

    [switch]$SignSelfSigned
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $SourceExe)) {
    throw "Source executable not found: '$SourceExe'"
}

# Convert 3-part SemVer (0.1.0) to 4-part quad notation (0.1.0.0) required by MSIX
$msixVersion = if ($Version -match '^\d+\.\d+\.\d+$') { "$Version.0" } else { $Version }

# Resolve identity properties (command line arg > env var > default)
if (-not $PackageName) {
    $PackageName = if ($env:MSIX_PACKAGE_NAME) { $env:MSIX_PACKAGE_NAME } else { "Downmark" }
}
if (-not $Publisher) {
    $Publisher = if ($env:MSIX_PUBLISHER) { $env:MSIX_PUBLISHER } else { "CN=Downmark" }
}
if (-not $PublisherDisplayName) {
    $PublisherDisplayName = if ($env:MSIX_PUBLISHER_DISPLAY_NAME) { $env:MSIX_PUBLISHER_DISPLAY_NAME } else { "Downmark Contributors" }
}

# Locate Windows SDK tools (MakeAppx.exe and SignTool.exe)
function Get-SdkTool([string]$ToolName) {
    $kitsRoot = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
        "${env:ProgramFiles}\Windows Kits\10\bin"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1

    if (-not $kitsRoot) {
        throw "Windows SDK directory not found under Program Files."
    }

    $sdkDir = Get-ChildItem -Directory $kitsRoot |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        Select-Object -First 1

    if (-not $sdkDir) {
        throw "No Windows SDK version directories found under '$kitsRoot'."
    }

    $toolPath = Join-Path $sdkDir.FullName "x64\$ToolName"
    if (-not (Test-Path $toolPath)) {
        throw "Tool '$ToolName' not found at '$toolPath'."
    }
    return $toolPath
}

$makeAppx = Get-SdkTool "makeappx.exe"
$signTool = if ($SignSelfSigned) { Get-SdkTool "signtool.exe" } else { $null }

# Prepare staging workspace
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$resolvedOutputDir = (Resolve-Path $OutputDir).Path
$stagingDir = Join-Path ([System.IO.Path]::GetTempPath()) ("downmark-msix-" + [System.Guid]::NewGuid().ToString("N"))
$stagingAssets = Join-Path $stagingDir "Assets"
New-Item -ItemType Directory -Force -Path $stagingAssets | Out-Null

try {
    # 1. Copy payload binary
    Copy-Item -LiteralPath $SourceExe -Destination (Join-Path $stagingDir "Downmark.exe")

    # 2. Copy in-app visual assets
    $assetsSource = Join-Path $scriptDir "assets"
    foreach ($asset in @("Square44x44Logo.png", "Square150x150Logo.png", "StoreLogo.png")) {
        $sourceAsset = Join-Path $assetsSource $asset
        if (Test-Path -LiteralPath $sourceAsset) {
            Copy-Item -LiteralPath $sourceAsset -Destination (Join-Path $stagingAssets $asset)
        } else {
            throw "Required asset missing: $sourceAsset"
        }
    }

    # 3. Generate AppxManifest.xml with substituted identity
    $templatePath = Join-Path $scriptDir "AppxManifest.xml.template"
    $manifestXml = Get-Content -LiteralPath $templatePath -Raw -Encoding UTF8
    $manifestXml = $manifestXml.Replace('__PACKAGE_NAME__', $PackageName)
    $manifestXml = $manifestXml.Replace('__PUBLISHER__', $Publisher)
    $manifestXml = $manifestXml.Replace('__PUBLISHER_DISPLAY_NAME__', $PublisherDisplayName)
    $manifestXml = $manifestXml.Replace('__VERSION__', $msixVersion)

    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText((Join-Path $stagingDir "AppxManifest.xml"), $manifestXml, $utf8NoBom)

    # 4. Pack MSIX package
    $outputPackage = Join-Path $resolvedOutputDir "Downmark-$Version-windows-x64.msix"
    Write-Host "Packing MSIX: $outputPackage"
    Write-Host "  Package Name: $PackageName"
    Write-Host "  Publisher:    $Publisher"
    Write-Host "  Version:      $msixVersion"
    & $makeAppx pack /d $stagingDir /p $outputPackage /o
    if ($LASTEXITCODE -ne 0) {
        throw "MakeAppx.exe failed with exit code $LASTEXITCODE"
    }

    # 5. Sign with temporary self-signed certificate if requested
    if ($SignSelfSigned) {
        Write-Host "Creating self-signed test certificate for '$Publisher'..."
        $cert = New-SelfSignedCertificate `
            -Type Custom `
            -Subject $Publisher `
            -KeyUsage DigitalSignature `
            -FriendlyName "Downmark Test Certificate" `
            -CertStoreLocation "Cert:\CurrentUser\My" `
            -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3")

        $cerPath = Join-Path $resolvedOutputDir "Downmark-TestCert.cer"
        Export-Certificate -Cert $cert -FilePath $cerPath | Out-Null
        Write-Host "Exported test certificate: $cerPath"

        Write-Host "Signing MSIX package..."
        & $signTool sign /fd SHA256 /sha1 $cert.Thumbprint $outputPackage
        if ($LASTEXITCODE -ne 0) {
            throw "SignTool.exe failed with exit code $LASTEXITCODE"
        }
    }

    Write-Host "MSIX package built successfully: $outputPackage"
}
finally {
    if (Test-Path -LiteralPath $stagingDir) {
        Remove-Item -Recurse -Force -LiteralPath $stagingDir -ErrorAction SilentlyContinue
    }
}
