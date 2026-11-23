// option_tests.ctl - connection option tests of the WCCOArsmqtt driver.
//
// Every case creates a broker DP (_RsMqttBroker) with the options under test,
// assigns it to driver DRV, waits for State.ConnState and - where a
// connection is expected - sends a value through it: RsMqttOpt1.out
// (output address) -> broker -> RsMqttOpt1.in (input address).
//
// Needs: tests/broker/start-broker.ps1 (broker on 18840..18845),
// tests/broker/netutil.py (proxies 18850/18851, silent port 18852),
// WCCOArsmqtt -num DRV running. Run a group from any CTRL manager:
//
//   string src; fileToString("<repo>/tests/winccoa/option_tests.ctl", src);
//   anytype res; evalScript(res, src, makeDynString(), "transport");
//
// Groups: transport, auth, tls, ws, connect, v5, session, buffer,
//         will_arm / will_check, persist_arm / persist_check, cleanup

const int    DRV      = 3;
const string DRV_DP   = "_RsMqttDriver_3";
const string CERTS    = "C:/Repos/rust_mqtt/tests/broker/certs/";
const string TEST_DP  = "RsMqttOpt1";
const string BROKER_CONTAINER = "rsmqtt-mosq-full";
const string FREEZE   = "C:/Repos/rust_mqtt/target/netutil.freeze"; // netutil forwarder drops all bytes while it exists

dyn_string gResults;
dyn_string gBrokers;   // created in this run, deleted by cleanup()

//------------------------------------------------------------------ helpers

string rnd(int n)
{
  string s;
  for (int i = 0; i < n; i++) { string h; sprintf(h, "%04x", rand()); s += h; }
  return s;
}

void result(string name, bool ok, string detail = "")
{
  dynAppend(gResults, (ok ? "PASS  " : "FAIL  ") + name + (detail != "" ? "  | " + detail : ""));
}

void info(string name, string detail)
{
  dynAppend(gResults, "INFO  " + name + "  | " + detail);
}

string sh(string cmd)
{
  string out;
  system(cmd, out);
  strreplace(out, "\r", "");
  return strrtrim(out, "\n");
}

void ensureTestDp()
{
  if ( dynlen(dpTypes("RsMqttOpt")) == 0 )
  {
    dyn_dyn_string n = makeDynAnytype(makeDynString("RsMqttOpt", ""), makeDynString("", "in"), makeDynString("", "out"));
    dyn_dyn_int t = makeDynAnytype(makeDynInt(DPEL_STRUCT), makeDynInt(0, DPEL_STRING), makeDynInt(0, DPEL_STRING));
    dpTypeCreate(n, t);
  }
  if ( !dpExists(TEST_DP) )
    dpCreate(TEST_DP, "RsMqttOpt");
}

/** Creates (or rewrites) broker DP "_RsMqtt_<name>" and assigns it to DRV. */
void broker(string name, mapping cfg)
{
  string dp = "_RsMqtt_" + name;
  if ( !dpExists(dp) )
  {
    dpCreate(dp, "_RsMqttBroker");
    dynAppend(gBrokers, name);
  }
  mapping c = makeMapping(
    "ServerURI", "", "ServerURIs", makeDynString(), "ClientId", "opt-" + rnd(3),
    "MQTTVersion", 5, "KeepAlive", 30, "CleanSession", TRUE, "Username", "", "Password", "",
    "TlsTrustStore", "", "TlsKeyStore", "", "TlsPrivateKey", "", "TlsVerify", TRUE,
    "WillTopic", "", "WillPayload", "", "WillQoS", 0, "WillRetain", FALSE,
    "SubscribeQoS", 1, "PublishQoS", 1, "PublishRetain", FALSE, "Options", makeDynString());
  for (int i = 1; i <= mappinglen(cfg); i++)
    c[mappingGetKey(cfg, i)] = mappingGetValue(cfg, i);

  dyn_string dpes;
  dyn_anytype vals;
  for (int i = 1; i <= mappinglen(c); i++)
  {
    if ( mappingGetKey(c, i) == "Active" ) continue;
    dynAppend(dpes, dp + ".Config." + mappingGetKey(c, i));
    vals[dynlen(dpes)] = mappingGetValue(c, i);   // indexed: keeps dyn values whole
  }
  dpSetWait(dpes, vals);
  dpSetWait(dp + ".Config.Active", mappingHasKey(cfg, "Active") ? cfg["Active"] : TRUE);

  dyn_string list;
  dpGet(DRV_DP + ".Brokers", list);
  if ( dynContains(list, name) < 1 ) dynAppend(list, name);
  dpSetWait(DRV_DP + ".Brokers", list);
}

