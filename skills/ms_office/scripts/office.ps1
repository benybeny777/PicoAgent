param(
  [string]$Action,
  [string]$App,
  [string]$InputPath,
  [string]$OutputPath,
  [string]$Needle,
  [string]$Replacement
)
$ErrorActionPreference = "Stop"
function Save-Word($doc, $path) {
  if ($path) { $doc.SaveAs([ref]$path) } else { $doc.Save() }
}
function Save-Excel($wb, $path) {
  if ($path) { $wb.SaveAs($path) } else { $wb.Save() }
}
function Save-PowerPoint($pres, $path) {
  if ($path) { $pres.SaveAs($path) } else { $pres.Save() }
}
if ($Action -eq "replace_text") {
  if (-not $Needle) { throw "needle が空です" }
  if ($App -eq "word") {
    $word = New-Object -ComObject Word.Application
    $word.Visible = $false
    try {
      $doc = $word.Documents.Open($InputPath)
      $find = $doc.Content.Find
      [void]$find.Execute($Needle, $false, $true, $false, $false, $false, $true, 1, $false, $Replacement, 2)
      Save-Word $doc $OutputPath
      $doc.Close()
    } finally { $word.Quit() }
  } elseif ($App -eq "excel") {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    try {
      $wb = $excel.Workbooks.Open($InputPath)
      foreach ($ws in $wb.Worksheets) {
        if ($ws.UsedRange) { [void]$ws.UsedRange.Replace($Needle, $Replacement, 2) }
      }
      Save-Excel $wb $OutputPath
      $wb.Close($true)
    } finally { $excel.Quit() }
  } elseif ($App -eq "powerpoint") {
    $ppt = New-Object -ComObject PowerPoint.Application
    try {
      $pres = $ppt.Presentations.Open($InputPath, $false, $false, $false)
      foreach ($slide in $pres.Slides) {
        foreach ($shape in $slide.Shapes) {
          if ($shape.HasTextFrame -and $shape.TextFrame.HasText) {
            [void]$shape.TextFrame.TextRange.Replace($Needle, $Replacement)
          }
        }
      }
      Save-PowerPoint $pres $OutputPath
      $pres.Close()
    } finally { $ppt.Quit() }
  } else { throw "app は word/excel/powerpoint のいずれかです" }
  Write-Output "OK: replace_text"
} elseif ($Action -eq "export_pdf") {
  if (-not $OutputPath) { throw "output_path が必要です" }
  if ($App -eq "word") {
    $word = New-Object -ComObject Word.Application
    $word.Visible = $false
    try {
      $doc = $word.Documents.Open($InputPath)
      $doc.ExportAsFixedFormat($OutputPath, 17)
      $doc.Close($false)
    } finally { $word.Quit() }
  } elseif ($App -eq "excel") {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    try {
      $wb = $excel.Workbooks.Open($InputPath)
      $wb.ExportAsFixedFormat(0, $OutputPath)
      $wb.Close($false)
    } finally { $excel.Quit() }
  } elseif ($App -eq "powerpoint") {
    $ppt = New-Object -ComObject PowerPoint.Application
    try {
      $pres = $ppt.Presentations.Open($InputPath, $false, $false, $false)
      $pres.SaveAs($OutputPath, 32)
      $pres.Close()
    } finally { $ppt.Quit() }
  } else { throw "app は word/excel/powerpoint のいずれかです" }
  Write-Output "OK: export_pdf"
} else { throw "action は replace_text/export_pdf のいずれかです" }
