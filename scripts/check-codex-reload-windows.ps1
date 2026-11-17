# Hosted-runner-only integration test. Codex deliberately refuses managed daemons
# under an elevated token, so exercise the real Windows route as a temporary local
# standard user. All credentials, conversations and model responses are synthetic.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'This fixture is only for an ephemeral GitHub Actions runner' }
$taskRoot = Join-Path $env:SystemDrive ('sr' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$taskUser = 'sw' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$taskOldPath = $env:PATH
$taskOldTemp = $env:TEMP
$taskOldTmp = $env:TMP
$taskOldBinary = $env:SWITCHER_SESSION_TEST_BINARY
$taskProcess = $null
$taskUserCreated = $false
try {
    New-Item -ItemType Directory -Path "$taskRoot/tmp" -Force | Out-Null
    $npmRoot = (& npm root --global).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate the installed Codex package' }
    $candidates = @(
        "$npmRoot/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc",
        "$npmRoot/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc"
    )
    $package = $candidates | Where-Object { Test-Path "$_/codex-package.json" } | Select-Object -First 1
    if (-not $package) { throw 'A complete pinned Codex package is required' }
    Copy-Item -Path $package -Destination "$taskRoot/codex" -Recurse
    # Cargo's artifact layout differs across toolchains. Ask it for the executable
    # instead of assuming target/debug/deps, which is absent on some runners.
    $artifacts = & cargo test --locked --manifest-path src-tauri/Cargo.toml --no-run --lib --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot build the isolated Rust integration test' }
    $test = $artifacts | ForEach-Object {
        $entry = $_ | ConvertFrom-Json
        if ($entry.reason -eq 'compiler-artifact' -and $entry.profile.test -and $entry.target.name -eq 'switcher_lib' -and $entry.executable) { $entry.executable }
    } | Select-Object -Last 1
    if (-not $test) { throw 'Rust integration test executable not found' }
    Copy-Item -LiteralPath $test -Destination "$taskRoot/check.exe"
    Copy-Item src-tauri/target/release/switcher.exe "$taskRoot/switcher.exe"
    New-Item -ItemType Directory -Path "$taskRoot/probe/app-server-daemon" -Force | Out-Null
    '{"updater":{"autoUpdateEnabled":false}}' | Set-Content "$taskRoot/probe/app-server-daemon/settings.json" -Encoding utf8NoBOM
    "cli_auth_credentials_store = `"file`"`nchatgpt_base_url = `"http://127.0.0.1:9`"`n" | Set-Content "$taskRoot/probe/config.toml" -Encoding utf8NoBOM

    $taskPassword = ConvertTo-SecureString ('Sw!' + [Guid]::NewGuid().ToString('N') + '7z') -AsPlainText -Force
    $account = New-LocalUser -Name $taskUser -Password $taskPassword -PasswordNeverExpires
    $taskUserCreated = $true
    $usersGroup = Get-LocalGroup -SID 'S-1-5-32-545'
    Add-LocalGroupMember -Group $usersGroup -Member $account
    & icacls.exe $taskRoot /grant "${taskUser}:(OI)(CI)F" /T /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Cannot grant access to the synthetic test workspace' }
    # Codex validates this pre-created state directory before starting. Its
    # contract is an exact protected, inheritable, user-only DACL and owner;
    # merely granting our standard user access to an admin-created tree fails.
    $stateAcl = [System.Security.AccessControl.DirectorySecurity]::new()
    $taskSid = $account.SID.Value
    $stateAcl.SetSecurityDescriptorSddlForm("O:${taskSid}D:P(A;OICI;FA;;;${taskSid})")
    Set-Acl -LiteralPath "$taskRoot/probe/app-server-daemon" -AclObject $stateAcl
    $credential = [System.Management.Automation.PSCredential]::new("$env:COMPUTERNAME\$taskUser", $taskPassword)
    $env:PATH = "$taskRoot/codex/bin;$taskOldPath"
    $env:TEMP = "$taskRoot/tmp"
    $env:TMP = "$taskRoot/tmp"
    $env:SWITCHER_SESSION_TEST_BINARY = "$taskRoot/switcher.exe"
    # Supply the fixture paths explicitly even if logon builds a new environment.
    @"
@echo off
set "PATH=$taskRoot\codex\bin;%PATH%"
set "TEMP=$taskRoot\tmp"
set "TMP=$taskRoot\tmp"
set "SWITCHER_SESSION_TEST_BINARY=$taskRoot\switcher.exe"
set "OPENAI_API_KEY="
set "CODEX_API_KEY="
set "OPENAI_BASE_URL="
set "CODEX_HOME=$taskRoot\probe"
where codex
codex --version
codex app-server daemon start
if errorlevel 1 exit /b 1
codex app-server daemon stop
if errorlevel 1 exit /b 1
set "CODEX_HOME="
"$taskRoot\check.exe" installed_codex_daemon_reloads_selected_account_without_external_auth --ignored --test-threads=1
"@ | Set-Content "$taskRoot/run.cmd" -Encoding ascii
    $taskProcess = Start-Process -FilePath $env:ComSpec -ArgumentList @('/d', '/s', '/c', "$taskRoot/run.cmd") `
      -WorkingDirectory $taskRoot -Credential $credential -LoadUserProfile -PassThru `
      -RedirectStandardOutput "$taskRoot/output.log" -RedirectStandardError "$taskRoot/error.log"
    if (-not $taskProcess.WaitForExit(180000)) {
        $taskProcess.Kill($true)
        throw 'The isolated Codex integration test timed out'
    }
    Get-Content "$taskRoot/output.log"
    Get-Content "$taskRoot/error.log"
    if ($taskProcess.ExitCode -ne 0) { throw "Codex integration test failed ($($taskProcess.ExitCode))" }
} finally {
    $env:PATH = $taskOldPath
    $env:TEMP = $taskOldTemp
    $env:TMP = $taskOldTmp
    $env:SWITCHER_SESSION_TEST_BINARY = $taskOldBinary
    if ($taskUserCreated) { Remove-LocalUser -Name $taskUser -ErrorAction SilentlyContinue }
    Remove-Item $taskRoot -Recurse -Force -ErrorAction SilentlyContinue
}