void removeBroker(string name)
{
  dyn_string list;
  dpGet(DRV_DP + ".Brokers", list);
  int pos = dynContains(list, name);
  if ( pos > 0 ) { dynRemove(list, pos); dpSetWait(DRV_DP + ".Brokers", list); }
  delay(0, 500);
  if ( dpExists("_RsMqtt_" + name) ) dpDelete("_RsMqtt_" + name);
}

/** Waits until ConnState == want (or the time is up); returns "state|error". */
string waitState(string name, int want, float maxSec)
{
  int st; string err;
  time t0 = getCurrentTime();
  do
  {
    dpGet("_RsMqtt_" + name + ".State.ConnState", st, "_RsMqtt_" + name + ".State.LastError", err);
    if ( st == want ) break;
    delay(0, 200);
  } while ( (float)(getCurrentTime() - t0) < maxSec );
  return st + "|" + err;
}

/** Waits for an error (connection expected to fail); returns "state|error". */
string waitError(string name, float maxSec)
{
  int st; string err;
  time t0 = getCurrentTime();
  do
  {
    dpGet("_RsMqtt_" + name + ".State.ConnState", st, "_RsMqtt_" + name + ".State.LastError", err);
    if ( err != "" || st == 3 ) break;
    delay(0, 200);
  } while ( (float)(getCurrentTime() - t0) < maxSec );
  return st + "|" + err;
}

int stateOf(string s) { return (int) substr(s, 0, strpos(s, "|")); }
string errOf(string s) { return substr(s, strpos(s, "|") + 1); }

void setAddr(string dpe, string ref, int direction)
{
  int mode = direction == 2 ? DPATTR_ADDR_MODE_OUTPUT_SINGLE : DPATTR_ADDR_MODE_INPUT_SPONT;
  dpSetWait(dpe + ":_distrib.._type", DPCONFIG_DISTRIBUTION_INFO, dpe + ":_distrib.._driver", DRV);
  dpSetWait(dpe + ":_address.._type", DPCONFIG_PERIPH_ADDR_MAIN,
            dpe + ":_address.._drv_ident", "RSMQTT",
            dpe + ":_address.._reference", ref,
            dpe + ":_address.._mode", mode,
            dpe + ":_address.._datatype", 1001,
            dpe + ":_address.._active", TRUE);
}

/** out -> broker -> in through broker `name`; returns "" or the problem. */
string roundTrip(string name, string topic = "")
{
  if ( topic == "" ) topic = "opt/" + rnd(3) + "/rt";
  setAddr(TEST_DP + ".in", name + ":" + topic, 1);
  setAddr(TEST_DP + ".out", name + ":" + topic, 2);
  delay(0, 700);   // subscription in place
  string payload = "rt " + rnd(2), got;
  dpSetWait(TEST_DP + ".out", payload);
  for (int k = 0; k < 50; k++) { dpGet(TEST_DP + ".in", got); if ( got == payload ) return ""; delay(0, 100); }
  return "sent '" + payload + "', in = '" + got + "'";
}

/** Expects a working connection incl. round trip. */
void expectConnected(string test, string name, mapping cfg, float maxSec = 10)
{
  broker(name, cfg);
  string s = waitState(name, 3, maxSec);
  if ( stateOf(s) != 3 ) { result(test, FALSE, "ConnState " + s); return; }
  string rt = roundTrip(name);
  result(test, rt == "", rt);
}

