# rsmqtt — MQTT for WinCC OA, in Rust

Two parts:

| Part | Path | What it is |
|---|---|---|
| **rsmqtt** library | [crates/rsmqtt](crates/rsmqtt) | MQTT client with a C ABI ([rsmqtt.h](crates/rsmqtt/include/rsmqtt.h)) and a header-only C++17 wrapper ([rsmqtt.hpp](crates/rsmqtt/include/rsmqtt.hpp)). Built on the pure-Rust Eclipse Paho client ([paho.mqtt.rust](https://github.com/eclipse-paho/paho.mqtt.rust), branch `v2`). No OpenSSL or Paho C DLLs, so it can't clash with the OpenSSL that ships with WinCC OA. |
| **WCCOArsmqtt** driver | [driver](driver) | A WinCC OA 3.21 driver (ComDrv / `DrvManager`) that maps DPEs to MQTT topics through `_address` configs. Links rsmqtt statically. |

## Features (the Paho C `MQTTAsync` feature set)

- MQTT 3.1, 3.1.1 and 5.0; QoS 0, 1 and 2, with session state carried over reconnects
- `tcp://` `mqtt://` `ssl://` `tls://` `mqtts://` `ws://` `wss://`, HTTP(S) proxy for WebSockets, custom HTTP headers
- TLS via rustls: trust store file/dir, Windows system store, client certs (mutual TLS), host name check (`verify`), chain check (`enableServerCertAuth`), cipher suites, minimum version, ALPN
- Last will, retained messages, automatic reconnect (min/max back-off), fail-over list of server URIs
- Offline buffering (`sendWhileDisconnected`, `maxBufferedMessages`, `deleteOldestMessages`), `maxInflight`
- Persistence: none, memory or file (`persistenceDir`)
- MQTT v5: all properties, reason codes, subscription options (no-local, retain-as-published, retain handling), shared subscriptions, session expiry, user properties
- MQTT v5 enhanced authentication: SCRAM-SHA-1/256/512 built in (`auth.method`), other methods through a callback (`rsmqtt_options_set_auth_callback`)
- Option names are the Paho C struct field names, and return codes -1…-18 match `MQTTASYNC_*`

Not supported: TLS-PSK and password-protected private keys (rustls can't handle either).

Enhanced authentication: `auth.method=SCRAM-SHA-256` (or `-1`, `-512`; MQTT v5 only) takes the credentials from `username` and `password`. The password is not sent, and the connect fails unless the broker's CONNACK proves that it knows the password (the server signature); the check is repeated on every automatic reconnect. Up to 1,000,000 PBKDF2 iterations are accepted, and the password is used without SASLprep (the same for ASCII). For other methods (Kerberos, a vendor's token scheme), `rsmqtt_options_set_auth_callback()` answers the broker's AUTH challenges, with `auth.data` as the data of CONNECT. It is called on a library thread while the connect waits, the one exception to the polling model, so it must answer quickly and must not call into the client; once set it answers the SCRAM methods too, and then the library does not check the server signature (the callback never sees the CONNACK). A CONNACK whose server signature is wrong ends the connection, also on an automatic reconnect, which then stops: the reason is reported as a lost connection, and the client stays disconnected until the host connects again.

## Library model

```cpp
#include <rsmqtt.hpp>

rsmqtt::Options opts;
opts.set("serverURI", "tcp://localhost:1883")
    .set("clientId", "winccoa-1")
    .set("automaticReconnect", true);

rsmqtt::Client cli(opts);
cli.wait(cli.connect());
cli.subscribe("plant/#", 1);
cli.publish("plant/cmd", "start", 1);

// in the manager's work loop, never blocking:
while (auto ev = cli.poll()) {
  if (ev->type() == rsmqtt::EventType::Message)
    handle(ev->topic(), ev->payload());
}
```

- **Poll based.** Operations return a token right away. Results, incoming messages and connection changes go into a per-client queue that the host drains with `poll()`. The library never calls into the host from its own threads, except for an optional notify callback and the trace callback. That suits WinCC OA's single-threaded manager loop.
- **C ABI underneath.** Rust can't export a C++ ABI, so `rsmqtt.h` is the binary interface and `rsmqtt.hpp` adds RAII and exceptions on top. Nothing is generated and the build isn't tied to a particular MSVC toolset.
- **Driver logic in Rust too.** [crates/rsmqtt-wincc](crates/rsmqtt-wincc) holds everything in the driver that needs no WinCC OA API: the address syntax, the JSON paths and the one-pass extractor, the dyn value lists, the redundancy names and the Sparkplug B host (protobuf types from [srad-types](https://crates.io/crates/srad-types)). Its ABI is `rsmqtt_wincc.h`, wrapped by `rsmqtt_wincc.hpp`. The crate links `rsmqtt` into one static library, `rsmqtt_wincc.lib`, so the Rust standard library is in the executable only once.
- `waitForCompletion(token, timeout)` gives the blocking style of Paho's `MQTTClient`.

## Building

Requirements: Rust (MSVC target), Visual Studio with C++ tools, CMake, and the WinCC OA 3.21 API.

```powershell
cargo build --release -p rsmqtt-wincc
```

```powershell
$env:API_ROOT = "C:\Program Files\Siemens\WinCC_OA\3.21\api"
```

```powershell
cmake -S driver -B driver/build -G "Visual Studio 18 2026" -A x64
```

```powershell
cmake --build driver/build --config RelWithDebInfo
```

The driver's CMake runs `cargo build --release -p rsmqtt-wincc` on its own and links `target/release/rsmqtt_wincc.lib` (the client library alone is `cargo build --release -p rsmqtt`). The result is `driver/build/RelWithDebInfo/WCCOArsmqtt.exe`; copy it into the project's `bin` directory.

The pure-Rust Paho client (2.0) is not on crates.io yet, so Cargo fetches it from the Eclipse repository, pinned to one commit of its `v2` branch (see [crates/rsmqtt/Cargo.toml](crates/rsmqtt/Cargo.toml)); the first build needs network access. To build against a local checkout of paho.mqtt.rust instead, e.g. next to this repository, create `.cargo/config.toml` (ignored by git):

```toml
[patch."https://github.com/eclipse-paho/paho.mqtt.rust"]
paho-mqtt = { path = "../paho.mqtt.rust" }
```

While the patch is active, Cargo writes the Paho entries into `Cargo.lock` without their git source; don't commit `Cargo.lock` in that state (remove the patch and run `cargo metadata` once to get it back).

On Linux the libraries build the same way (`librsmqtt.a`, `librsmqtt_wincc.a`); the crates, broker tests and C++ examples are tested on Debian 12.

## Driver usage

### Installing into a project

[driver/project](driver/project) has the same layout as a WinCC OA project. Copy it over the project:

| File | Purpose |
|---|---|
| `dplist/rsmqtt_types.asc` | DPTs `_RsMqttDriver` and `_RsMqttBroker`; import with the ASCII manager (`WCCOAascii -in rsmqtt_types.asc`) |
| `dplist/rsmqtt_example.asc` | optional: `_RsMqttDriver_2` and an empty broker `_RsMqtt_plant` |
| `scripts/libs/rsmqtt.ctl` | CTRL library: create, change and delete brokers; set addresses |
| `scripts/libs/rsmqttService.ctl` | CTRL library: callbacks for MQTT topics and publishing, through the driver's service (see [CTRL callbacks for topics](#ctrl-callbacks-for-topics-manager-service-api)) |
| `scripts/userDrivers.ctl` | registers "MQTT (rsmqtt)" in the PARA driver list |
| `scripts/userPara.ctl` | PARA address panel functions for the `rsmqtt` type |
| `panels/para/address_rsmqtt.pnl` | PARA address panel (generated by [driver/tools/gen_panels.py](driver/tools/gen_panels.py)) |
| `panels/para/rsmqtt_brokers.pnl` | broker panel: create, edit, reconnect and delete brokers, with live state (same generator) |
| `config/config.rsmqtt.example` | optional `[rsmqtt]` section for the config file |
| `config/config.redu` | redundant projects only: copies the configuration to the second host's DPs (merge into an existing `config.redu`) |

A project has only one `userDrivers.ctl` and one `userPara.ctl`. If yours already registers other drivers, merge the `rsmqtt` entries into them instead of overwriting.

Then add `WCCOArsmqtt -num <n>` in the console.

### Brokers as datapoints

Every broker is a DP `_RsMqtt_<broker>` of type `_RsMqttBroker`. The driver serves the brokers listed in `_RsMqttDriver_<n>.Brokers`. Changes take effect immediately, because the driver re-creates the client.

The easiest way to manage brokers is the broker panel `para/rsmqtt_brokers.pnl`. Open it with the **…** button next to the broker field of the address panel, or from a panel script of your own. Its Close button returns the selected broker with `PanelOffReturn()`, so open it with a `...Return` function:

```
dyn_float df;
dyn_string ds;
ChildPanelOnCentralModalReturn("para/rsmqtt_brokers.pnl", "MQTT brokers",
                               makeDynString("$broker:plant"), df, ds);
// df[1] == 1: closed with Close; ds[1] is the broker selected at that time
```

- The table on the left lists all brokers with driver, state (in colour), messages in/out and last error, updated every second; a click shows the broker in the form. Or pick it from the list, or press **New** and type a name. The form covers every `Config.*` element.
- **Copy** turns the shown broker into a new one (`<name>_copy`) with the same settings; the client ID and the Sparkplug host IDs are cleared, as two brokers must not share them, and the stored password is taken over on **Save**.
- Picking another broker, **New** and **Close** ask before unsaved changes are lost.
- The Sparkplug B section sets `sparkplug.hostId` and `redu.sparkplug.hostId` (the options are kept there, not in the options field) and shows the STATE topics.
- The list next to the options field offers every rsmqtt option without a field of its own (MQTT 5 properties, `auth.method=SCRAM-…`, buffering and persistence, TLS, last will, `redu.*`), each with an example value: picking one adds it to the field, where its value is adjusted before **Save**. An option that is already set is not added twice (except `userProperty` / `httpHeader`).
- **Save** checks the input, creates the DP if needed and adds the broker to the selected driver's `Brokers` list (removing it from any other driver's).
- **Reconnect** and **Delete** (with confirmation) act on the selected broker. A deleted broker can be created again under the same name while the driver runs. If you delete and re-create a broker DP by hand instead, write `_RsMqttDriver_<n>.Brokers` again (or restart the driver) so the driver takes over the new DP. To delete one by hand, remove it from `Brokers` first and delete the DP a second later (as `rsmqttDeleteBroker()` does): the driver lets go of a broker's DP once it leaves the list, while a DP deleted while its broker is still listed makes WinCC OA log one warning 107 ("Automatic disconnect") per element.
- The state line updates every second: connection state in colour, last error, URI, client ID and message counters.
- Texts and messages follow the UI language: German, otherwise English (also the state line of the address panel).

From CTRL:

```
#uses "rsmqtt"
rsmqttCreateBroker("plant", 2, "tcp://broker.local:1883");
rsmqttCreateBroker("cloud", 2, "mqtts://mqtt.example.com:8883",
                   makeMapping("Username", "winccoa", "Password", "secret",
                               "TlsTrustStore", "C:/certs/ca.pem", "TlsVerify", TRUE));
```

| Element | Meaning |
|---|---|
| `Config.Active` | enables the connection |
| `Config.ServerURI`, `Config.ServerURIs` | broker URI; optional fail-over list |
| `Config.ClientId`, `MQTTVersion` (0/3/4/5), `KeepAlive` (s), `CleanSession` | session settings |
| `Config.Username`, `Config.Password` | credentials |
| `Config.TlsTrustStore`, `TlsKeyStore`, `TlsPrivateKey` | TLS: CA certificates (PEM; leave empty for publicly trusted broker certificates, the Windows/system store is used), client certificate and key for mutual TLS |
| `Config.TlsVerify` | TLS: TRUE checks that the certificate names the host in the URI; FALSE skips that check (e.g. a broker reached by IP address), the certificate chain is still checked. Brokers created with the panel or `rsmqttCreateBroker()` start with TRUE; a DP created by hand starts with FALSE |
| `Config.WillTopic`, `WillPayload`, `WillQoS`, `WillRetain` | last will |
| `Config.SubscribeQoS`, `PublishQoS`, `PublishRetain` | defaults for addresses without `qos=` / `retain` |
| `Config.Options` | any further rsmqtt option as `name=value`, e.g. `sessionExpiryInterval=3600`, `ssl.alpn=mqtt` |
| `Command.Reconnect` | set to TRUE to drop and re-open the connection |
| `State.ConnState` | 0 disabled, 1 disconnected, 2 connecting, 3 connected, 4 failure |
| `State.LastError`, `ServerURI`, `ClientId`, `MessagesIn`, `MessagesOut` | diagnostics |

#### Encrypted passwords

`Config.Password`, a `password=...` or `auth.data=...` entry of `Config.Options` and `password` in the config file can hold a password encrypted with `_DriverSecurity.PublicKey`. The driver decrypts it with `config/driver_private.key`, which it looks for in the project, then the sub projects, then the installation. Plain text passwords still work.

- `rsmqttCreateBroker()` / `rsmqttSetBrokerConfig()` and the broker panel encrypt a plain `Password` automatically. The panel shows `*****` for a stored password. If `_DriverSecurity.PublicKey` is empty, the password is stored unencrypted, with a warning in the log.
- `rsmqttEncryptPassword("secret")` returns the encrypted text, e.g. for the config file. It uses CTRL `encrypt(..., "ENVELOPE_RSA_AES_256_CBC")` and works in every manager.
- Values from the UI function `drvsSecSetPassword()` (driverSettings.ctl), which the panels of the WinCC OA drivers use, are accepted too.
- If an encrypted password cannot be decrypted, for example because the key is missing or does not match, the broker goes to `State.ConnState` 4 (failure) with the reason in `State.LastError`.

Every WinCC OA installation ships the same default key pair. For real protection, generate a key pair for the project in PARA (driver security / `para/driverSec_certificate.pnl`), then re-enter the passwords.

Brokers can also be configured in the `[rsmqtt]` section of the config file ([example](driver/project/config/config.rsmqtt.example)). There, each `connection = "<name>"` line starts a block, and every rsmqtt option after it applies to that broker.

### Addresses

In PARA, choose the driver type **MQTT (rsmqtt)**. The panel offers the broker (filled from the broker DPs and editable; **…** opens the broker panel), the topic, QoS, retain, data type, direction, extra options and the JSON settings (JSON payload, JSON path, timestamp path), and it shows the broker's current state. From CTRL:

```
rsmqttSetAddress("Pump1.temp",     "plant:line1/pump1/temp",            RSMQTT_FLOAT, RSMQTT_IN,  2);
rsmqttSetAddress("Pump1.setpoint", "plant:line1/pump1/sp;qos=1;retain", RSMQTT_FLOAT, RSMQTT_OUT, 2);
rsmqttSetAddress("Pump1.online",   "plant:$state",                      RSMQTT_BOOL,  RSMQTT_IN,  2);
rsmqttSetAddress("Pump1.hum",      "plant:line1/pump1;json=data.hum;ts=data.time", RSMQTT_FLOAT, RSMQTT_IN, 2);
```

### Address syntax

`<broker>:<topic>[;qos=0|1|2][;retain[=0|1]][;nolocal][;rap][;rh=0|1|2][;json[=<path>]][;ts=<path>][;merge]`

`<broker>:$spb/<group>/<edge node>[/<device>];metric=<name>|;online` (Sparkplug B, see below)

- Input addresses subscribe. Filters with `+` / `#` and `$share/<group>/…` work too, and every matching message updates the DPE. A filter must be valid MQTT, or the address is refused: `#` only as the last level, `+` only as a whole level, `$share/<group>/<filter>` with a group without wildcards and a filter after it.
- Output addresses publish when the DPE changes.
- `$state` is the connection state: 1 while connected, otherwise 0.
- After every (re)connect the driver subscribes again, so the broker resends retained values. That works like a general query.

### JSON payloads

`json=<path>` says that the payload is JSON and where the value is; `json` without a path means the whole payload.

| Path | Meaning |
|---|---|
| `temp`, `data.temp` | member of the top-level object, nested member |
| `values[0]`, `data.list[1].x` | array element (index from 0), also nested |
| `[2]` | element of a top-level array |
| `['a.b']`, `["a.b"]` | member whose name contains `.`, `[` or `]` |
| `$.data.temp` | a leading `$` (JSONPath style) is optional |

- Input: several DPEs can read one topic, each with its own path. The driver reads a message once for all `json=` and `ts=` paths of its topic, so many addresses on one message cost little more than one (1000 addresses on a 64 KB message: about 0.16 ms per message). Strings are unescaped, numbers and `true`/`false` are converted to the data type, objects and arrays reach a string DPE as JSON text. A payload that is not JSON or has nothing at the path leaves the DPE unchanged and is logged once until a payload fits again; `null` leaves it unchanged too.
- Output: the value is published nested along its path. Numbers and bool become JSON numbers and `true`/`false`, string and blob DPEs JSON strings: `json=data.value` publishes `{"data":{"value":12.5}}`, `json=values[1]` publishes `{"values":[null,true]}`. Each DPE publishes its own message, unless it has `merge` (below).
- `ts=<path>` adds the timestamp. Input: the value at that path becomes the source time (`_original.._stime`); ISO 8601 (`2026-11-23T10:00:00.123Z`, `+01:00`; without an offset UTC) or a number of seconds since 1970 (milliseconds when it is 1e11 or more). Output: the DPE's time is added as ISO 8601 in UTC, e.g. `json=data.value;ts=data.time` publishes `{"data":{"value":12.5,"time":"2026-11-23T10:00:00.123Z"}}`. WinCC OA never stamps a value earlier than the current value of the DPE, so an older source time is moved up to that time.

#### Several DPEs in one message (`merge`)

Output addresses with `merge` on the same topic of a broker publish one JSON document together:

```
plant:line1/status;json=temp;ts=time;merge
plant:line1/status;json=info.mode;merge
plant:line1/status;json=info.on;merge
```

publish `{"temp":12.5,"time":"2026-11-24T10:00:00.123Z","info":{"mode":"auto","on":true}}`.

- The driver collects the writes of one round of its work loop and then publishes each changed document once, so a `dpSet` of several DPEs gives one message. Every message holds the last value of every member, also when only one of them changed; a member that has not been written since the driver started (or since its address was set) is left out.
- Members are written into the document in the order the driver received their addresses.
- An address is refused if its `json=` / `ts=` paths collide with those of another member (one the parent of the other, or a level used as object and as array), and if its `qos=` / `retain=` differ from the other members' (leave them out for the broker defaults, or give all members the same).
- `merge` needs `json=<path>` with a path and a topic without wildcards; it does not apply to `$state`, `$spb` and `$share/` topics. Addresses without `merge` on the same topic still publish on their own. An input/output address with `merge` reads its value at its path from the documents it receives, as any `json=` input does.
- In the address panel, enter `merge` in the options field.
- Redundancy: the passive host does not see the writes, so after a switchover the new active host publishes documents that hold only the members written since then.

### Data types (`_address.._datatype`)

| Value | Type | Payload |
|---|---|---|
| 1001 (and 1000/0) | string | UTF-8 text |
| 1002 / 1003 | int32 / uint32 | decimal text (`"12"`, `"12.0"`, `"true"`) |
| 1004 / 1005 | int64 / uint64 | decimal text |
| 1006 | float (double) | decimal text; published in its shortest round-trip form |
| 1007 | bool | `true/false`, `1/0`, `on/off`, `yes/no`; published as `true`/`false` |
| 1008 | blob | raw bytes |

dyn elements (`dyn_string`, `dyn_int`, `dyn_uint`, `dyn_long`, `dyn_ulong`, `dyn_float`, `dyn_bool`, `dyn_blob`) work with the same data types 1001..1008, which then give the type of the items; the driver recognizes the dyn element by itself.

- Input: a JSON array gives one item per array element (`[1.5, 2, -3e2]`, `["a", "b"]`; for a `dyn_string` numbers become their text and objects their JSON text). A single value gives one item, JSON (`42.5`, `"text"`) or not (`hello`). An empty payload or `[]` empties the element. If an item cannot be converted, or is `null`, the element keeps its value and the driver logs a warning.
- Output: always a JSON array, e.g. `[1.5,2,-300]` or `["a","b\"c"]`; an empty element publishes `[]`.
- `dyn_blob`: JSON has no bytes, so the items are base64 strings (RFC 4648, padding optional on input): `["Zm9v","AP/+"]` both ways, and a single JSON string is one item. A payload that is no JSON array, string or `null` (e.g. raw binary data) is one item with its bytes as they are. Sparkplug `Bytes` / `File` metrics belong on a `blob` element, not a `dyn_blob`.
- With `json=<path>` the array is taken from, or published at, that path: `json=data.values` reads `{"data":{"values":[1,2,3]}}`. A JSON string at the path is one item, even if its text looks like an array.

Driver defaults for every connection, all of which can be overridden: `automaticReconnect=1`, `sendWhileDisconnected=1`, `maxBufferedMessages=10000`, `deleteOldestMessages=1`, and only failed operations are reported (`completionEvents=errors`).

Where options behave differently from Paho C (paho.mqtt.rust with rustls):

- `persistence=file` stores QoS 1/2 publishes that are in flight (sent, not yet acknowledged); after a restart of the driver they are sent again. Values written while the connection is already down wait in the send-while-disconnected buffer, which is kept in memory and is lost when the process ends. The driver keeps the files in `<project>/data/rsmqtt`; `persistenceDir` sets another folder, a relative one under `<project>/data`. `persistenceDir` alone selects file persistence; an explicit `persistence=none|memory` wins.
- `maxBufferedMessages` is at least 4; smaller values count as 4.
- `ssl.sslVersion` is a minimum. rustls has no TLS 1.0/1.1, so `1`/`2` still negotiate TLS 1.2 or 1.3.
- `ssl.enabledCipherSuites` takes rustls names (`TLS13_AES_256_GCM_SHA384`, `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`, ...).
- Encrypted private keys (`ssl.privateKeyPassword`) are not supported; the connection fails with a clear error.
- `httpProxy` / `httpsProxy` apply to `ws://` / `wss://` only; `tcp://` and `ssl://` connect directly.
- MQTT 5: `CleanSession=FALSE` keeps the session only together with `sessionExpiryInterval=<s>` in `Config.Options` (the expiry defaults to 0, so the session ends with the connection).
- MQTT 5 enhanced authentication: `auth.method=SCRAM-SHA-256` (or `SCRAM-SHA-1`, `SCRAM-SHA-512`) in `Config.Options` authenticates with `Config.Username` / `Config.Password` by SCRAM; the password never goes to the broker. A broker that cannot prove it knows the password (wrong server signature) is disconnected, and the driver does not retry it on its own: `State.LastError` says why, and a change of `Config` connects again. Methods other than SCRAM need a C callback and are not available in the driver.

### Sparkplug B

The driver is a Sparkplug B (3.0) Host Application: it receives the data of edge nodes and their devices and writes to them with commands.

```
rsmqttSetAddress("Line1.temp",   "plant:$spb/Factory/Line1;metric=temp",               RSMQTT_FLOAT, RSMQTT_IN,  2);
rsmqttSetAddress("PLC7.speed",   "plant:$spb/Factory/Line1/PLC7;metric=Motor/speed",   RSMQTT_INT32, RSMQTT_IO,  2);
rsmqttSetAddress("PLC7.online",  "plant:$spb/Factory/Line1/PLC7;online",               RSMQTT_BOOL,  RSMQTT_IN,  2);
```

- `$spb/<group>/<edge node>` addresses the edge node's own metrics, `$spb/<group>/<edge node>/<device>` a device. `metric=<name>` names the metric (names with `/` are fine, `;` is not); `online` is 1 between birth and death, else 0.
- For each edge node the driver subscribes `spBv1.0/<group>/+/<edge node>/#` and decodes the protobuf payloads: NBIRTH/DBIRTH (values, data types, aliases), NDATA/DDATA (also by alias only), NDEATH/DDEATH. Values keep the metric's timestamp as source time.
- After a death, or when the connection to the broker is lost, the metrics keep their last value with the invalid bit set until the next birth. A rebirth that no longer contains a metric sets that metric invalid as well. An NDEATH whose `bdSeq` does not match the current birth (a late will of an earlier session) is ignored.
- Signed Int8 and Int16 values are read in their own width, so both encodings in use work (Int8 -23 as `0xFFFFFFE9` or, as Tahu does it, `0xE9`).
- Normal addresses can watch the same topics, e.g. `plant:spBv1.0/STATE/+` for the host states or the edge node's `spBv1.0/<group>/+/<edge node>/#` for the raw payloads.
- If the driver misses something (data before the birth, a gap in `seq`, an unknown alias or metric), it asks the edge node for a rebirth (NCMD `Node Control/Rebirth`, at most every 5 s per edge node).
- Output and input/output addresses write with NCMD (edge node) or DCMD (device), with the data type from the birth; before the birth a write fails with a log entry.
- Data types: integers, Float, Double, Boolean, String/Text/UUID, DateTime (milliseconds since 1970), Bytes/File (blob) and all array types (JSON arrays, for dyn DPEs). DataSet and Template metrics are read as JSON text (`{"columns":[...],"types":[...],"rows":[[...]]}`, and an object of the template's member metrics); they can't be written. PropertySets are metadata and are skipped, as are historical metrics.
- `sparkplug.hostId=<id>` in `Config.Options` (or `sparkplug.hostId = "<id>"` in the config file) makes the driver the primary host application: after connecting it publishes `spBv1.0/STATE/<id>` `{"online":true,"timestamp":...}` (retained, QoS 1), with `{"online":false,...}` as will and before a clean disconnect, so that edge nodes following that host rebirth when it comes back. This will replaces a configured will (`WillTopic` or `will.*`); the driver logs a warning then. Sparkplug expects a clean session for the host (`CleanSession` TRUE, the default). Writes reject values that don't fit the metric (`NaN`, infinity, a Float out of range, a NUL character inside a StringArray item).
- [tests/broker/spb_edge.py](tests/broker/spb_edge.py) simulates an edge node for the tests (no third-party modules).

### Redundancy

In a redundant project the driver works like the WinCC OA drivers:

- Every DP exists twice. Host 1 uses `_RsMqttDriver_<num>` and `_RsMqtt_<broker>`, host 2 the twins `_RsMqttDriver_<num>_2` and `_RsMqtt_<broker>_2`. Addresses and the broker list keep the plain broker names.
- Panels and `rsmqttCreateBroker()` write the configuration to the host-1 DPs and create the twins first. The event manager copies `Config`, `Command` and `Brokers` to the twins and forwards what each driver reports (`State`, `Version`) to the other system; that is what [config/config.redu](driver/project/config/config.redu) sets up. For a project that becomes redundant later, `rsmqttUpgradeToRedundant()` creates the missing twins with their values.
- Both drivers connect and receive; outputs are written by the active host only (ComDrv discards them on the passive one, unless `passiveDriverWrites` is set).
- The two hosts must not share an MQTT client id, or the broker would disconnect one whenever the other connects. Host 2 connects with the configured or default client id plus `_2` (`WCCOArsmqtt_3_plant_2`); `State.ClientId` of each twin shows the one in use.
- `redu.<option>=<value>` in `Config.Options` or in the config file applies to host 2 only and wins over the other settings, e.g. `redu.serverURI=tcp://broker-b:1883`, `redu.clientId=scada-b` (used as it is, without `_2`) or `redu.username=...`. Host 1 ignores these entries but checks the option names.
- Sparkplug B: only host 1 is the primary host application of `sparkplug.hostId`. Two drivers publishing one STATE would each have their own will and timestamp, and edge nodes ignore a STATE older than the last one, so one host's death could turn the host offline for good. Host 2 receives and writes, and asks for rebirths, but publishes no STATE unless `redu.sparkplug.hostId=<id>` gives it a host id of its own.
- New broker names must not end with `_2`; in a redundant project the driver ignores such names in the broker list.
- The broker panel and the address panel show the active host's state and add the other host's, e.g. `host 1: connected | host 2: connected`.

### CTRL callbacks for topics (Manager Service API)

The driver hosts the vRPC service `RsMqtt` of the WinCC OA 3.21 Manager Service API (MSA), as a manager-specific service of `WCCOArsmqtt -num <n>`. With [rsmqttService.ctl](driver/project/scripts/libs/rsmqttService.ctl) any CTRL manager can have a function called for the messages of a topic, and publish, over the driver's broker connections and without datapoints:

```
#uses "rsmqttService"

void onCommand(string topic, string payload, mapping info)
{
  DebugTN(info["broker"], topic, payload, info["retained"]);
  rsmqttPublish("plant", "line1/pump1/ack", payload);
}

main()
{
  int h = rsmqttTopicConnect("onCommand", "plant", "line1/+/cmd");
  // ...
  rsmqttTopicDisconnect(h);
}
```

- `rsmqttTopicConnect(callback, broker, topic, options)` returns a handle (> 0) or -1 (the reason is logged). `callback` is a function name or `function_ptr` with `(string topic, anytype payload, mapping info)`; `info` has `broker`, `driver`, `qos`, `retained`, `binary`, `time` (receive time), `handle` and `dropped`. `topic` may be a filter with `+`, `#` or `$share/<group>/`; an invalid filter (e.g. `a/#/b`) is refused. Options: `qos` (default `Config.SubscribeQoS`), `binary` (every payload as `blob`), `driver` (driver number; by default the driver whose `Brokers` list has the broker; needed for brokers from the config file).
- The payload is a string, or a blob if `binary` is set or the payload is no valid UTF-8 text or contains NUL bytes (`info["binary"]` is then TRUE). Declare it `string` only for topics that carry text.
- The callback runs in a thread of the script that called `rsmqttTopicConnect()`, one message after the other in the order they arrived. An error in the callback is logged and the next message is delivered. Retained messages arrive when the subscription starts, with `info["retained"] == TRUE`: every new subscription sends SUBSCRIBE, also for a filter the driver already uses, so addresses and other callbacks on that filter get its retained messages again.
- `rsmqttTopicDisconnect(handle)` ends a subscription, `rsmqttTopicHandles()` lists those of the manager.
- `rsmqttPublish(broker, topic, payload, qos = -1, retain = -1, driverNum = 0)` publishes a string or blob (-1: the broker's `PublishQoS` / `PublishRetain`); 0 means the MQTT client took the message, -1 an error (logged). Like the driver's outputs, payloads above `maxPayload` are refused, and so is publishing on the passive host of a redundant pair (unless `passiveDriverWrites` is set). The broker's driver number is cached for 30 s; in a callback, `info["driver"]` can be passed.
- If the driver is not running, `rsmqttTopicConnect()` and `rsmqttPublish()` fail with status 14 ("Service is not available"). If the driver restarts, it resumes the subscriptions: it keeps them in `<project>/data/rsmqtt/service_<num>.state` and makes them again with their ids as soon as their broker is configured, and the callbacks get the messages queued since. Messages published while the driver is down reach it only if the broker keeps them: a persistent session (`CleanSession` FALSE, with MQTT 5 also `sessionExpiryInterval`; the default client id stays the same across restarts) and QoS 1 or 2. A restored subscription that no manager resumes within 120 s is dropped (a `rsmqttTopicDisconnect()` whose call fails, e.g. while the driver is down, keeps trying to unsubscribe for 5 minutes); if the state file is missing or cannot be written, the library subscribes again and messages that arrive in between are lost.

How it works: vRPC in 3.21 has request/response functions only (`Subscribe`, `Receive`, `Renew`, `Unsubscribe`, `Publish`, each with a mapping). The driver adds the filter to its connection (shared with the addresses on the same filter) and queues matching messages; a thread of the library calls `Receive`, which the driver holds until messages arrive or 10 s pass (long poll). Up to 10000 messages or 64 MB of payload are queued per subscription; beyond that the oldest are dropped and `info["dropped"]` of the next message says how many. A subscription without a `Receive` or `Renew` for 120 s is dropped (with a warning in the log that counts the messages lost), so a manager that ends without disconnecting leaves nothing behind. While callbacks work through a batch of messages, the library calls `Renew` every 30 s, so one callback call must not take longer than 2 minutes. Your own client can call the service with `VrpcStub` (`classes/msa/vrpc`), see [RsMqttService.hxx](driver/RsMqttService.hxx).

Notes:

- Overlapping filters on one broker (e.g. `a/#` and `a/b`, also an address and a callback) make brokers like mosquitto send a message once per filter, back to back. Callbacks get it once: the driver skips identical messages (topic, payload, retained flag) that follow within 100 ms, up to the number of the broker's filters that match the topic; the same value published again later is a new message, also with brokers that send only one copy. `$share/` filters are not counted (the broker may or may not send them a copy), so a callback overlapping a shared subscription can see a message twice. Addresses still get every copy.
- Any CTRL manager of the system can publish to every broker of the driver this way. `serviceApi = 0` in the `[rsmqtt]` section turns the service off.
- Redundancy: both drivers register the service; which host's driver answers is up to MSA. If NotFound answers repeat (e.g. calls reaching different drivers), the library waits 2 s between attempts. Not tested in a redundant system.
- Functions defined in an `evalScript()` script (such as the WinCC OA MCP `ctrl-eval`) cannot be callbacks; put them in a library.

[tests/winccoa/service_demo.ctl](tests/winccoa/service_demo.ctl) is a small example to run as a CTRL manager (`WCCOActrl -num 97 service_demo.ctl`, copied to the project's `scripts`): it subscribes to `plant:demo/#`, logs every message, answers it on `demo_ack/<rest>` and publishes a counter on `demo/tick` every 5 s. A second subscription on `demo_q2/#` with QoS 2 checks every 5 s that its own numbered QoS 2 messages arrive once, in order and with `qos` 2 (`service_demo: QoS 2 PASS` in the manager's log).

[tests/winccoa/service_lease_test.ctl](tests/winccoa/service_lease_test.ctl) checks the lease (run it the same way, `WCCOActrl -num 96 service_lease_test.ctl`, about 5 minutes): a batch of 700 messages whose callbacks take 0.2 s each (about 140 s) arrives completely and in order, because the library renews the subscription; and a single callback call of 130 s loses the messages published meanwhile, after which the subscription is made again and new messages arrive. Results are `service_lease_test: PASS/FAIL` lines in the manager's log.

## Tests

```powershell
docker run -d --name rsmqtt-mosq -p 18830:1883 -p 18831:9001 -v "${PWD}\tests\mosquitto.conf:/mosquitto/config/mosquitto.conf" eclipse-mosquitto:2
```

```powershell
$env:RSMQTT_TEST_URI="tcp://localhost:18830"; $env:RSMQTT_TEST_WS_URI="ws://localhost:18831"; cargo test
```

The broker tests cover v3.1, v3.1.1 and v5 round trips at QoS 0–2, WebSocket, TLS (when `RSMQTT_TEST_TLS_URI` and `RSMQTT_TEST_CA` are set), v5 properties and subscription options, will messages, retained messages, offline buffering and error codes. SCRAM (built in and through the callback) needs a broker with SCRAM authentication, e.g. EMQX:

```powershell
docker run -d --name rsmqtt-emqx -p 18860:1883 -p 18868:18083 -e EMQX_AUTHENTICATION__1__MECHANISM=scram -e EMQX_AUTHENTICATION__1__BACKEND=built_in_database -e EMQX_AUTHENTICATION__1__ALGORITHM=sha256 emqx/emqx:5.8.6
```

Add a user in the dashboard (http://localhost:18868, admin / public: Access Control, Authentication, Users) or with its REST API, then:

```powershell
$env:RSMQTT_TEST_SCRAM_URI="tcp://localhost:18860"; $env:RSMQTT_TEST_SCRAM_USER="scramuser"; $env:RSMQTT_TEST_SCRAM_PASSWORD="<password>"; cargo test -p rsmqtt --test broker scram auth_callback
``` `cargo test -p rsmqtt-wincc` tests the driver logic (addresses, JSON, dyn lists, redundancy names, Sparkplug decoding and commands, the state file of the MSA service) without a broker. [examples/cpp](examples/cpp) has a C++ publish/subscribe example and `wincc_test`, which checks that values, strings with NUL bytes, errors and handles cross the `rsmqtt_wincc` C ABI.

### Connection option tests in WinCC OA

[tests/winccoa/option_tests.ctl](tests/winccoa/option_tests.ctl) runs the driver (number 3) against a local mosquitto with every listener type, one group at a time:

```powershell
powershell -File tests/broker/start-broker.ps1
```

```powershell
python tests/broker/netutil.py
```

[start-broker.ps1](tests/broker/start-broker.ps1) creates test certificates ([make-certs.sh](tests/broker/make-certs.sh), Git Bash) and starts `rsmqtt-mosq-full` with MQTT, WebSocket, TLS, WSS, TLS with client certificates and a password listener on ports 18840–18845. [netutil.py](tests/broker/netutil.py) provides HTTP CONNECT proxies (18850, 18851 with credentials), a server that never answers (18852) and a forwarder that can be frozen (18853). Run a group from any CTRL manager, e.g. with the WinCC OA MCP `ctrl-eval`:

```
string src; fileToString("C:/Repos/rust_mqtt/tests/winccoa/option_tests.ctl", src);
anytype res; evalScript(res, src, makeDynString(), "tls"); return res;
```

| Group | Covers |
|---|---|
| `transport` | tcp, mqtt, ws (with path), ssl, tls, mqtts, wss; MQTT 3.1, 3.1.1, 5 |
| `auth` | user name and password (also via `Config.Options`), wrong and missing credentials |
| `tls` | CA file and directory, foreign CA, system store, host name check (`TlsVerify`), `enableServerCertAuth`, `disableDefaultTrustStore`, mutual TLS (separate and combined PEM), encrypted key, cipher suites, TLS versions, ALPN |
| `ws` | `httpHeader`, `httpProxy` / `httpsProxy` with and without credentials |
| `connect` | `connectTimeout`, `ServerURIs` fail-over, empty client id, keep alive, runtime/event options, `maxInflight`, unknown option, bad value |
| `v5` | CONNECT properties, `maximumPacketSize` |
| `spb` | Sparkplug B: STATE and will, NBIRTH/DBIRTH with all value kinds, NDATA by alias, DDATA, seq gap -> rebirth, NCMD/DCMD writes, DDEATH/NDEATH (bdSeq) -> invalid, rebirth -> valid |
| `redu` | redundancy on host 1: `redu.*` options checked but not applied, unknown `redu.*` option |
| `dyn` | dyn_float / int / long / string / bool from JSON arrays and single values, `[]`, bad items, arrays at a JSON path, JSON array output; dyn_blob as base64 arrays and strings (also at a JSON path), raw payloads and output |
| `json` | `json=` paths (nested, array, quoted name, whole payload), string escapes, objects as text, `ts=` as ISO 8601 and epoch ms, non-JSON payloads, JSON output with timestamp |
| `payload` | plain string (1 to `maxPayload` characters) and blob outputs of many sizes published intact, a string above `maxPayload` refused |
| `merge` | `merge` outputs: three DPEs in one `dpSet` give one document, a single change publishes the whole document again, `maxPayload` applies to the document (75 KB refused, 30 KB published) |
| `session` | clean session / clean start with and without session expiry |
| `buffer` | keep alive detects a hung broker, automatic reconnect, `maxBufferedMessages` + `deleteOldestMessages` |
| `will_arm`, `will_check` | will with retain, QoS 1 and v5 properties; kill `WCCOArsmqtt` between the two |
| `persist_arm`, `persist_down`, `persist_check` | `persistence=file`: kill the driver after `persist_arm`, run `persist_down`, start the driver, then `persist_check` |
| `host2_prepare`, `host2`, `host2_check` | the second redundant host without a redundant system (see below): twins, client id `_2`, Sparkplug STATE only with `redu.sparkplug.hostId` (online, set online again, offline, will), decoding and rebirths on host 2 |

Each group returns `PASS`/`FAIL`/`INFO` lines and removes its broker DPs afterwards (`json:keep`, `dyn:keep` and `spb:keep` leave the test DPs, their addresses and the broker in place; the other groups ignore `:keep`).

A redundant system needs a license, so the driver can act as a host of a redundant pair on a single system for tests: `simulateReduHost = 2` (or `1`) in the `[rsmqtt]` section makes it use the `_2` twins and the host-2 rules, and it logs a warning at startup. Sequence: run `host2_prepare` (creates `_RsMqttDriver_<num>_2`), add `simulateReduHost = 2` and restart the driver, run `host2`, kill `WCCOArsmqtt`, run `host2_check` (checks the will and removes the twins), remove the entry and start the driver. Not covered: the event manager copying the configuration to the twins and the switch between active and passive host.

[tests/winccoa/redu_lib_tests.ctl](tests/winccoa/redu_lib_tests.ctl) checks the redundancy functions of `rsmqtt.ctl` (twins, lists, status line, `rsmqttUpgradeToRedundant()`), once as a single system and once with `isRedundant()` replaced by `TRUE`; the file header shows how to run it.

### Stress tests

[crates/rsmqtt/tests/stress.rs](crates/rsmqtt/tests/stress.rs) checks message counts, duplicates and per-topic order under load. Run it in release mode against the broker above; `RSMQTT_STRESS_SCALE` multiplies every count:

```powershell
$env:RSMQTT_TEST_URI="tcp://localhost:18830"; $env:RSMQTT_STRESS_SCALE="10"; cargo test --release -p rsmqtt --test stress -- --test-threads=1 --nocapture
```

The load generator [crates/rsmqtt/examples/stress.rs](crates/rsmqtt/examples/stress.rs) publishes or counts messages, for example to flood a WinCC OA project:

```powershell
cargo run --release --example stress -- pub --uri tcp://localhost:18830 --topic "stress/in/{i}" --topics 500 --rounds 2000 --qos 1
```

```powershell
cargo run --release --example stress -- sub --uri tcp://localhost:18830 --filter "stress/out/#" --expect 10000
```

Results on one desktop PC (local mosquitto in Docker; WinCC OA 3.21 DemoApplication, demo license):

| Scenario | Result |
|---|---|
| Library, QoS 0, 500k msgs | 763k msg/s published, nothing lost |
| Library, QoS 1, 300k msgs | 267k msg/s, no loss, in order |
| Library, QoS 2, 15k msgs | 21k msg/s, exactly once, in order |
| Library, 100k topics under one `#` filter | 227k msg/s, every topic delivered |
| Library, 8 threads on one client, 400k msgs | 274k msg/s, each thread's order kept |
| Library, 2000 × 256 KiB payloads | 659 MiB/s, content intact |
| Library, `maxQueuedEvents=1000`, 200k msgs not polled | queue stays at 1000, the newest are kept, the rest are counted as dropped |
| Driver, 1M msgs on 500 input DPEs as fast as possible | about 190k–270k msg/s through the driver; every DPE ends at the last value |
| Driver → Event Manager, default output queue (about 1000 values) | no loss up to 60k value changes/s; from 80k/s the queue overflows and WinCC OA keeps only the newest value per DPE (`_Driver<n>.OV` = TRUE) |
| Driver → Event Manager, `maxOutputQueueSize = 1000000` | no overflow at 80k, 100k and 150k msg/s steady, nor with a 1M burst; all values in WinCC OA when the generator finished; driver peak memory 33 MB |
| WinCC OA → driver → broker, 10k output values | all 10k published within 0.4 s |

Set WinCC OA's driver output queue in the `[rsmqtt]` section of the project config (see the [example](driver/project/config/config.rsmqtt.example)): `maxOutputQueueSize = 1000000`. The default overflows on short spikes; the large value costs memory (about 100 bytes per value) only while a burst is being processed.

**Known issue:** QoS 2 *publishing* slows down disproportionately when tens of thousands of QoS 2 messages are queued at once (15k: 21k msg/s, 150k: 2.7k msg/s). It sits in the outgoing QoS 2 handling of paho.mqtt.rust; QoS 0/1, receiving, and normal QoS 2 rates are not affected.

### Compared with the WinCC OA MQTT driver

The same load through `WCCOArsmqtt` and through the MQTT driver that ships with WinCC OA 3.21 (`WCCOAmqtt`), on the same PC and broker as above. Both drivers ran in the project at the same time, each with its own 500 input DPEs on its own topics, and were measured one after the other: the driver not under test received nothing. The [stress example](crates/rsmqtt/examples/stress.rs) published 500 topics × N rounds at QoS 1 (`--payload` sets the payload, e.g. `--payload '{"Value":{r}}'`).

| Scenario | `WCCOArsmqtt` | `WCCOAmqtt` | Difference |
|---|---|---|---|
| Plain number, 1M messages | 280k msg/s, 4.6 µs CPU/msg | 25k msg/s, 39 µs CPU/msg | 11× faster, 8.5× less CPU |
| Small JSON `{"Value":n}`, 500k messages | 280k msg/s, 5.1 µs CPU/msg | 25k msg/s, 40 µs CPU/msg | 11× faster, 7.8× less CPU |
| 4 KB JSON, value after a 4000 character string, 100k messages | 113k msg/s, 14.5 µs CPU/msg | 15k msg/s, 60 µs CPU/msg | 7.6× faster, 4.1× less CPU |

- **Measured:** the time from the first value change until all 500 DPEs show the last round (in WinCC OA), and the CPU time of the driver process per message; the average of two runs each, which differed by at most 5 %. `WCCOArsmqtt` was done when the generator was, so its limit is higher than these rates.
- **No loss on either side:** all 500 DPEs ended at the last value, 10 sample DPEs received every value, and no output queue overflowed (`_Driver<n>.OV`).
- **Same settings where both have them:** `maxOutputQueueSize = 1000000` in `[rsmqtt]` and `[mqtt]` (with the default queue `WCCOAmqtt` overflowed and dropped intermediate values), old/new comparison off. JSON: `json=Value` here, the JSON profile "Value only" (`_value` = `Value`) in `WCCOAmqtt`. Otherwise `WCCOAmqtt` ran with its defaults (keep alive 20 s, no persistent session); other settings may change its numbers.

## License

rsmqtt (both crates, the driver, panels, scripts and tests) is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

Third-party code keeps its own license: the libraries and the driver link [paho.mqtt.rust](https://github.com/eclipse-paho/paho.mqtt.rust) (EPL-2.0) and crates under MIT, Apache-2.0, ISC and BSD licenses (`cargo metadata` lists them), and the driver is built against the WinCC OA API, which is Siemens' and not covered by this license. Whoever ships the binaries passes on those licenses' notices.
