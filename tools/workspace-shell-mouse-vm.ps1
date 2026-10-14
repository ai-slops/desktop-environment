param([int]$SourceX,[int]$SourceY,[int]$TargetX,[int]$TargetY)
$ErrorActionPreference='Stop'
$computer=Get-CimInstance Win32_ComputerSystem
if($computer.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU'){throw 'Native mouse probe requires a test VM'}
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WorkspaceVmMouse {
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
}
'@
[WorkspaceVmMouse]::SetCursorPos($SourceX,$SourceY)|Out-Null
Start-Sleep -Milliseconds 100
[WorkspaceVmMouse]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
try{
    for($step=1;$step -le 25;$step++){
        $x=[int]($SourceX+($TargetX-$SourceX)*$step/25)
        $y=[int]($SourceY+($TargetY-$SourceY)*$step/25)
        [WorkspaceVmMouse]::SetCursorPos($x,$y)|Out-Null
        Start-Sleep -Milliseconds 20
    }
}finally{[WorkspaceVmMouse]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}