/** Expects no connection; `must` (if given) must appear in State.LastError. */
void expectRefused(string test, string name, mapping cfg, string must = "", float maxSec = 10)
{
  broker(name, cfg);
  string s = waitError(name, maxSec);
  string err = errOf(s);
  bool ok = stateOf(s) != 3 && err != "" && (must == "" || strpos(strtolower(err), strtolower(must)) >= 0);
  result(test, ok, "ConnState " + stateOf(s) + ": " + err);
}

string mosqPub(string args) { return sh("docker exec " + BROKER_CONTAINER + " mosquitto_pub -p 1883 " + args); }

void cleanup()
{
  for (int i = 1; i <= dynlen(gBrokers); i++)
    removeBroker(gBrokers[i]);
  dynClear(gBrokers);
}

//------------------------------------------------------------------ groups

void transport()
{
  string ca = CERTS + "ca.crt";
  expectConnected("tcp://, MQTT 3.1",         "o_tcp31",  makeMapping("ServerURI", "tcp://localhost:18840", "MQTTVersion", 3));
  expectConnected("tcp://, MQTT 3.1.1",       "o_tcp311", makeMapping("ServerURI", "tcp://localhost:18840", "MQTTVersion", 4));
  expectConnected("tcp://, MQTT 5",           "o_tcp5",   makeMapping("ServerURI", "tcp://localhost:18840", "MQTTVersion", 5));
  expectConnected("tcp://, version default",  "o_tcpdef", makeMapping("ServerURI", "tcp://localhost:18840", "MQTTVersion", 0));
  expectConnected("mqtt://",                  "o_mqtt",   makeMapping("ServerURI", "mqtt://localhost:18840"));
  expectConnected("ws://",                    "o_ws",     makeMapping("ServerURI", "ws://localhost:18841"));
  expectConnected("ws:// with path, 3.1.1",   "o_wsp",    makeMapping("ServerURI", "ws://localhost:18841/mqtt", "MQTTVersion", 4));
  expectConnected("ssl:// + TlsTrustStore",   "o_ssl",    makeMapping("ServerURI", "ssl://localhost:18842", "TlsTrustStore", ca));
  expectConnected("tls:// + TlsTrustStore",   "o_tls",    makeMapping("ServerURI", "tls://localhost:18842", "TlsTrustStore", ca));
  expectConnected("mqtts://, MQTT 3.1.1",     "o_mqtts",  makeMapping("ServerURI", "mqtts://localhost:18842", "TlsTrustStore", ca, "MQTTVersion", 4));
  expectConnected("wss:// + TlsTrustStore",   "o_wss",    makeMapping("ServerURI", "wss://localhost:18843/mqtt", "TlsTrustStore", ca));
  cleanup();
}

void auth()
{
  string uri = "tcp://localhost:18845";
  expectConnected("Username + Password",          "o_auth",    makeMapping("ServerURI", uri, "Username", "testuser", "Password", "test pass"));
  expectConnected("Username + Password, 3.1.1",   "o_auth4",   makeMapping("ServerURI", uri, "Username", "testuser", "Password", "test pass", "MQTTVersion", 4));
  expectConnected("password via Config.Options",  "o_authopt", makeMapping("ServerURI", uri, "Username", "testuser", "Options", makeDynString("password=test pass")));
  expectRefused("wrong password",                 "o_authbad", makeMapping("ServerURI", uri, "Username", "testuser", "Password", "wrong"), "");
  expectRefused("no credentials",                 "o_authnone", makeMapping("ServerURI", uri), "");
  cleanup();
}

