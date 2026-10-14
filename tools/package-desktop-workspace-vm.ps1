param([string]$ReuseGuestSource)
$ErrorActionPreference='Stop'
if($ReuseGuestSource -and $ReuseGuestSource -notmatch '^source-[a-f0-9]{64}$'){throw 'Expected an owned VM source snapshot name'}
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zipPath=Join-Path $repo 'target\desktop-control-vm-source.zip'
$temporary=Join-Path $repo ('target\workspace-vm-source-'+[guid]::NewGuid().ToString('N')+'.zip')
$zip=[IO.Compression.ZipFile]::Open($temporary,[IO.Compression.ZipArchiveMode]::Create)
function Add-File([string]$path,[string]$entry){[IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,$path,$entry,[IO.Compression.CompressionLevel]::Fastest)|Out-Null}
function Add-Tree([string]$directory,[string]$prefix){Get-ChildItem -LiteralPath $directory -Recurse -File | ForEach-Object {$relative=$_.FullName.Substring($directory.Length).TrimStart([IO.Path]::DirectorySeparatorChar).Replace([IO.Path]::DirectorySeparatorChar,[char]'/');Add-File $_.FullName ($prefix+'/'+$relative)}}
try{
    Push-Location $repo
    try{$files=& git ls-files --cached --others --exclude-standard;if($LASTEXITCODE){throw 'Cannot list source files'}}finally{Pop-Location}
    foreach($file in $files){if($file -match '^(apps|libs|platforms|tools|docs)/|^(Cargo[.](toml|lock)|justfile|mise[.]toml|README[.]md|[.]cargo/config[.]toml)$'){$path=Join-Path $repo $file;if(Test-Path -LiteralPath $path -PathType Leaf){Add-File $path $file}}}
    if($ReuseGuestSource){
        Add-File (Join-Path $repo 'target\desktop-workspace-package\package.json') 'target/workspace-test-package/package.json'
        Add-File (Join-Path $repo 'target\desktop-workspace-package\DesktopWorkspace.exe') 'target/workspace-test-package/DesktopWorkspace.exe'
        $entry=$zip.CreateEntry('target/workspace-vm-reuse.json');$writer=[IO.StreamWriter]::new($entry.Open());try{$writer.Write('{"source":"'+$ReuseGuestSource+'"}')}finally{$writer.Dispose()}
        Add-Tree (Join-Path $repo 'apps\desktop-workspace\dist') 'apps/desktop-workspace/dist'
        $metadata=Get-Content -LiteralPath (Join-Path $repo 'target\desktop-workspace-package\package.json') -Raw|ConvertFrom-Json
        Add-Tree (Join-Path $repo ('target\desktop-workspace-package\payload\'+$metadata.version+'\resources\app\bin')) 'target/workspace-test-bin'
    }else{Add-Tree (Join-Path $repo 'target\desktop-workspace-package') 'target/workspace-test-package'}
    $modules=Join-Path $repo 'apps\desktop-workspace\node_modules'
    foreach($name in @('@playwright/test','playwright','playwright-core')){
        Push-Location (Join-Path $repo 'apps\desktop-workspace')
        try{$resolved=& node -e "const p=require('node:path');process.stdout.write(p.dirname(require.resolve('$name/package.json',{paths:[p.dirname(require.resolve('@playwright/test'))]})))";if($LASTEXITCODE){throw 'Test module resolution failed'}}finally{Pop-Location}
        $path=[string]$resolved
        Add-Tree $path ('apps/desktop-workspace/node_modules/'+$name)
    }
    $entry=$zip.CreateEntry('target/vm-test-options.json');$writer=[IO.StreamWriter]::new($entry.Open());try{$writer.Write('{"scope":"workspace-shell"}')}finally{$writer.Dispose()}
}finally{$zip.Dispose()}
Move-Item -LiteralPath $temporary -Destination $zipPath -Force
Write-Output "VM source and package prepared: $zipPath"
