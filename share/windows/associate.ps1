param([switch]$Remove)
$ErrorActionPreference = 'Stop'
$launcher = Join-Path $PSScriptRoot 'pfx.exe'
$console = Join-Path $PSScriptRoot 'pfx.com'
$classes = 'HKCU:\Software\Classes'
$extension = '.pitofx'
$progId = 'pfx.document'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'pfx.lnk'

function Ensure-Key([string]$path) {
    if (-not (Test-Path -LiteralPath $path)) {
        New-Item -Path $path -Force | Out-Null
    }
}

function Set-Default([string]$path, [string]$value) {
    Ensure-Key $path
    Set-ItemProperty -LiteralPath $path -Name '(default)' -Value $value
}

function Send-Changed {
    $signature = '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int eventId, uint flags, System.IntPtr item1, System.IntPtr item2);'
    $shell = Add-Type -MemberDefinition $signature -Name 'Notify' -Namespace 'Pfx' -PassThru
    $shell::SHChangeNotify(0x08000000, 0, [System.IntPtr]::Zero, [System.IntPtr]::Zero)
}

if ($Remove) {
    foreach ($key in "$classes\$progId", "$classes\Applications\pfx.exe") {
        if (Test-Path -LiteralPath $key) {
            Remove-Item -LiteralPath $key -Recurse -Force
        }
    }
    $type = "$classes\$extension"
    if (Test-Path -LiteralPath $type) {
        Remove-ItemProperty -LiteralPath "$type\OpenWithProgids" -Name $progId -ErrorAction SilentlyContinue
        if ((Get-ItemProperty -LiteralPath $type).'(default)' -eq $progId) {
            Remove-ItemProperty -LiteralPath $type -Name '(default)'
        }
    }
    if (Test-Path -LiteralPath $shortcut) {
        Remove-Item -LiteralPath $shortcut -Force
    }
    Send-Changed
    Write-Output "pfx: $extension and the Start menu entry are removed"
    exit 0
}

foreach ($file in $launcher, $console) {
    if (-not (Test-Path -LiteralPath $file)) {
        Write-Error "pfx: $(Split-Path -Leaf $file) is not beside this script ($PSScriptRoot)"
        exit 1
    }
}

$command = '"{0}" "%1"' -f $launcher
$icon = '"{0}",0' -f $launcher
Set-Default "$classes\$progId" 'pfx document'
Set-Default "$classes\$progId\DefaultIcon" $icon
Set-Default "$classes\$progId\shell\open\command" $command
Set-Default "$classes\$extension" $progId
Set-ItemProperty -LiteralPath "$classes\$extension" -Name 'Content Type' -Value 'application/x-pfx'
Ensure-Key "$classes\$extension\OpenWithProgids"
New-ItemProperty -LiteralPath "$classes\$extension\OpenWithProgids" -Name $progId -PropertyType String -Value '' -Force | Out-Null
Set-Default "$classes\Applications\pfx.exe\shell\open\command" $command
Ensure-Key "$classes\Applications\pfx.exe\SupportedTypes"
New-ItemProperty -LiteralPath "$classes\Applications\pfx.exe\SupportedTypes" -Name $extension -PropertyType String -Value '' -Force | Out-Null

$link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
$link.TargetPath = $launcher
$link.WorkingDirectory = $PSScriptRoot
$link.IconLocation = "$launcher,0"
$link.Description = 'pfx'
$link.Save()

Send-Changed
Write-Output "pfx: $extension opens with $launcher; pfx is in the Start menu"