void tls()
{
  string ca = CERTS + "ca.crt";
  string uri = "mqtts://localhost:18842";
  expectConnected("ssl.CApath (directory)",            "o_capath",  makeMapping("ServerURI", uri, "Options", makeDynString("ssl.CApath=" + CERTS + "capath")));
  expectRefused("foreign CA refused",                  "o_otherca", makeMapping("ServerURI", uri, "TlsTrustStore", CERTS + "other-ca.crt"), "certificate");
  expectRefused("no CA: system store has no test CA",  "o_noca",    makeMapping("ServerURI", uri), "UnknownIssuer");
  expectConnected("ssl.enableServerCertAuth=0, no CA", "o_noauth",  makeMapping("ServerURI", uri, "Options", makeDynString("ssl.enableServerCertAuth=0")));
  expectRefused("TlsVerify=TRUE via 127.0.0.1",        "o_ipver",   makeMapping("ServerURI", "mqtts://127.0.0.1:18842", "TlsTrustStore", ca, "TlsVerify", TRUE), "not valid for name");
  expectConnected("TlsVerify=FALSE via 127.0.0.1",     "o_ipnover", makeMapping("ServerURI", "mqtts://127.0.0.1:18842", "TlsTrustStore", ca, "TlsVerify", FALSE));
  expectConnected("ssl.disableDefaultTrustStore=1 + CA", "o_nostore", makeMapping("ServerURI", uri, "TlsTrustStore", ca, "Options", makeDynString("ssl.disableDefaultTrustStore=1")));

  string mtls = "mqtts://localhost:18844";
  expectRefused("mutual TLS without client cert",      "o_mtls0",   makeMapping("ServerURI", mtls, "TlsTrustStore", ca), "");
  expectConnected("mutual TLS: TlsKeyStore + TlsPrivateKey", "o_mtls", makeMapping("ServerURI", mtls, "TlsTrustStore", ca,
                  "TlsKeyStore", CERTS + "client.crt", "TlsPrivateKey", CERTS + "client.key"));
  expectConnected("mutual TLS: cert + key in TlsKeyStore",   "o_mtlspem", makeMapping("ServerURI", mtls, "TlsTrustStore", ca,
                  "TlsKeyStore", CERTS + "client.pem"));
  // rustls cannot decrypt protected keys: expected to be refused with a clear message
  expectRefused("encrypted key + ssl.privateKeyPassword (not supported by rustls)", "o_mtlsenc", makeMapping("ServerURI", mtls, "TlsTrustStore", ca,
                "TlsKeyStore", CERTS + "client.crt", "TlsPrivateKey", CERTS + "client-enc.key",
                "Options", makeDynString("ssl.privateKeyPassword=secret")), "");

  expectConnected("ssl.enabledCipherSuites TLS 1.3 suite", "o_cs13", makeMapping("ServerURI", uri, "TlsTrustStore", ca, "Options", makeDynString("ssl.enabledCipherSuites=TLS13_AES_128_GCM_SHA256")));
  expectConnected("ssl.enabledCipherSuites TLS 1.2 suite", "o_cs12", makeMapping("ServerURI", uri, "TlsTrustStore", ca, "Options", makeDynString("ssl.enabledCipherSuites=TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256")));
  expectRefused("ssl.enabledCipherSuites unknown name",    "o_csbad", makeMapping("ServerURI", uri, "TlsTrustStore", ca, "Options", makeDynString("ssl.enabledCipherSuites=NO_SUCH_SUITE")), "");
  expectConnected("ssl.sslVersion=3 (TLS 1.2)",            "o_v12",   makeMapping("ServerURI", uri, "TlsTrustStore", ca, "Options", makeDynString("ssl.sslVersion=3")));
  broker("o_v10", makeMapping("ServerURI", uri, "TlsTrustStore", ca, "Options", makeDynString("ssl.sslVersion=1")));
  info("ssl.sslVersion=1 (TLS 1.0)", "ConnState " + waitError("o_v10", 8));
  expectConnected("ssl.alpn=mqtt",                         "o_alpn",  makeMapping("ServerURI", uri, "TlsTrustStore", ca, "Options", makeDynString("ssl.alpn=mqtt")));
  cleanup();
}

const string NETUTIL_LOG = "C:/Repos/rust_mqtt/target/netutil.log";   // written by tests/broker/netutil.py

int proxyLines(string match)
{
  string log;
  fileToString(NETUTIL_LOG, log);
  int n;
  for (int pos = strpos(log, match); pos >= 0; pos = strpos(log, match, pos + 1))
    n++;
  return n;
}

