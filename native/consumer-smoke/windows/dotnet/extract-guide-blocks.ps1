# extract-guide-blocks.ps1
# Extracts csharp code blocks from docs/windows-dotnet-guide.md into generated files.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File path\to\extract-guide-blocks.ps1
#
# This script is called as a pre-build target from NativeOidcReleaseConsumer.csproj.
# It validates the guide's code blocks and generates:
#   - GuideClient.cs (concatenation of all csharp blocks in order)
#   - GuideMain.cs (the csharp main.cs block)
#
# Written in PowerShell 5.1-compatible syntax.

param(
    [string]$SourceFile = "docs/windows-dotnet-guide.md"
)

$sourcePath = Join-Path $PSScriptRoot "$SourceFile"
$outputDir = "$PSScriptRoot\guide"
$guideClientPath = Join-Path $outputDir "GuideClient.cs"
$guideMainPath = Join-Path $outputDir "GuideMain.cs"

# Ensure output directory exists
if (!(Test-Path $outputDir)) {
    New-Item -ItemType Directory -Path $outputDir | Out-Null
}

try {
    # Read the source file
    if (Test-Path $sourcePath) {
        $content = Get-Content -Path $sourcePath -Raw -Encoding UTF8
    } else {
        Write-Error "Source file not found: $sourcePath"
        exit 1
    }

    # Normalize line endings to LF (for cross-platform compatibility)
    if ($env:SYSTEMROOT -ne 'C:\Program Files\PowerShell\7') {
        $content = $content -creplace "`r`n", "`n"
    }

    # Count csharp fences and main.cs fences
    $allFences = [regex]::Matches($content, '`{3}csharp', 'IgnoreCase').Count
    $mainCsFences = [regex]::Matches($content, '`{3}csharp\s+main\.cs', 'IgnoreCase').Count
    
    if ($allFences -eq 0) {
        Write-Error "No csharp blocks found in $SourceFile"
        exit 1
    }

    if ($mainCsFences -ne 1) {
        Write-Error "Expected exactly one csharp main.cs block, found $($mainCsFences)"
        exit 1
    }

    # Extract non-main.c#harp blocks into GuideClient.cs
    $guideBlocks = @()
    $lastEnd = 0
    
    foreach ($match in [regex]::Matches($content, '`{3}csharp', 'IgnoreCase')) {
        $start = $match.Index
        
        # Check if this is main.cs block
        if ([regex]::Match($content.Substring($start), '[^`]{3,4}csharp\s+main\.cs', 'IgnoreCase').Success) {
            # Skip - this is the main.cs block
            continue
        }

        # Find the closing fence
        $remaining = $content.Substring($start + 3)
        $depth = 0
        $i = 0
        while ($i -lt $remaining.Length) {
            if ([char]::IsLetterOrDigit($remaining[$i]) -or $remaining[$i] -eq ' ') {
                if ($remaining.Substring($i, 3) -eq '```') {
                    $depth++
                    if ($depth -eq 0) {
                        # Found closing fence
                        break
                    }
                } else if ($remaining.Substring($i, 4) -eq "````") {
                    $depth = [Math]::Max(0, $depth - 1)
                }
            }
            $i++
        }
        
        # Extract block content (between opening and closing fences)
        if ($i -gt 0) {
            $blockContent = $content.Substring($start + 3, $i - 3)
            $guideBlocks += $blockContent.Trim()
            
            # Track position after this block for next search
            $nextSearchStart = $start + 3 + $i
            if ([regex]::Match($content.Substring($nextSearchStart), '[^`]{3,4}csharp', 'IgnoreCase').Success) {
                continue
            } else {
                # Make sure we're past all fence markers for the next block search
                while ($nextSearchStart -lt $content.Length - 200) {
                    if ($content.Substring($nextSearchStart, 3) -eq '```' -or
                        [regex]::Match($content.Substring($nextSearchStart), '[^`]{3,4}csharp', 'IgnoreCase').Success) {
                        $nextSearchStart += 500
                    } else {
                        break
                    }
                }
            }
        }
    }

    # Generate GuideClient.cs
    if ($guideBlocks.Count -eq 0) {
        Write-Error "No non-main.c#harp csharp blocks found"
        exit 1
    }

    $guideClientContent = "// Generated from docs/windows-dotnet-guide.md. Do not edit.`n`n"
    
    foreach ($block in $guideBlocks) {
        if (-not [string]::IsNullOrWhiteSpace($block)) {
            $guideClientContent += "`n`n$block"
        }
    }

    Set-Content -Path $guideClientPath -Value $guideClientContent -Encoding UTF8
    
    # Extract main.cs block into GuideMain.cs
    $mainCsMatch = [regex]::Matches($content, '`{3}csharp\s+main\.cs', 'IgnoreCase')
    if ($mainCsMatch.Count -eq 0) {
        Write-Error "No csharp main.cs block found"
        exit 1
    }

    $mainStart = $mainCsMatch[0].Index
    $mainRemaining = $content.Substring($mainStart + 3)
    $depth = 0
    $i = 0
    while ($i -lt $mainRemaining.Length) {
        if ([char]::IsLetterOrDigit($mainRemaining[$i]) -or $mainRemaining[$i] -eq ' ') {
            if ($mainRemaining.Substring($i, 3) -eq '```') {
                $depth++
                if ($depth -eq 0) {
                    # Found closing fence
                    break
                }
            } else if ($mainRemaining.Substring($i, 4) -eq "````") {
                $depth = [Math]::Max(0, $depth - 1)
            }
        }
        $i++
    }

    if ($i -gt 0) {
        $mainContent = $content.Substring($mainStart + 3, $i - 3)
        Set-Content -Path $guideMainPath -Value ("// Generated from docs/windows-dotnet-guide.md. Do not edit.`n`n$mainContent".Trim()) -Encoding UTF8
    }

    Write-Host "Generated $guideClientPath with $($guideBlocks.Count) blocks"
    Write-Host "Generated $guideMainPath (main.cs block)"

} catch {
    Write-Error "Error processing guide: $($_.Exception.Message)"
    exit 1
}

exit 0
