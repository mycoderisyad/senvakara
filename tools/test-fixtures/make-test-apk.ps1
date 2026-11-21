# Builds target/test-fixtures/senvakara-testfixture.apk from AndroidManifest.xml.
# Needs the user's Android SDK (build-tools + a platform) and a JDK; nothing is
# downloaded. Signs with a throwaway debug key generated next to the output.
$ErrorActionPreference = 'Stop'
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$bt = Get-ChildItem (Join-Path $sdk 'build-tools') | Sort-Object { [version]($_.Name -replace '[^0-9.]','') } | Select-Object -Last 1
$platform = Get-ChildItem (Join-Path $sdk 'platforms') | Where-Object Name -match '^android-\d+$' | Sort-Object { [int]($_.Name -replace 'android-','') } | Select-Object -Last 1
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root 'target\test-fixtures'
New-Item -ItemType Directory -Force $out | Out-Null
$unsigned = Join-Path $out 'unsigned.apk'
$aligned = Join-Path $out 'aligned.apk'
$apk = Join-Path $out 'senvakara-testfixture.apk'
$ks = Join-Path $out 'fixture-debug.keystore'

& (Join-Path $bt.FullName 'aapt2.exe') link --manifest (Join-Path $PSScriptRoot 'AndroidManifest.xml') `
  -I (Join-Path $platform.FullName 'android.jar') --min-sdk-version 21 --target-sdk-version 34 -o $unsigned
if ($LASTEXITCODE) { throw 'aapt2 link failed' }
& (Join-Path $bt.FullName 'zipalign.exe') -f -p 4 $unsigned $aligned
if ($LASTEXITCODE) { throw 'zipalign failed' }
if (-not (Test-Path $ks)) {
  keytool -genkeypair -keystore $ks -storepass fixture -keypass fixture -alias fixture `
    -keyalg RSA -keysize 2048 -validity 3650 -dname 'CN=Senvakara Test Fixture' | Out-Null
}
& (Join-Path $bt.FullName 'apksigner.bat') sign --ks $ks --ks-pass pass:fixture --key-pass pass:fixture --out $apk $aligned
if ($LASTEXITCODE) { throw 'apksigner failed' }
Remove-Item $unsigned, $aligned
Write-Output "built $apk using build-tools $($bt.Name) / $($platform.Name)"