void ws()
{
  string ca = CERTS + "ca.crt";
  expectConnected("httpHeader",                    "o_hdr",   makeMapping("ServerURI", "ws://localhost:18841", "Options", makeDynString("httpHeader=X-Test: 1")));
  int p0 = proxyLines("CONNECT localhost:18841");
  expectConnected("ws:// through httpProxy",       "o_prx",   makeMapping("ServerURI", "ws://localhost:18841", "Options", makeDynString("httpProxy=http://127.0.0.1:18850")));
  result("  ... proxy log shows the tunnel", proxyLines("CONNECT localhost:18841") > p0);
  int p1 = proxyLines("CONNECT localhost:18843");
  expectConnected("wss:// through httpsProxy",     "o_prxs",  makeMapping("ServerURI", "wss://localhost:18843", "TlsTrustStore", ca, "Options", makeDynString("httpsProxy=http://127.0.0.1:18850")));
  result("  ... proxy log shows the tunnel", proxyLines("CONNECT localhost:18843") > p1);
  expectConnected("httpProxy with credentials",    "o_prxa",  makeMapping("ServerURI", "ws://localhost:18841", "Options", makeDynString("httpProxy=http://proxyuser:proxypass@127.0.0.1:18851")));
  expectRefused("httpProxy, wrong credentials",    "o_prxab", makeMapping("ServerURI", "ws://localhost:18841", "Options", makeDynString("httpProxy=http://proxyuser:wrong@127.0.0.1:18851")), "");
  int p2 = proxyLines("CONNECT localhost:18840");
  broker("o_prxtcp", makeMapping("ServerURI", "tcp://localhost:18840", "Options", makeDynString("httpProxy=http://127.0.0.1:18850")));
  string s = waitState("o_prxtcp", 3, 8);
  info("httpProxy with tcp:// URI", "ConnState " + s + ", through proxy: " + (proxyLines("CONNECT localhost:18840") > p2));
  cleanup();
}

void connect()
{
  // connectTimeout: the silent port accepts TCP but never sends CONNACK
  time t0 = getCurrentTime();
  broker("o_cto", makeMapping("ServerURI", "tcp://127.0.0.1:18852", "Options", makeDynString("connectTimeout=2")));
  string s = waitError("o_cto", 15);
  float took = (float)(getCurrentTime() - t0);
  result("connectTimeout=2 against a silent server", stateOf(s) != 3 && errOf(s) != "" && took < 6,
         "after " + took + " s: " + errOf(s));

  // fail-over list: the first URI is refused, the second works
  broker("o_fo", makeMapping("ServerURI", "tcp://localhost:1", "ServerURIs", makeDynString("tcp://localhost:1", "tcp://localhost:18840")));
  s = waitState("o_fo", 3, 15);
  string uri; dpGet("_RsMqtt_o_fo.State.ServerURI", uri);
  result("ServerURIs fail-over", stateOf(s) == 3 && strpos(uri, "18840") > 0, "State.ServerURI = " + uri + ", " + s);

  // empty ClientId -> driver default
  broker("o_cid", makeMapping("ServerURI", "tcp://localhost:18840", "ClientId", ""));
  waitState("o_cid", 3, 8);
  string cid; dpGet("_RsMqtt_o_cid.State.ClientId", cid);
  result("ClientId empty -> WCCOArsmqtt_<num>_<broker>", cid == "WCCOArsmqtt_" + DRV + "_o_cid", cid);

  // keep alive 1 s: stays connected while idle (PINGREQ/PINGRESP)
  broker("o_ka", makeMapping("ServerURI", "tcp://localhost:18840", "KeepAlive", 1));
  waitState("o_ka", 3, 8);
  time c0; dpGet("_RsMqtt_o_ka.State.ConnState:_online.._stime", c0);
  delay(4);
  int st; time c1; dpGet("_RsMqtt_o_ka.State.ConnState", st, "_RsMqtt_o_ka.State.ConnState:_online.._stime", c1);
  result("KeepAlive=1, idle 4 s", st == 3 && c1 == c0, "ConnState " + st);

  expectConnected("runtimeThreads=2, completionEvents=all, maxQueuedEvents=100", "o_misc",
                  makeMapping("ServerURI", "tcp://localhost:18840", "Options", makeDynString("runtimeThreads=2", "completionEvents=all", "maxQueuedEvents=100")));
  expectConnected("maxInflight=1", "o_infl", makeMapping("ServerURI", "tcp://localhost:18840", "Options", makeDynString("maxInflight=1")));
  expectRefused("unknown option name", "o_badopt", makeMapping("ServerURI", "tcp://localhost:18840", "Options", makeDynString("noSuchOption=1")), "unknown option");
  expectRefused("bad option value", "o_badval", makeMapping("ServerURI", "tcp://localhost:18840", "Options", makeDynString("keepAliveInterval=abc")), "");
  cleanup();
}

