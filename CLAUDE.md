# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

An MQTT driver for Siemens WinCC OA 3.21, written in Rust and C++. See README.md for the user-facing documentation (address syntax, options, Sparkplug B, redundancy, panels); it is the reference for behavior and is kept up to date with every change.

Three layers, each statically linked into the next:

1. `crates/rsmqtt` – MQTT client with a C ABI (`include/rsmqtt.h`) and a header-only C++ wrapper (`rsmqtt.hpp`), built on the pure-Rust Paho client (`paho-mqtt` 2.0 from the `eclipse-paho/paho.mqtt.rust` git repo, pinned to a `rev`). Poll based: operations return tokens, results/messages/connection changes go into a per-client queue drained with `poll()`; the library never calls into the host from its threads (fits WinCC OA's single-threaded manager loop).
2. `crates/rsmqtt-wincc` – all driver logic that needs no WinCC OA API, as a second C ABI (`include/rsmqtt_wincc.h` + `rsmqtt_wincc.hpp`, namespace `rsmqtt::wincc`): address syntax (`address.rs`), JSON paths and the one-pass `Extractor` (`json.rs`), dyn value lists, redundancy names (`redu.rs`), Sparkplug B host application (`sparkplug.rs`, protobuf types from `srad-types`), connection settings (`settings.rs`: broker DP / config file → rsmqtt options, default QoS, Sparkplug host ids; passwords are decrypted through a callback into the driver), message router (`router.rs`: per connection the addresses, subscriptions, `$state` addresses and the Sparkplug host; a message or connection change returns deliveries / publishes / warnings for the driver to carry out), payload text → int / uint / float / bool (`values.rs`, used by `RsMqttTrans`). It links `rsmqtt` as an rlib, so `rsmqtt_wincc.lib` is the only Rust library in the driver (two Rust staticlibs would bring the Rust std in twice).
3. `driver/` – the WinCC OA ComDrv driver `WCCOArsmqtt` (C++). `RsMqttHWService` (work loop, carries out the router's outcomes as HWObjects, writes), `RsMqttConnections` (one `RsMqttConnection` per broker: its `Settings` and `Router`, client state, address index), `RsMqttHWMapper` (addresses → subscriptions), `RsMqttTrans` (value conversion incl. dyn), `RsMqttBrokerDp` (`DrvIntDp` objects for `_RsMqtt_<broker>` / `_RsMqttDriver_<n>` DPs), `RsMqttRedu` (host 1/2), `RsMqttResources` (config sections `[rsmqtt]` / `[rsmqtt_<num>]`), `RsMqttSecret` (encrypted passwords via the API's `DrvSecurity` / `CipherUtility`). The `RsMqttAddress/Json/Sparkplug/Redu.hxx` headers forward to the Rust ABI.

`driver/project/` mirrors a WinCC OA project (CTRL library `scripts/libs/rsmqtt.ctl`, `scripts/userPara.ctl`, `scripts/userDrivers.ctl`, PARA panels in `panels/para`, `dplist/*.asc`, `config/config.redu`). Brokers are DPs `_RsMqtt_<broker>` listed in `_RsMqttDriver_<n>.Brokers`; the driver re-creates the client when their `Config` changes.

## Commands

Windows (MSVC Rust target, VS with C++, CMake, WinCC OA 3.21 API). Use PowerShell syntax when giving commands to the user.

```powershell
cargo build --release -p rsmqtt-wincc          # both crates (driver CMake also runs this)
cargo test -p rsmqtt-wincc                      # driver logic, no broker needed
cargo test -p rsmqtt-wincc sparkplug::tests::host   # a single test
cargo clippy -p rsmqtt -p rsmqtt-wincc --all-targets -- -D warnings
$env:API_ROOT = "C:\Program Files\Siemens\WinCC_OA\3.21\api"
cmake -S driver -B driver/build -G "Visual Studio 18 2026" -A x64
cmake --build driver/build --config RelWithDebInfo   # -> driver/build/RelWithDebInfo/WCCOArsmqtt.exe
cmake -S examples/cpp -B examples/cpp/build; cmake --build examples/cpp/build --config Release   # pubsub, wincc_test
python driver/tools/gen_panels.py               # regenerates both .pnl files
```

Broker tests (`crates/rsmqtt/tests/broker.rs`) run only with `RSMQTT_TEST_URI` set (plus optional `RSMQTT_TEST_WS_URI`, `RSMQTT_TEST_TLS_URI` + `RSMQTT_TEST_CA`):

```powershell
docker run -d --name rsmqtt-mosq -p 18830:1883 -p 18831:9001 -v "${PWD}\tests\mosquitto.conf:/mosquitto/config/mosquitto.conf" eclipse-mosquitto:2
$env:RSMQTT_TEST_URI="tcp://localhost:18830"; $env:RSMQTT_TEST_WS_URI="ws://localhost:18831"; cargo test -p rsmqtt --lib --test broker
```

Live tests in WinCC OA: `tests/broker/start-broker.ps1` starts `rsmqtt-mosq-full` (ports 18840–18845, certs via `make-certs.sh`), `python tests/broker/netutil.py` provides proxies and a freezable forwarder. `tests/winccoa/option_tests.ctl` runs one group per call through `evalScript` from any CTRL manager (the README lists the groups; `<group>:keep` leaves the test DPs, honored only by `json`, `dyn` and `spb`). `tests/winccoa/redu_lib_tests.ctl` is appended to the library source and run with `evalScript` (see its header). `tests/broker/spb_edge.py` simulates a Sparkplug edge node.

## Working rules

- Panels are generated: edit `driver/tools/gen_panels.py` (record layouts copied from stock V14 panels, LANG entries need UTF-8 byte counts), never the `.pnl` files. GEDI rewrites panels when saving (it dropped the `$broker` dollar declaration), so don't save generated panels from GEDI.
- The WinCC OA project uses copies: after changing `driver/project/**` or the driver, copy `rsmqtt.ctl` / panels into the project and `WCCOArsmqtt.exe` into its `bin` (stop the driver manager first). A running UI manager keeps the CTRL library it loaded at startup.
- The driver runs unlicensed in demo mode and stops itself after 2 hours; restart the manager before live tests.
- CTRL pitfalls seen here: `file` is a type keyword (not a parameter name); no 64-bit integer literals (`(long) "..."`); `dpCopy` lives in `mp.ctl`; DPs named `*_2` are reserved for redundancy twins. WinCC OA clamps a source time older than the current value's time.
- `rsmqttT(de, en)` gives panel/state texts in the UI language; tests comparing texts must use it (the project language is `de_AT`).
- Deleting a broker DP while it is still listed in `Brokers` makes WinCC OA log warning 107 per element; unlist first, delete a second later (the driver releases its `DrvIntDp` on unlisting).
- `simulateReduHost = 1|2` in `[rsmqtt]` makes the driver act as a redundant host on a single system (tests only; see `host2_*` groups).
- Local paho development: a `[patch]` for the git URL in `.cargo/config.toml` (git-ignored); never commit `Cargo.lock` while it is active (the paho entries lose their git source).
- Don't run stress tests (`crates/rsmqtt/tests/stress.rs`, `crates/rsmqtt/examples/stress.rs`) against public brokers; use random client ids in tests.
- Files mix CRLF and LF; scripts that rewrite files keep the existing line endings. Python heredocs in Git Bash break on long CTRL code: write the script to a file first.
