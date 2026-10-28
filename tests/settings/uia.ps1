# Drives twm-settings through UI Automation (AccessKit). Invoke/SetValue do
# not move the mouse or type, so this runs while the PC is in use.
#   uia.ps1 -Action list|invoke|setvalue -Name <element name> [-Value v] [-Index n] [-Type Button|Slider|...]
# invoke looks for buttons and setvalue for sliders unless -Type says otherwise
# (a control and its row label share the same name).
param([string]$Action = "list", [string]$Name = "", [string]$Value = "", [int]$Index = 0, [string]$Type = "")
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$cond = New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, "TilingWindowsManager Settings")
$win = $null
for ($i = 0; $i -lt 40 -and -not $win; $i++) {
  $win = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
  if (-not $win) { Start-Sleep -Milliseconds 250 }
}
if (-not $win) { "error: settings window not found"; exit 1 }
$all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
if ($Action -eq "list") {
  foreach ($e in $all) { "{0} | {1}" -f $e.Current.ControlType.ProgrammaticName, $e.Current.Name }
  exit 0
}
if (-not $Type) { $Type = @{ invoke = "Button"; setvalue = "Slider" }[$Action] }
$matches = @($all | Where-Object { $_.Current.Name -eq $Name -and $_.Current.ControlType.ProgrammaticName -eq "ControlType.$Type" })
if ($matches.Count -le $Index) { "error: no element named '$Name' (#$Index of $($matches.Count))"; exit 1 }
$e = $matches[$Index]
switch ($Action) {
  "invoke" {
    # egui exposes plain buttons with Invoke and selectable ones (the page
    # list) as toggles / selection items.
    $p = $null
    if ($e.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$p)) { $p.Invoke(); "invoked '$Name'" }
    elseif ($e.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$p)) { $p.Select(); "selected '$Name'" }
    elseif ($e.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$p)) { $p.Toggle(); "toggled '$Name'" }
    else { "error: '$Name' cannot be invoked; patterns: " + (($e.GetSupportedPatterns() | % { $_.ProgrammaticName }) -join ", "); exit 1 }
  }
  "setvalue" {
    $p = $null
    if ($e.TryGetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern, [ref]$p)) { $p.SetValue([double]$Value); "range set '$Name' = $Value" }
    elseif ($e.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$p)) { $p.SetValue($Value); "value set '$Name' = $Value" }
    else { "error: '$Name' supports neither RangeValue nor Value"; exit 1 }
  }
}