void v5()
{
  expectConnected("v5 CONNECT properties", "o_v5p", makeMapping("ServerURI", "tcp://localhost:18840", "Options", makeDynString(
    "sessionExpiryInterval=60", "receiveMaximum=5", "topicAliasMaximum=10", "requestResponseInformation=1",
    "requestProblemInformation=0", "userProperty=origin=winccoa")));

  // maximumPacketSize: the broker must not send bigger packets to the client
  broker("o_mps", makeMapping("ServerURI", "tcp://localhost:18840", "Options", makeDynString("maximumPacketSize=1000")));
  waitState("o_mps", 3, 8);
  string topic = "opt/" + rnd(3) + "/mps";
  setAddr(TEST_DP + ".in", "o_mps:" + topic, 1);
  delay(1);
  dpSetWait(TEST_DP + ".in", "");
  string big = "big"; while ( strlen(big) < 3000 ) big += "x";
  mosqPub("-V 5 -q 1 -t " + topic + " -m " + big);
  delay(1);
  string got; dpGet(TEST_DP + ".in", got);
  bool bigBlocked = (got == "");
  mosqPub("-V 5 -q 1 -t " + topic + " -m small");
  delay(1);
  dpGet(TEST_DP + ".in", got);
  result("maximumPacketSize=1000: 3000 B dropped, small delivered", bigBlocked && got == "small", "in = '" + substr(got, 0, 20) + "'");
  cleanup();
}

/** Offline message for a persistent session. `cfg` decides session handling. */
void sessionCase(string test, string name, mapping cfg, bool expectDelivered)
{
  string topic = "opt/" + rnd(3) + "/sess";
  cfg["ServerURI"] = "tcp://localhost:18840";
  cfg["ClientId"] = "opt-sess-" + rnd(3);
  broker(name, cfg);
  waitState(name, 3, 8);
  setAddr(TEST_DP + ".in", name + ":" + topic + ";qos=1", 1);
  delay(1);
  dpSetWait(TEST_DP + ".in", "");
  dpSetWait("_RsMqtt_" + name + ".Config.Active", FALSE);      // disconnects
  waitState(name, 0, 5);
  delay(2);   // ConnState drops at once, the DISCONNECT itself goes out asynchronously
  mosqPub("-V 5 -q 1 -t " + topic + " -m offline-msg");       // while the client is away
  dpSetWait("_RsMqtt_" + name + ".Config.Active", TRUE);
  waitState(name, 3, 8);
  string got;
  for (int i = 0; i < 30 && got != "offline-msg"; i++)
  {
    delay(0, 100);
    dpGet(TEST_DP + ".in", got);
  }
  result(test, (got == "offline-msg") == expectDelivered, "in = '" + got + "'");
  dpSetWait("_RsMqtt_" + name + ".Config.Active", FALSE);       // no late delivery into the next case
}

