# Starts the broker for the WinCC OA option tests (tests/winccoa/option_tests.ctl).
#
#   powershell -File tests/broker/start-broker.ps1
#   python tests/broker/netutil.py        # proxies, silent listener, forwarder
#
# The password file is created inside the container: a password file mounted
# from a Windows folder is not readable for the mosquitto user.
$ErrorActionPreference = "Stop"
$dir = $PSScriptRoot
if ( -not (Test-Path "$dir\certs\ca.crt") ) {
    & "C:\Program Files\Git\bin\bash.exe" "$dir/make-certs.sh"
}
docker rm -f rsmqtt-mosq-full 2>$null | Out-Null
docker run -d --name rsmqtt-mosq-full `
    -p 18840:1883 -p 18841:9001 -p 18842:8883 -p 18843:8884 -p 18844:8885 -p 18845:1885 `
    -v "$dir\mosquitto-full.conf:/mosquitto/config/mosquitto.conf" `
    -v "$dir\certs:/certs" `
    eclipse-mosquitto:2 `
    sh -c "mosquitto_passwd -c -b /mosquitto/passwd testuser 'test pass' && chown mosquitto:mosquitto /mosquitto/passwd && chmod 600 /mosquitto/passwd && exec mosquitto -c /mosquitto/config/mosquitto.conf" | Out-Null
Start-Sleep 2
# mosquitto prints a deprecation note for per_listener_settings on stderr
$ErrorActionPreference = "Continue"
docker logs rsmqtt-mosq-full 2>&1 | ForEach-Object { "$_" } | Select-String "listen socket on port|running" | ForEach-Object { $_.Line }
