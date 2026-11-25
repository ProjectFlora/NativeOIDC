# Copies the C# code of docs/windows-dotnet-guide.md into source files, so
# that the guide's code is what the .NET guide fixtures compile:
#
# - every ```csharp block, in order, into GuideClient.cs
# - the single ```csharp Program.cs block into Program.cs
#
# A ```csharp Program.cs fence is not a ```csharp fence, because the match
# includes the newline after the info string. Fails when the guide has no
# csharp block, not exactly one Program.cs block, or an unterminated block.
# Called by GuideSources.targets before each build. Compatible with Windows
# PowerShell 5.1.
param(
    [Parameter(Mandatory = $true)][string]$Guide,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)

$ErrorActionPreference = "Stop"

function Get-GuideBlocks {
    param([string]$Text, [string]$Fence)

    $blocks = New-Object System.Collections.Generic.List[string]
    $position = 0
    while ($true) {
        $start = $Text.IndexOf($Fence, $position, [System.StringComparison]::Ordinal)
        if ($start -lt 0) { break }
        $start += $Fence.Length
        $end = $Text.IndexOf("`n``````", $start, [System.StringComparison]::Ordinal)
        if ($end -lt 0) { throw "Unterminated code block in $Guide" }
        $blocks.Add($Text.Substring($start, $end - $start))
        $position = $end + 4
    }
    return , $blocks
}

$text = [System.IO.File]::ReadAllText($Guide).Replace("`r`n", "`n")
$clientBlocks = Get-GuideBlocks -Text $text -Fence "``````csharp`n"
$programBlocks = Get-GuideBlocks -Text $text -Fence "``````csharp Program.cs`n"
if ($clientBlocks.Count -eq 0) { throw "No csharp blocks found in $Guide" }
if ($programBlocks.Count -ne 1) {
    throw "Expected one csharp Program.cs block in $Guide, found $($programBlocks.Count)"
}

$notice = "// Generated from docs/windows-dotnet-guide.md by extract-guide-blocks.ps1. Do not edit.`n"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[void][System.IO.Directory]::CreateDirectory($OutputDirectory)
[System.IO.File]::WriteAllText((Join-Path $OutputDirectory "GuideClient.cs"),
    $notice + (($clientBlocks | ForEach-Object { "`n$_`n" }) -join ""), $utf8)
[System.IO.File]::WriteAllText((Join-Path $OutputDirectory "Program.cs"),
    $notice + $programBlocks[0] + "`n", $utf8)
Write-Host "Extracted $($clientBlocks.Count) csharp blocks and Program.cs from $Guide"