void session()
{
  sessionCase("3.1.1 CleanSession=FALSE keeps the session", "o_s4k", makeMapping("MQTTVersion", 4, "CleanSession", FALSE), TRUE);
  sessionCase("3.1.1 CleanSession=TRUE discards it",       "o_s4c", makeMapping("MQTTVersion", 4, "CleanSession", TRUE), FALSE);
  sessionCase("5 CleanSession=FALSE + sessionExpiryInterval=300", "o_s5k", makeMapping("MQTTVersion", 5, "CleanSession", FALSE,
              "Options", makeDynString("sessionExpiryInterval=300")), TRUE);
  sessionCase("5 CleanSession=FALSE, no session expiry",  "o_s5e", makeMapping("MQTTVersion", 5, "CleanSession", FALSE), FALSE);
  cleanup();
}

/** Broker paused -> keep alive detects the loss -> values are buffered ->
    broker resumed -> automatic reconnect sends the newest 4
    (paho raises maxBufferedMessages to at least 4). */
void buffer()
{
  string topic = "opt/" + rnd(3) + "/buf";
  broker("o_buf", makeMapping("ServerURI", "tcp://localhost:18840", "KeepAlive", 1,
         "Options", makeDynString("maxBufferedMessages=4", "deleteOldestMessages=1", "minRetryInterval=1", "maxRetryInterval=2")));
  waitState("o_buf", 3, 8);
  setAddr(TEST_DP + ".out", "o_buf:" + topic + ";qos=1", 2);
  // subscriber inside the broker container, connected before the pause
  sh("docker exec -d " + BROKER_CONTAINER + " sh -c \"mosquitto_sub -p 1883 -q 1 -t " + topic + " -W 40 > /tmp/buf.txt\"");
  delay(1);
  sh("docker pause " + BROKER_CONTAINER);
  string lost = waitState("o_buf", 1, 8);
  result("KeepAlive=1 detects a hung broker", stateOf(lost) == 1, "ConnState " + lost);
  for (int i = 1; i <= 6; i++) { dpSetWait(TEST_DP + ".out", "buffered-" + i); delay(0, 100); }
  sh("docker unpause " + BROKER_CONTAINER);
  string back = waitState("o_buf", 3, 10);
  result("automaticReconnect after the broker is back", stateOf(back) == 3, "ConnState " + back);
  delay(3);
  string got = sh("docker exec " + BROKER_CONTAINER + " cat /tmp/buf.txt");
  strreplace(got, "\n", ",");
  result("maxBufferedMessages=4 + deleteOldestMessages=1 send the newest 4", got == "buffered-3,buffered-4,buffered-5,buffered-6", "received: " + got);
  cleanup();
}

/** Will with v5 properties: arm, then the caller kills the driver process. */
void will_arm()
{
  string topic = "opt/will/" + rnd(3);
  broker("o_will", makeMapping("ServerURI", "tcp://localhost:18840", "WillTopic", topic, "WillPayload", "driver gone",
         "WillQoS", 1, "WillRetain", TRUE, "Options", makeDynString("will.contentType=text/plain", "will.userProperty=origin=winccoa",
         "will.responseTopic=opt/will/reply", "will.payloadFormatIndicator=1", "will.messageExpiryInterval=300", "will.delayInterval=1")));
  string s = waitState("o_will", 3, 8);
  result("will armed", stateOf(s) == 3, s);
  dpSetWait("RsMqttOpt1.in", topic);   // remember the topic for will_check
}

void will_check()
{
  string topic; dpGet("RsMqttOpt1.in", topic);
  string out = sh("docker exec " + BROKER_CONTAINER + " mosquitto_sub -p 1883 -V 5 -q 1 -t " + topic +
                  " -C 1 -W 5 -F \"%r|%q|%p|%C|%R|%F|%P\"");
  result("will published with retain, QoS 1 and v5 properties",
         out == "1|1|driver gone|text/plain|opt/will/reply|1|origin:winccoa", "got: " + out);
  mosqPub("-r -n -t " + topic);   // clear the retained will
  gBrokers = makeDynString("o_will");
  cleanup();
}

/** QoS 1 value in flight (no PUBACK) when the connection hangs, then the driver
    is killed; persistence=file must publish it after the restart.

    The driver connects through the netutil forwarder (18853); freezing it drops
    every byte, so the original PUBLISH never reaches the broker and only the
    copy restored from disk can deliver the value (docker pause would not do:
    the broker reads the frozen socket buffer after the unpause).
    paho.mqtt.rust stores in-flight messages only; values written after the
    loss is detected sit in the in-memory send-while-disconnected buffer and
    die with the process.

    Sequence: persist_arm -> kill the driver -> persist_down -> restart the
    driver -> persist_check. persist_down unfreezes and changes the output
    value while the driver is gone: a starting driver publishes the current
    value of its output addresses, which must not be mistaken for the restore. */
const string PERSIST_STORE = PROJ_PATH + DATA_REL_PATH + "rsmqtt/";   // driver default for persistence=file

/** This run's persistence folders: paho names them <clientId>-<uri>-<hash>. */
dyn_string persistDirs(string clientId)
{
  return getFileNames(PERSIST_STORE, clientId + "-*", FILTER_DIRS);
}

void persist_arm()
{
  string topic = "opt/" + rnd(3) + "/persist";
  string clientId = "opt-persist-" + rnd(2);
  remove(FREEZE);
  broker("o_per", makeMapping("ServerURI", "tcp://localhost:18853", "ClientId", clientId, "KeepAlive", 1, "CleanSession", FALSE,
         "Options", makeDynString("persistence=file", "sessionExpiryInterval=300")));
  waitState("o_per", 3, 8);
  setAddr(TEST_DP + ".out", "o_per:" + topic + ";qos=1", 2);
  sh("docker exec -d " + BROKER_CONTAINER + " sh -c \"mosquitto_sub -p 1883 -q 1 -t " + topic + " -W 90 > /tmp/per.txt\"");
  delay(1);
  file f = fopen(FREEZE, "w"); fclose(f);
  dpSetWait(TEST_DP + ".out", "persisted-value");   // sent into the frozen forwarder
  string lost = waitState("o_per", 1, 8);
  delay(1);
  result("value in flight while the connection hangs", stateOf(lost) == 1, "ConnState " + lost);
  // no persistenceDir: the driver stores it under <project>/data/rsmqtt
  dyn_string stored;
  dyn_string dirs = persistDirs(clientId);
  for (int i = 1; i <= dynlen(dirs); i++)
    dynAppend(stored, getFileNames(PERSIST_STORE + dirs[i], "*"));
  result("persistence folder defaults to <project>/data/rsmqtt", dynlen(stored) > 0,
         PERSIST_STORE + " " + strjoin(dirs, ",") + ": " + strjoin(stored, ","));
}

void persist_down()
{
  remove(FREEZE);
  dpSetWait(TEST_DP + ".out", "current-value");
  result("driver down, output changed", TRUE, "");
}

void persist_check()
{
  remove(FREEZE);
  string got = sh("docker exec " + BROKER_CONTAINER + " cat /tmp/per.txt");
  strreplace(got, "\n", ",");
  result("persistence=file publishes the stored value after a driver restart", strpos(got, "persisted-value") >= 0, "received: '" + got + "'");
  string clientId; dpGet("_RsMqtt_o_per.Config.ClientId", clientId);
  gBrokers = makeDynString("o_per");
  cleanup();
  // paho empties its folder but leaves it in the project
  dyn_string dirs = clientId != "" ? persistDirs(clientId) : makeDynString();
  for (int i = 1; i <= dynlen(dirs); i++)
    rmdir(PERSIST_STORE + dirs[i], true);
}

dyn_string main(string group)
{
  ensureTestDp();
  switch ( group )
  {
    case "transport":     transport(); break;
    case "auth":          auth(); break;
    case "tls":           tls(); break;
    case "ws":            ws(); break;
    case "connect":       connect(); break;
    case "v5":            v5(); break;
    case "session":       session(); break;    case "buffer":        buffer(); break;
    case "will_arm":      will_arm(); break;
    case "will_check":    will_check(); break;
    case "persist_arm":   persist_arm(); break;
    case "persist_down":  persist_down(); break;
    case "persist_check": persist_check(); break;
    default: dynAppend(gResults, "unknown group " + group);
  }
  return gResults;
}
