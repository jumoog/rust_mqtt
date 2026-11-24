// rsmqtt.ctl - CTRL library for the WCCOArsmqtt MQTT driver.
//
//   #uses "rsmqtt"
//
// Brokers (datapoints of type _RsMqttBroker, see dplist/rsmqtt_types.asc):
//
//   rsmqttCreateBroker("plant", 2, "tcp://broker:1883");
//   rsmqttCreateBroker("cloud", 2, "mqtts://mqtt.example.com:8883",
//                      makeMapping("Username", "winccoa", "Password", "secret",
//                                  "TlsTrustStore", "C:/certs/ca.pem", "TlsVerify", TRUE));
//   rsmqttSetBrokerConfig("plant", makeMapping("KeepAlive", 20));
//   rsmqttDeleteBroker("plant");
//
// Addresses:
//
//   rsmqttSetAddress("Pump1.temp",     "plant:line1/pump1/temp",            RSMQTT_FLOAT, RSMQTT_IN,  2);
//   rsmqttSetAddress("Pump1.setpoint", "plant:line1/pump1/sp;qos=1;retain", RSMQTT_FLOAT, RSMQTT_OUT, 2);
//   rsmqttSetAddress("Pump1.online",   "plant:$state",                      RSMQTT_BOOL,  RSMQTT_IN,  2);

// _address.._drv_ident of rsmqtt addresses
const string RSMQTT_DRV_IDENT = "RSMQTT";

// _address.._datatype values (see RsMqttTransformationType.hxx)
const int RSMQTT_STRING = 1001;
const int RSMQTT_INT32  = 1002;
const int RSMQTT_UINT32 = 1003;
const int RSMQTT_INT64  = 1004;
const int RSMQTT_UINT64 = 1005;
const int RSMQTT_FLOAT  = 1006;
const int RSMQTT_BOOL   = 1007;
const int RSMQTT_BLOB   = 1008;

// directions
const int RSMQTT_IN  = 1;   // subscribe
const int RSMQTT_OUT = 2;   // publish
const int RSMQTT_IO  = 3;   // both

// State.ConnState of a broker
const int RSMQTT_CS_DISABLED     = 0;
const int RSMQTT_CS_DISCONNECTED = 1;
const int RSMQTT_CS_CONNECTING   = 2;
const int RSMQTT_CS_CONNECTED    = 3;
const int RSMQTT_CS_FAILURE      = 4;

//------------------------------------------------------------------------------
// Redundancy
//
// As with the WinCC OA drivers, every DP exists twice in a redundant project:
// host 1 uses "<dp>", host 2 "<dp>_2". Scripts and panels write Config,
// Command and Brokers to "<dp>"; the event manager copies them to "<dp>_2"
// (copyDpType in config/config.redu). Both drivers report into their own DP.

const string RSMQTT_REDU = "_2";

/** "<dp>_2" in a redundant project, "" otherwise. */
string rsmqttReduTwin(string dp)
{
  return isRedundant() ? dp + RSMQTT_REDU : "";
}

/** The host whose driver is active: 1 or 2 (1 in a single system). */
int rsmqttActiveHost()
{
  int host = 1;
  if ( isRedundant() )
    reduActive(host);
  return host == 2 ? 2 : 1;
}

/** The DP of host `host` (1 or 2): "<dp>" or "<dp>_2". */
string rsmqttHostDp(string dp, int host)
{
  return host == 2 ? dp + RSMQTT_REDU : dp;
}

/** Creates the "_2" twin of a DP (with all values) if the project is
    redundant and the twin is missing. Returns 0 or -1. */
int rsmqttCreateTwin(string dp, string dpType)
{
  string twin = rsmqttReduTwin(dp);
  if ( twin == "" || !dpExists(dp) || dpExists(twin) )
    return 0;
  if ( dpCreate(twin, dpType) != 0 || !dpExists(twin) )
  {
    DebugTN("rsmqtt: cannot create " + twin, getLastError());
    return -1;
  }
  // the values, like drvsUpgradeToRedundant() of the WinCC OA drivers does it
  dyn_string dpes = dpNames(dp + ".**:_original.._value", dpType);
  dyn_anytype values;
  if ( dynlen(dpes) > 0 && dpGet(dpes, values) == 0 )
  {
    for (int i = 1; i <= dynlen(dpes); i++)
      strreplace(dpes[i], dpSubStr(dp, DPSUB_DP) + ".", dpSubStr(twin, DPSUB_DP) + ".");
    dpSetWait(dpes, values);
  }
  return 0;
}

/** Creates the missing "_2" twins of all broker and driver DPs, e.g. after a
    project was made redundant (like drvsUpgradeToRedundant() of the WinCC OA
    drivers). Returns the number of twins created. */
int rsmqttUpgradeToRedundant()
{
  int created;
  if ( !isRedundant() )
    return 0;
  dyn_string brokers = rsmqttGetAllBrokers();
  for (int i = 1; i <= dynlen(brokers); i++)
    if ( !dpExists(rsmqttBrokerDp(brokers[i]) + RSMQTT_REDU) && rsmqttCreateTwin(rsmqttBrokerDp(brokers[i]), "_RsMqttBroker") == 0 )
      created++;
  dyn_string drivers = rsmqttDriverDps();
  for (int i = 1; i <= dynlen(drivers); i++)
    if ( !dpExists(drivers[i] + RSMQTT_REDU) && rsmqttCreateTwin(drivers[i], "_RsMqttDriver") == 0 )
      created++;
  return created;
}

//------------------------------------------------------------------------------
// Brokers

/** DP name of a broker (of host 1; host 2 uses the "_2" twin). */
string rsmqttBrokerDp(string broker)
{
  return "_RsMqtt_" + broker;
}

/** DP name of the driver DP that lists the brokers of a driver number. */
string rsmqttDriverDp(int driverNum)
{
  return "_RsMqttDriver_" + driverNum;
}

/** The _RsMqttDriver DPs "_RsMqttDriver_<num>" (without the "_2" twins). */
dyn_string rsmqttDriverDps()
{
  dyn_string dps = dpNames("_RsMqttDriver_*", "_RsMqttDriver"), out;
  for (int i = 1; i <= dynlen(dps); i++)
  {
    string n = substr(dpSubStr(dps[i], DPSUB_DP), strlen("_RsMqttDriver_"));
    if ( n != "" && strltrim(n, "0123456789") == "" )
      dynAppend(out, dpSubStr(dps[i], DPSUB_DP));
  }
  return out;
}

/** All broker names (from the _RsMqttBroker DPs of the local system; the
    "_2" twins of a redundant project are not brokers of their own, in a
    single system "<x>_2" is an ordinary broker). */
dyn_string rsmqttGetAllBrokers()
{
  dyn_string dps = dpNames("_RsMqtt_*", "_RsMqttBroker"), names;
  bool redu = isRedundant();
  for (int i = 1; i <= dynlen(dps); i++)
  {
    string n = dpSubStr(dps[i], DPSUB_DP);
    if ( redu && patternMatch("*" + RSMQTT_REDU, n) && dpExists(substr(n, 0, strlen(n) - strlen(RSMQTT_REDU))) )
      continue;
    dynAppend(names, substr(n, strlen("_RsMqtt_")));
  }
  dynSort(names);
  return names;
}

/** Brokers served by a driver number. */
dyn_string rsmqttGetBrokers(int driverNum)
{
  dyn_string brokers;
  string dp = rsmqttDriverDp(driverNum);
  if ( dpExists(dp) )
    dpGet(dp + ".Brokers", brokers);
  return brokers;
}

/** Driver number that serves a broker, 0 if none. */
int rsmqttGetBrokerDriver(string broker)
{
  dyn_string dps = rsmqttDriverDps();
  for (int i = 1; i <= dynlen(dps); i++)
  {
    dyn_string brokers;
    dpGet(dps[i] + ".Brokers", brokers);
    if ( dynContains(brokers, broker) > 0 )
      return (int) substr(dpSubStr(dps[i], DPSUB_DP), strlen("_RsMqttDriver_"));
  }
  return 0;
}

/** Creates (or updates) a broker DP and assigns it to a driver number.
  @param broker     name used in addresses ("<broker>:<topic>"); letters, digits, '_' and '-'
  @param driverNum  number the driver is started with (-num)
  @param serverURI  e.g. tcp://host:1883, mqtts://host:8883, ws://host/mqtt, wss://...
  @param config     optional Config.* values, e.g. makeMapping("Username", "u", "KeepAlive", 20)
  @return 0 on success, -1 on error
*/
int rsmqttCreateBroker(string broker, int driverNum, string serverURI, mapping config = makeMapping())
{
  if ( !rsmqttIsValidBrokerName(broker) )
  {
    DebugTN("rsmqttCreateBroker: invalid broker name", broker);
    return -1;
  }

  string dp = rsmqttBrokerDp(broker);
  if ( !dpExists(dp) )
  {
    if ( !rsmqttIsNewBrokerName(broker) )
    {
      DebugTN("rsmqttCreateBroker: a new broker name must not end with \"" + RSMQTT_REDU + "\" (redundancy)", broker);
      return -1;
    }
    if ( dpCreate(dp, "_RsMqttBroker") != 0 || !dpExists(dp) )
    {
      DebugTN("rsmqttCreateBroker: cannot create", dp, getLastError());
      return -1;
    }
    // redundant project: the twin first, so that copyDpType copies every value below
    if ( rsmqttCreateTwin(dp, "_RsMqttBroker") != 0 )
      return -1;
    // defaults
    dpSetWait(dp + ".Config.MQTTVersion", 5,
              dp + ".Config.KeepAlive", 30,
              dp + ".Config.CleanSession", TRUE,
              dp + ".Config.SubscribeQoS", 1,
              dp + ".Config.PublishQoS", 1,
              dp + ".Config.PublishRetain", FALSE,
              dp + ".Config.TlsVerify", TRUE);   // check the host name (TLS URIs)
  }

  if ( dpSetWait(dp + ".Config.ServerURI", serverURI) != 0 )
    return -1;
  if ( rsmqttSetBrokerConfig(broker, config) != 0 )
    return -1;
  if ( rsmqttAssignBroker(broker, driverNum) != 0 )
    return -1;

  // Active last: the driver then sees the complete configuration at once.
  if ( !mappingHasKey(config, "Active") )
    dpSetWait(dp + ".Config.Active", TRUE);
  return 0;
}

//------------------------------------------------------------------------------
// Passwords
//
// Config.Password (and a "password=..." entry of Config.Options or the config
// file) may hold a password encrypted with _DriverSecurity.PublicKey; the
// driver decrypts it with config/driver_private.key. Both the envelope format
// of rsmqttEncryptPassword() and the format of the UI function
// drvsSecSetPassword() (driverSettings.ctl) are accepted.

const string RSMQTT_PUBLIC_KEY_DPE = "_DriverSecurity.PublicKey";
const string RSMQTT_PASSWORD_MASK  = "*****";

/** Encrypts a password with _DriverSecurity.PublicKey (works in every manager).
  @return hex text for Config.Password, "" for an empty password or on error
          (no public key)
*/
string rsmqttEncryptPassword(string plain)
{
  if ( plain == "" )
    return "";

  string publicKey;
  if ( dpExists(RSMQTT_PUBLIC_KEY_DPE) )
    dpGet(RSMQTT_PUBLIC_KEY_DPE, publicKey);
  if ( publicKey == "" )
  {
    DebugTN("rsmqttEncryptPassword: " + RSMQTT_PUBLIC_KEY_DPE + " is empty, cannot encrypt");
    return "";
  }

  blob plainBlob, crypt;
  blobAppendValue(plainBlob, plain, strlen(plain));
  encrypt(plainBlob, publicKey, crypt, "ENVELOPE_RSA_AES_256_CBC");
  if ( bloblen(crypt) == 0 )
  {
    DebugTN("rsmqttEncryptPassword: encryption failed", getLastError());
    return "";
  }
  return (string) crypt;
}

/** TRUE if `value` looks like an encrypted password (same rules as the driver). */
bool rsmqttIsEncryptedPassword(string value)
{
  int len = strlen(value);
  if ( len == 0 || (len % 2) != 0 )
    return FALSE;
  for (int i = 0; i < len; i++)
    if ( strpos("0123456789ABCDEFabcdef", substr(value, i, 1)) < 0 )
      return FALSE;
  if ( strtoupper(substr(value, 0, 32)) == "01000000010000006400000064000000" )
    return TRUE;                                   // envelope (rsmqttEncryptPassword)
  return dynContains(makeDynInt(128, 256, 384, 512, 768, 1024), len / 2) > 0;  // RSA (drvsSecSetPassword)
}

/** Sets Config.* elements of a broker; keys are the element names below
    Config (e.g. "ServerURI", "Username", "TlsVerify", "Options").
    A plain "Password" is stored encrypted with _DriverSecurity.PublicKey
    (as is if there is no public key); an encrypted one is stored unchanged. */
int rsmqttSetBrokerConfig(string broker, mapping config)
{
  string dp = rsmqttBrokerDp(broker);
  if ( !dpExists(dp) )
    return -1;

  if ( mappingHasKey(config, "Password") )
  {
    string pw = config["Password"];
    if ( pw != "" && !rsmqttIsEncryptedPassword(pw) )
    {
      string crypt = rsmqttEncryptPassword(pw);
      if ( crypt != "" )
        config["Password"] = crypt;
      else
        DebugTN("rsmqttSetBrokerConfig: storing the password of broker '" + broker + "' unencrypted");
    }
  }

  dyn_string dpes;
  dyn_anytype values;
  for (int i = 1; i <= mappinglen(config); i++)
  {
    dynAppend(dpes, dp + ".Config." + mappingGetKey(config, i));
    // indexed: dynAppend() would flatten dyn values (ServerURIs, Options)
    values[dynlen(dpes)] = mappingGetValue(config, i);
  }
  if ( dynlen(dpes) == 0 )
    return 0;
  return dpSetWait(dpes, values) == 0 ? 0 : -1;
}

/** Makes `driverNum` serve the broker (and no other driver). */
int rsmqttAssignBroker(string broker, int driverNum)
{
  // remove from other drivers
  dyn_string dps = rsmqttDriverDps();
  string target = rsmqttDriverDp(driverNum);
  for (int i = 1; i <= dynlen(dps); i++)
  {
    if ( dpSubStr(dps[i], DPSUB_DP) == target ) continue;
    dyn_string brokers;
    dpGet(dps[i] + ".Brokers", brokers);
    int pos = dynContains(brokers, broker);
    if ( pos > 0 )
    {
      dynRemove(brokers, pos);
      dpSetWait(dps[i] + ".Brokers", brokers);
    }
  }

  if ( !dpExists(target) && dpCreate(target, "_RsMqttDriver") != 0 )
  {
    DebugTN("rsmqttAssignBroker: cannot create", target, getLastError());
    return -1;
  }
  if ( rsmqttCreateTwin(target, "_RsMqttDriver") != 0 )
    return -1;
  dyn_string brokers;
  dpGet(target + ".Brokers", brokers);
  if ( dynContains(brokers, broker) < 1 )
    dynAppend(brokers, broker);
  // Written even when unchanged: the hotlink lets the driver take over a
  // broker DP that was deleted and created again under the same name.
  return dpSetWait(target + ".Brokers", brokers) == 0 ? 0 : -1;
}

/** Removes a broker from its driver and deletes its DP. */
int rsmqttDeleteBroker(string broker)
{
  dyn_string dps = rsmqttDriverDps();
  for (int i = 1; i <= dynlen(dps); i++)
  {
    dyn_string brokers;
    dpGet(dps[i] + ".Brokers", brokers);
    int pos = dynContains(brokers, broker);
    if ( pos > 0 )
    {
      dynRemove(brokers, pos);
      dpSetWait(dps[i] + ".Brokers", brokers);
    }
  }
  string dp = rsmqttBrokerDp(broker);
  if ( dpExists(dp) )
  {
    // give the driver time to close the connection before the DP is gone
    delay(1);
    if ( dpExists(dp + RSMQTT_REDU) && isRedundant() )
      dpDelete(dp + RSMQTT_REDU);
    return dpDelete(dp) == 0 ? 0 : -1;
  }
  return 0;
}

/** Asks the driver to drop and re-open the connection. */
int rsmqttReconnectBroker(string broker)
{
  return dpSetWait(rsmqttBrokerDp(broker) + ".Command.Reconnect", TRUE) == 0 ? 0 : -1;
}

/** TRUE for a name a new broker may get: valid, and not ending with "_2",
    which names the second redundant host's twin (as for the WinCC OA drivers).
    Existing brokers keep working with such names in single systems. */
bool rsmqttIsNewBrokerName(string broker)
{
  return rsmqttIsValidBrokerName(broker) && !patternMatch("*" + RSMQTT_REDU, broker);
}

bool rsmqttIsValidBrokerName(string broker)
{
  if ( broker == "" ) return FALSE;
  for (int i = 0; i < strlen(broker); i++)
  {
    string c = broker[i];
    if ( strpos("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-", c) < 0 )
      return FALSE;
  }
  return TRUE;
}

/** One-line status of a broker for panels, e.g. "tcp://host:1883  -  connected". */
string rsmqttBrokerStatusLine(string broker)
{
  if ( broker == "" )
    return "";
  string dp = rsmqttBrokerDp(broker);
  if ( !dpExists(dp) )
    return "no datapoint " + dp + " (broker from the config file?)";

  int host = rsmqttActiveHost();
  string sdp = rsmqttHostDp(dp, host);
  if ( !dpExists(sdp) )
    sdp = dp;
  int state;
  string err, uri, configUri;
  // the URI in use (host 2 may have redu.serverURI), else the configured one
  dpGet(sdp + ".State.ConnState", state,
        sdp + ".State.LastError", err,
        sdp + ".State.ServerURI", uri,
        dp + ".Config.ServerURI", configUri);
  if ( uri == "" )
    uri = configUri;
  string line = uri + "  -  " + rsmqttConnStateText(state) + (err != "" ? "  (" + err + ")" : "");
  return line + rsmqttOtherHostState(dp, host);
}

/** "   |  host 2: connected" in a redundant project, "" otherwise. */
string rsmqttOtherHostState(string dp, int activeHost)
{
  if ( !isRedundant() )
    return "";
  int other = activeHost == 2 ? 1 : 2;
  string odp = rsmqttHostDp(dp, other);
  string hostText = "   |  " + rsmqttT("Host ", "host ") + other + ": ";
  if ( !dpExists(odp) )
    return hostText + rsmqttT("kein Datenpunkt ", "no datapoint ") + odp;
  int state;
  dpGet(odp + ".State.ConnState", state);
  return hostText + rsmqttConnStateText(state);
}

/** Text in the UI language: German or English (the panels' languages). */
string rsmqttT(string de, string en)
{
  return strpos(getLocale(getActiveLang()), "de") == 0 ? de : en;
}

string rsmqttConnStateText(int state)
{
  switch ( state )
  {
    case RSMQTT_CS_DISABLED:     return rsmqttT("deaktiviert", "disabled");
    case RSMQTT_CS_DISCONNECTED: return rsmqttT("getrennt", "disconnected");
    case RSMQTT_CS_CONNECTING:   return rsmqttT("verbindet", "connecting");
    case RSMQTT_CS_CONNECTED:    return rsmqttT("verbunden", "connected");
    case RSMQTT_CS_FAILURE:      return rsmqttT("Fehler", "failure");
  }
  return rsmqttT("unbekannt", "unknown");
}

//------------------------------------------------------------------------------
// Addresses

/** Builds an address reference.
  @param qos       -1 = broker default
  @param retain    -1 = broker default, 0, 1
  @param extra     further options, e.g. "nolocal;rh=1", or "merge" (with json and a path: the
                   outputs with merge on one topic publish one JSON document)
  @param json      the payload is JSON
  @param jsonPath  where the value is, e.g. "data.temp" or "values[0]"; "" = the whole payload
  @param tsPath    with json: where the timestamp is, e.g. "data.time"; "" = none
*/
string rsmqttMakeReference(string broker, string topic, int qos = -1, int retain = -1, string extra = "",
                           bool json = FALSE, string jsonPath = "", string tsPath = "")
{
  string ref = broker + ":" + topic;
  if ( qos >= 0 )    ref += ";qos=" + qos;
  if ( retain >= 0 ) ref += ";retain=" + retain;
  if ( json )        ref += jsonPath == "" ? ";json" : ";json=" + jsonPath;
  if ( json && tsPath != "" ) ref += ";ts=" + tsPath;
  if ( extra != "" ) ref += ";" + extra;
  return ref;
}

/** Removes option <name>[=value] from an option list like "json=a.b;nolocal".
  @return TRUE if the option was there; value gets its value ("" without "=")
*/
bool rsmqttTakeOption(string &extra, string name, string &value)
{
  bool found = FALSE;
  string rest;
  value = "";
  dyn_string parts = strsplit(extra, ";");
  for (int i = 1; i <= dynlen(parts); i++)
  {
    string p = strltrim(strrtrim(parts[i]));
    if ( p == "" ) continue;
    int eq = strpos(p, "=");
    string n = strtolower(strltrim(strrtrim(eq < 0 ? p : substr(p, 0, eq))));
    if ( n == strtolower(name) )
    {
      found = TRUE;
      value = eq < 0 ? "" : strltrim(strrtrim(substr(p, eq + 1)));
    }
    else
      rest += (rest == "" ? "" : ";") + p;
  }
  extra = rest;
  return found;
}

/** Splits a reference into its parts; returns FALSE if it is malformed. */
bool rsmqttParseReference(string ref, string &broker, string &topic, int &qos, int &retain, string &extra)
{
  broker = ""; topic = ""; qos = -1; retain = -1; extra = "";
  int colon = strpos(ref, ":");
  if ( colon < 1 ) return FALSE;
  broker = substr(ref, 0, colon);
  dyn_string parts = strsplit(substr(ref, colon + 1), ";");
  if ( dynlen(parts) < 1 || parts[1] == "" ) return FALSE;
  topic = parts[1];
  for (int i = 2; i <= dynlen(parts); i++)
  {
    string p = strltrim(strrtrim(parts[i]));
    if ( p == "" ) continue;
    if ( strpos(p, "qos=") == 0 )
      qos = (int) substr(p, 4);
    else if ( p == "retain" || p == "retain=1" )
      retain = 1;
    else if ( p == "retain=0" )
      retain = 0;
    else
      extra += (extra == "" ? "" : ";") + p;
  }
  return TRUE;
}

/** Sets the driver and peripheral address configs of a DPE.
  @param dpe        datapoint element, e.g. "Pump1.temperature"
  @param reference  "<broker>:<topic>[;qos=N][;retain][;nolocal][;rap][;rh=N][;json[=path]][;ts=path][;merge]"
  @param datatype   RSMQTT_STRING .. RSMQTT_BLOB; for a dyn element the type of its items,
                    the payload is then a JSON array (dyn_blob: of base64 strings)
  @param direction  RSMQTT_IN, RSMQTT_OUT or RSMQTT_IO
  @param driverNum  number the driver is started with (-num)
  @return 0 on success, -1 on error
*/
int rsmqttSetAddress(string dpe, string reference, int datatype, int direction, int driverNum)
{
  int mode;
  switch ( direction )
  {
    case RSMQTT_OUT: mode = DPATTR_ADDR_MODE_OUTPUT_SINGLE; break;
    case RSMQTT_IO:  mode = DPATTR_ADDR_MODE_IO_SPONT;      break;
    default:         mode = DPATTR_ADDR_MODE_INPUT_SPONT;   break;
  }

  int rc = dpSetWait(dpe + ":_distrib.._type", DPCONFIG_DISTRIBUTION_INFO,
                     dpe + ":_distrib.._driver", driverNum);
  if ( rc != 0 )
  {
    DebugTN("rsmqttSetAddress: cannot set _distrib of " + dpe, getErrorText(getLastError()));
    return -1;
  }

  // Note: the event manager passes address configs on to the driver, so the
  // driver with this number must be running.
  rc = dpSetWait(dpe + ":_address.._type",      DPCONFIG_PERIPH_ADDR_MAIN,
                 dpe + ":_address.._drv_ident", RSMQTT_DRV_IDENT,
                 dpe + ":_address.._reference", reference,
                 dpe + ":_address.._mode",      mode,
                 dpe + ":_address.._datatype",  datatype,
                 dpe + ":_address.._active",    TRUE);
  if ( rc != 0 )
  {
    DebugTN("rsmqttSetAddress: cannot set the address of " + dpe + " (is WCCOArsmqtt -num " + driverNum + " running?)",
            getErrorText(getLastError()));
    return -1;
  }
  return 0;
}

/** Removes the peripheral address config of a DPE. */
int rsmqttRemoveAddress(string dpe)
{
  return dpSetWait(dpe + ":_address.._type", DPCONFIG_NONE) == 0 ? 0 : -1;
}

//------------------------------------------------------------------------------
// Broker configuration as a mapping (keys = element names below Config)

/** All Config.* values of a broker (empty mapping if the DP is missing). */
mapping rsmqttGetBrokerConfig(string broker)
{
  mapping cfg;
  string dp = rsmqttBrokerDp(broker);
  if ( !dpExists(dp) )
    return cfg;

  dyn_string names = makeDynString(
    "Active", "ServerURI", "ServerURIs", "ClientId", "MQTTVersion", "KeepAlive", "CleanSession",
    "Username", "Password", "TlsTrustStore", "TlsKeyStore", "TlsPrivateKey", "TlsVerify",
    "WillTopic", "WillPayload", "WillQoS", "WillRetain",
    "SubscribeQoS", "PublishQoS", "PublishRetain", "Options");
  dyn_string dpes;
  for (int i = 1; i <= dynlen(names); i++)
    dynAppend(dpes, dp + ".Config." + names[i]);
  dyn_anytype values;
  dpGet(dpes, values);
  for (int i = 1; i <= dynlen(names); i++)
    cfg[names[i]] = values[i];
  return cfg;
}

//------------------------------------------------------------------------------
// Broker panel (panels/para/rsmqtt_brokers.pnl). These functions read and
// write the panel's shapes by name and are called from its scripts.

// MQTT version combobox: 1 Default, 2 = 3.1, 3 = 3.1.1, 4 = 5
int rsmqttPanelVersionPos(int version)
{
  switch ( version )
  {
    case 3: return 2;
    case 4: return 3;
    case 5: return 4;
  }
  return 1;
}

int rsmqttPanelPosVersion(int pos)
{
  switch ( pos )
  {
    case 2: return 3;
    case 3: return 4;
    case 4: return 5;
  }
  return 0;
}

string rsmqttPanelJoin(dyn_string ds, string sep)
{
  string s;
  for (int i = 1; i <= dynlen(ds); i++)
    s += (i > 1 ? sep : "") + ds[i];
  return s;
}

dyn_string rsmqttPanelSplit(string s, string seps)
{
  dyn_string out;
  for (int i = 0; i < strlen(seps); i++)
    strreplace(s, seps[i], "\n");
  dyn_string parts = strsplit(s, "\n");
  for (int i = 1; i <= dynlen(parts); i++)
  {
    string p = strltrim(strrtrim(parts[i]));
    if ( p != "" ) dynAppend(out, p);
  }
  return out;
}

// Sparkplug fields of the panel: these options are shown and saved there
const string RSMQTT_OPT_SPB_HOST  = "sparkplug.hostid";
const string RSMQTT_OPT_SPB_HOST2 = "redu.sparkplug.hostid";

/** Name of an option "name=value", trimmed and in lower case. */
string rsmqttPanelOptName(string opt)
{
  int eq = strpos(opt, "=");
  return strtolower(strltrim(strrtrim(eq < 0 ? opt : substr(opt, 0, eq))));
}

/** TRUE for an option that may be given more than once (a name of rsmqttPanelOptNorm). */
bool rsmqttPanelOptRepeatable(string norm)
{
  if ( strpos(norm, "redu.") == 0 )
    norm = substr(norm, 5);
  return norm == "userproperty" || norm == "httpheader" || norm == "will.userproperty";
}

/** The broker panel's option list (cmbAddOption): adds the option picked
    there, "name=example value", to txtOptions. An option that is already
    set is not added twice (except the repeatable ones); the list goes back
    to its first entry. */
void rsmqttPanelAddOption()
{
  int pos;
  string opt, options;
  getMultiValue("cmbAddOption", "selectedPos", pos,
                "cmbAddOption", "text",        opt,
                "txtOptions",   "text",        options);
  if ( pos <= 1 )
    return;  // the "add an option" entry
  setValue("cmbAddOption", "selectedPos", 1);

  string name = rsmqttPanelOptName(opt);
  dyn_string opts = rsmqttPanelSplit(options, ";\n");
  if ( !rsmqttPanelOptRepeatable(rsmqttPanelOptNorm(name)) )
    for (int i = 1; i <= dynlen(opts); i++)
      if ( rsmqttPanelOptNorm(rsmqttPanelOptName(opts[i])) == rsmqttPanelOptNorm(name) )
      {
        rsmqttPanelMessage(rsmqttT("'" + rsmqttPanelOptName(opts[i]) + "' ist schon gesetzt.",
                                   "'" + rsmqttPanelOptName(opts[i]) + "' is set already."), TRUE);
        return;
      }
  dynAppend(opts, opt);
  setValue("txtOptions", "text", rsmqttPanelJoin(opts, "; "));
  rsmqttPanelMessage(rsmqttT("'" + opt + "' hinzugefügt: Wert anpassen und speichern.",
                             "'" + opt + "' added: adjust the value and save."), FALSE);
}

/** An option name as rsmqtt compares it: lower case, without '_' and '-'. */
string rsmqttPanelOptNorm(string name)
{
  strreplace(name, "_", "");
  strreplace(name, "-", "");
  return strtolower(strltrim(strrtrim(name)));
}

/** Value of an option "name=value", trimmed. */
string rsmqttPanelOptValue(string opt)
{
  int eq = strpos(opt, "=");
  return eq < 0 ? "" : strltrim(strrtrim(substr(opt, eq + 1)));
}

/** TRUE if a Sparkplug host id is usable in a topic ("" is allowed: none). */
bool rsmqttPanelValidHostId(string id)
{
  return strpos(id, "/") < 0 && strpos(id, "+") < 0 && strpos(id, "#") < 0 && strpos(id, ";") < 0;
}

/** Colour of a connection state. */
string rsmqttConnStateColor(int state)
{
  switch ( state )
  {
    case RSMQTT_CS_CONNECTED:    return "{0,140,0}";
    case RSMQTT_CS_CONNECTING:   return "{200,130,0}";
    case RSMQTT_CS_DISCONNECTED: return "{200,130,0}";
    case RSMQTT_CS_FAILURE:      return "{200,0,0}";
  }
  return "_3DText";
}

//------------------------------------------------------------------------------
// unsaved changes: the form as last shown or saved, kept in the hidden label
// lblSnapshot; lblShown names the broker the form belongs to ("" = new)

/** All form fields except the broker name (picking another broker is no change). */
string rsmqttPanelFormText()
{
  string drv, uri, uris, clientId, keepAlive, user, password, trust, keyStore, key,
         willTopic, willPayload, options, spb1, spb2;
  bool active, clean, verify, willRetain, pubRetain;
  int versionPos, willQosPos, subQosPos, pubQosPos;
  getMultiValue("spinDriver",     "text",        drv,
                "chkActive",      "state", 0,    active,
                "txtUri",         "text",        uri,
                "txtUris",        "text",        uris,
                "txtClientId",    "text",        clientId,
                "cmbVersion",     "selectedPos", versionPos,
                "txtKeepAlive",   "text",        keepAlive,
                "chkClean",       "state", 0,    clean,
                "txtUser",        "text",        user,
                "txtPassword",    "text",        password,
                "txtTrust",       "text",        trust,
                "txtKeyStore",    "text",        keyStore,
                "txtKey",         "text",        key,
                "chkVerify",      "state", 0,    verify,
                "txtWillTopic",   "text",        willTopic,
                "txtWillPayload", "text",        willPayload,
                "cmbWillQos",     "selectedPos", willQosPos,
                "chkWillRetain",  "state", 0,    willRetain,
                "cmbSubQos",      "selectedPos", subQosPos,
                "cmbPubQos",      "selectedPos", pubQosPos,
                "chkPubRetain",   "state", 0,    pubRetain,
                "txtSpbHost",     "text",        spb1,
                "txtSpbHost2",    "text",        spb2,
                "txtOptions",     "text",        options);
  return drv + "\t" + active + "\t" + uri + "\t" + uris + "\t" + clientId + "\t" + versionPos + "\t" + keepAlive +
         "\t" + clean + "\t" + user + "\t" + password + "\t" + trust + "\t" + keyStore + "\t" + key + "\t" + verify +
         "\t" + willTopic + "\t" + willPayload + "\t" + willQosPos + "\t" + willRetain + "\t" + subQosPos +
         "\t" + pubQosPos + "\t" + pubRetain + "\t" + spb1 + "\t" + spb2 + "\t" + options;
}

/** The form now matches `broker` ("" = a new, unsaved broker). */
void rsmqttPanelSetClean(string broker)
{
  setMultiValue("lblShown",    "text", broker,
                "lblSnapshot", "text", rsmqttPanelFormText(),
                "lblCopyFrom", "text", "");
  rsmqttPanelSelectRow(broker);
}

bool rsmqttPanelIsDirty()
{
  string snapshot;
  getValue("lblSnapshot", "text", snapshot);
  return snapshot != rsmqttPanelFormText();
}

/** TRUE if there are no unsaved changes or the user discards them. */
bool rsmqttPanelConfirmDiscard()
{
  if ( !rsmqttPanelIsDirty() )
    return TRUE;
  string shown;
  getValue("lblShown", "text", shown);
  string what = shown != "" ? " '" + shown + "'" : "";
  dyn_float df;
  dyn_string ds;
  ChildPanelOnCentralModalReturn("vision/MessageInfo", "rsmqtt",
    makeDynString("$1:" + rsmqttT("Ungespeicherte Änderungen" + (shown != "" ? " an" + what : "") + " verwerfen?",
                                  "Discard the unsaved changes" + (shown != "" ? " of" + what : "") + "?"),
                  "$2:" + getCatStr("general", "yes"), "$3:" + getCatStr("general", "no")),
    df, ds);
  return dynlen(df) > 0 && df[1] == 1;
}

//------------------------------------------------------------------------------
// overview table (tabBrokers): one line per broker, updated every second

/** Selects the line of `broker` ("" or unknown: none). */
void rsmqttPanelSelectRow(string broker)
{
  int n;
  getValue("tabBrokers", "lineCount", n);
  shape t = getShape("tabBrokers");
  for (int i = 0; i < n; i++)
  {
    string name;
    getValue("tabBrokers", "cellValueRC", i, "broker", name);
    if ( name == broker && broker != "" )
    {
      t.selectLineN(i);
      return;
    }
  }
}

void rsmqttPanelUpdateTable()
{
  dyn_string brokers = rsmqttGetAllBrokers();
  int n;
  getValue("tabBrokers", "lineCount", n);
  bool rebuild = n != dynlen(brokers);
  for (int i = 1; !rebuild && i <= n; i++)
  {
    string name;
    getValue("tabBrokers", "cellValueRC", i - 1, "broker", name);
    rebuild = name != brokers[i];
  }
  if ( rebuild )
  {
    setValue("tabBrokers", "deleteAllLines");
    for (int i = 1; i <= dynlen(brokers); i++)
      setValue("tabBrokers", "appendLine", "broker", brokers[i]);
    string shown;
    getValue("lblShown", "text", shown);
    rsmqttPanelSelectRow(shown);
  }

  // redundant project: the state of the active host, as in the form
  int host = rsmqttActiveHost();
  for (int i = 1; i <= dynlen(brokers); i++)
  {
    string dp = rsmqttBrokerDp(brokers[i]);
    string sdp = rsmqttHostDp(dp, host);
    if ( !dpExists(sdp) )
      sdp = dp;
    int state;
    string err;
    uint msgIn, msgOut;
    dpGet(sdp + ".State.ConnState", state, sdp + ".State.LastError", err,
          sdp + ".State.MessagesIn", msgIn, sdp + ".State.MessagesOut", msgOut);
    int drv = rsmqttGetBrokerDriver(brokers[i]);
    setMultiValue("tabBrokers", "cellValueRC",   i - 1, "drv",   drv > 0 ? (string) drv : "-",
                  "tabBrokers", "cellValueRC",   i - 1, "state", rsmqttConnStateText(state),
                  "tabBrokers", "cellForeColRC", i - 1, "state", rsmqttConnStateColor(state),
                  "tabBrokers", "cellValueRC",   i - 1, "io",    msgIn + " / " + msgOut,
                  "tabBrokers", "cellValueRC",   i - 1, "error", err);
  }
}

void rsmqttPanelTableClicked(int row)
{
  if ( row < 0 )
    return;
  string broker;
  getValue("tabBrokers", "cellValueRC", row, "broker", broker);
  if ( broker != "" )
    rsmqttPanelSelect(broker);
}

//------------------------------------------------------------------------------
// panel actions

/** Fills the broker list and shows `broker` (or the first one). */
void rsmqttPanelInit(string broker)
{
  setMultiValue("lblShown",    "visible", FALSE,
                "lblSnapshot", "visible", FALSE,
                "lblCopyFrom", "visible", FALSE,
                "tabBrokers",  "tableMode", TABLE_SELECT_BROWSE,
                "tabBrokers",  "selectByClick", TABLE_SELECT_LINE);
  dyn_string brokers = rsmqttGetAllBrokers();
  setMultiValue("cmbBroker", "editable", TRUE,
                "cmbBroker", "items", brokers);
  rsmqttPanelUpdateTable();
  if ( broker == "" && dynlen(brokers) > 0 )
    broker = brokers[1];
  if ( broker != "" && dynContains(brokers, broker) > 0 )
    rsmqttPanelShow(broker);
  else
    rsmqttPanelClear(broker);
  startThread("rsmqttPanelStateLoop");
}

/** An existing broker was picked in the list or the table. */
void rsmqttPanelSelect(string broker)
{
  string shown;
  getValue("lblShown", "text", shown);
  if ( broker == shown )
  {
    setValue("cmbBroker", "text", broker);
    return;
  }
  if ( !rsmqttPanelConfirmDiscard() )
  {
    setValue("cmbBroker", "text", shown);
    rsmqttPanelSelectRow(shown);
    return;
  }
  rsmqttPanelShow(broker);
}

/** "New": an empty form (after asking about unsaved changes). */
void rsmqttPanelNew()
{
  if ( !rsmqttPanelConfirmDiscard() )
    return;
  rsmqttPanelClear();
  setInputFocus(myModuleName(), myPanelName(), "cmbBroker");
}

/** Empty form with defaults for a new broker. */
void rsmqttPanelClear(string name = "")
{
  int drv = 1;
  dyn_string drivers = rsmqttDriverDps();
  if ( dynlen(drivers) > 0 )
    drv = (int) substr(dpSubStr(drivers[1], DPSUB_DP), strlen("_RsMqttDriver_"));

  setMultiValue("cmbBroker",      "text",        name,
                "spinDriver",     "text",        drv,
                "chkActive",      "state", 0,    TRUE,
                "txtUri",         "text",        "tcp://localhost:1883",
                "txtUris",        "text",        "",
                "txtClientId",    "text",        "",
                "cmbVersion",     "selectedPos", rsmqttPanelVersionPos(5),
                "txtKeepAlive",   "text",        "30",
                "chkClean",       "state", 0,    TRUE,
                "txtUser",        "text",        "",
                "txtPassword",    "text",        "",
                "txtTrust",       "text",        "",
                "txtKeyStore",    "text",        "",
                "txtKey",         "text",        "",
                "chkVerify",      "state", 0,    TRUE,
                "txtWillTopic",   "text",        "",
                "txtWillPayload", "text",        "",
                "cmbWillQos",     "selectedPos", 1,
                "chkWillRetain",  "state", 0,    FALSE,
                "cmbSubQos",      "selectedPos", 2,
                "cmbPubQos",      "selectedPos", 2,
                "chkPubRetain",   "state", 0,    FALSE,
                "txtSpbHost",     "text",        "",
                "txtSpbHost2",    "text",        "",
                "txtOptions",     "text",        "",
                "lblMessage",     "text",        "");
  rsmqttPanelSetClean("");
  rsmqttPanelUpdateState();
}

/** Shows the configuration of an existing broker. */
void rsmqttPanelShow(string broker)
{
  mapping c = rsmqttGetBrokerConfig(broker);
  if ( mappinglen(c) == 0 )
  {
    rsmqttPanelClear(broker);
    return;
  }
  // the Sparkplug host ids have their own fields
  string spb1, spb2;
  dyn_string opts, stored = c["Options"];
  for (int i = 1; i <= dynlen(stored); i++)
  {
    string o = stored[i];
    if ( rsmqttPanelOptName(o) == RSMQTT_OPT_SPB_HOST )
      spb1 = rsmqttPanelOptValue(o);
    else if ( rsmqttPanelOptName(o) == RSMQTT_OPT_SPB_HOST2 )
      spb2 = rsmqttPanelOptValue(o);
    else
      dynAppend(opts, o);
  }
  int drv = rsmqttGetBrokerDriver(broker);
  setMultiValue("cmbBroker",      "text",        broker,
                "spinDriver",     "text",        drv > 0 ? drv : 1,
                "chkActive",      "state", 0,    c["Active"],
                "txtUri",         "text",        c["ServerURI"],
                "txtUris",        "text",        rsmqttPanelJoin(c["ServerURIs"], " "),
                "txtClientId",    "text",        c["ClientId"],
                "cmbVersion",     "selectedPos", rsmqttPanelVersionPos(c["MQTTVersion"]),
                "txtKeepAlive",   "text",        c["KeepAlive"],
                "chkClean",       "state", 0,    c["CleanSession"],
                "txtUser",        "text",        c["Username"],
                "txtPassword",    "text",        c["Password"] != "" ? RSMQTT_PASSWORD_MASK : "",
                "txtTrust",       "text",        c["TlsTrustStore"],
                "txtKeyStore",    "text",        c["TlsKeyStore"],
                "txtKey",         "text",        c["TlsPrivateKey"],
                "chkVerify",      "state", 0,    c["TlsVerify"],
                "txtWillTopic",   "text",        c["WillTopic"],
                "txtWillPayload", "text",        c["WillPayload"],
                "cmbWillQos",     "selectedPos", (int) c["WillQoS"] + 1,
                "chkWillRetain",  "state", 0,    c["WillRetain"],
                "cmbSubQos",      "selectedPos", (int) c["SubscribeQoS"] + 1,
                "cmbPubQos",      "selectedPos", (int) c["PublishQoS"] + 1,
                "chkPubRetain",   "state", 0,    c["PublishRetain"],
                "txtSpbHost",     "text",        spb1,
                "txtSpbHost2",    "text",        spb2,
                "txtOptions",     "text",        rsmqttPanelJoin(opts, "; "),
                "lblMessage",     "text",        "");
  rsmqttPanelSetClean(broker);
  rsmqttPanelUpdateState();
}

/** "Copy": the form becomes a new broker with a free name; saved with Save. */
void rsmqttPanelCopy()
{
  string shown;
  getValue("lblShown", "text", shown);
  if ( shown == "" || !dpExists(rsmqttBrokerDp(shown)) )
  {
    rsmqttPanelMessage(rsmqttT("Erst einen gespeicherten Broker auswählen.", "Pick a saved broker first."), TRUE);
    return;
  }
  string name = shown + "_copy";
  for (int n = 2; dpExists(rsmqttBrokerDp(name)); n++)
    name = shown + "_copy" + n;
  // the client id must differ, or the broker disconnects one of the two; a
  // Sparkplug host id too, or both publish the same STATE with their own will
  setMultiValue("cmbBroker",   "text", name,
                "txtClientId", "text", "",
                "txtSpbHost",  "text", "",
                "txtSpbHost2", "text", "",
                "lblShown",    "text", "",
                "lblSnapshot", "text", "",   // unsaved until Save
                "lblCopyFrom", "text", shown);
  rsmqttPanelSelectRow("");
  rsmqttPanelMessage(rsmqttT("Kopie von '" + shown + "' (Client-ID und Sparkplug-Host-IDs geleert): Namen prüfen und speichern.",
                             "Copy of '" + shown + "' (client ID and Sparkplug host ids cleared): check the name and save."), FALSE);
  rsmqttPanelUpdateState();
}

void rsmqttPanelMessage(string text, bool error)
{
  setMultiValue("lblMessage", "text", text,
                "lblMessage", "foreCol", error ? "{200,0,0}" : "{0,120,0}");
}

/** Validates and saves the form; returns 0 on success. */
int rsmqttPanelSave()
{
  string broker, uri, uris, clientId, keepAlive, user, password, trust, keyStore, key,
         willTopic, willPayload, options, drvText, spb1, spb2, copyFrom;
  bool active, clean, verify, willRetain, pubRetain;
  int versionPos, willQosPos, subQosPos, pubQosPos;

  getMultiValue("cmbBroker",      "text",        broker,
                "spinDriver",     "text",        drvText,
                "chkActive",      "state", 0,    active,
                "txtUri",         "text",        uri,
                "txtUris",        "text",        uris,
                "txtClientId",    "text",        clientId,
                "cmbVersion",     "selectedPos", versionPos,
                "txtKeepAlive",   "text",        keepAlive,
                "chkClean",       "state", 0,    clean,
                "txtUser",        "text",        user,
                "txtPassword",    "text",        password,
                "txtTrust",       "text",        trust,
                "txtKeyStore",    "text",        keyStore,
                "txtKey",         "text",        key,
                "chkVerify",      "state", 0,    verify,
                "txtWillTopic",   "text",        willTopic,
                "txtWillPayload", "text",        willPayload,
                "cmbWillQos",     "selectedPos", willQosPos,
                "chkWillRetain",  "state", 0,    willRetain,
                "cmbSubQos",      "selectedPos", subQosPos,
                "cmbPubQos",      "selectedPos", pubQosPos,
                "chkPubRetain",   "state", 0,    pubRetain,
                "txtSpbHost",     "text",        spb1,
                "txtSpbHost2",    "text",        spb2,
                "txtOptions",     "text",        options,
                "lblCopyFrom",    "text",        copyFrom);

  broker = strltrim(strrtrim(broker));
  uri = strltrim(strrtrim(uri));
  spb1 = strltrim(strrtrim(spb1));
  spb2 = strltrim(strrtrim(spb2));
  int drv = (int) drvText;
  bool isNew = !dpExists(rsmqttBrokerDp(broker));

  if ( !rsmqttIsValidBrokerName(broker) )
  {
    rsmqttPanelMessage(rsmqttT("Broker-Name: nur Buchstaben, Ziffern, '_' und '-'.",
                               "Broker name: letters, digits, '_' and '-' only."), TRUE);
    return -1;
  }
  if ( isNew && !rsmqttIsNewBrokerName(broker) )
  {
    rsmqttPanelMessage(rsmqttT("Ein neuer Broker-Name darf nicht auf \"_2\" enden (für Redundanz reserviert).",
                               "A new broker name must not end with \"_2\" (reserved for redundancy)."), TRUE);
    return -1;
  }
  if ( uri == "" || strpos(uri, "://") < 1 )
  {
    rsmqttPanelMessage(rsmqttT("Server-URI wie tcp://host:1883, mqtts://host:8883 oder ws://host/mqtt.",
                               "Server URI like tcp://host:1883, mqtts://host:8883 or ws://host/mqtt."), TRUE);
    return -1;
  }
  if ( drv < 1 || drv > 255 )
  {
    rsmqttPanelMessage(rsmqttT("Treibernummer 1..255.", "Driver number must be 1..255."), TRUE);
    return -1;
  }
  int ka = 0;
  if ( strltrim(strrtrim(keepAlive)) != "" )
  {
    ka = (int) keepAlive;
    if ( ka <= 0 || ka > 65535 )
    {
      rsmqttPanelMessage(rsmqttT("Keep alive 1..65535 Sekunden (leer = Vorgabe der Bibliothek).",
                                 "Keep alive must be 1..65535 seconds (empty = library default)."), TRUE);
      return -1;
    }
  }
  if ( !rsmqttPanelValidHostId(spb1) || !rsmqttPanelValidHostId(spb2) )
  {
    rsmqttPanelMessage(rsmqttT("Sparkplug-Host-ID ohne / + # ;", "Sparkplug host id without / + # ;"), TRUE);
    return -1;
  }
  dyn_string opts = rsmqttPanelSplit(options, ";\n");
  for (int i = 1; i <= dynlen(opts); i++)
  {
    if ( strpos(opts[i], "=") < 1 )
    {
      rsmqttPanelMessage(rsmqttT("Option '" + opts[i] + "' ist nicht name=wert.", "Option '" + opts[i] + "' is not name=value."), TRUE);
      return -1;
    }
    string n = rsmqttPanelOptName(opts[i]);
    if ( n == RSMQTT_OPT_SPB_HOST || n == RSMQTT_OPT_SPB_HOST2 )
    {
      rsmqttPanelMessage(rsmqttT("Die Sparkplug-Host-ID gehört in die Sparkplug-Felder, nicht in die Optionen.",
                                 "Put the Sparkplug host id into the Sparkplug fields, not into the options."), TRUE);
      return -1;
    }
  }
  if ( spb1 != "" ) dynAppend(opts, "sparkplug.hostId=" + spb1);
  if ( spb2 != "" ) dynAppend(opts, "redu.sparkplug.hostId=" + spb2);

  // the mask stands for the stored password: of this broker, or of the one copied
  bool keepPassword = password == RSMQTT_PASSWORD_MASK;
  if ( keepPassword && isNew && (copyFrom == "" || !dpExists(rsmqttBrokerDp(copyFrom))) )
  {
    rsmqttPanelMessage(rsmqttT("Bitte das Passwort neu eingeben.", "Please enter the password again."), TRUE);
    return -1;
  }

  mapping cfg = makeMapping(
    "Active",        active,
    "ServerURIs",    rsmqttPanelSplit(uris, " ,;"),
    "ClientId",      strltrim(strrtrim(clientId)),
    "MQTTVersion",   rsmqttPanelPosVersion(versionPos),
    "KeepAlive",     ka,
    "CleanSession",  clean,
    "Username",      user,
    "Password",      password,
    "TlsTrustStore", strltrim(strrtrim(trust)),
    "TlsKeyStore",   strltrim(strrtrim(keyStore)),
    "TlsPrivateKey", strltrim(strrtrim(key)),
    "TlsVerify",     verify,
    "WillTopic",     strltrim(strrtrim(willTopic)),
    "WillPayload",   willPayload,
    "WillQoS",       willQosPos - 1,
    "WillRetain",    willRetain,
    "SubscribeQoS",  subQosPos - 1,
    "PublishQoS",    pubQosPos - 1,
    "PublishRetain", pubRetain,
    "Options",       opts);
  if ( keepPassword )
    mappingRemove(cfg, "Password");

  if ( rsmqttCreateBroker(broker, drv, uri, cfg) != 0 )
  {
    rsmqttPanelMessage(rsmqttT("Speichern fehlgeschlagen, siehe Log.", "Saving failed, see the log."), TRUE);
    return -1;
  }
  // a copy: the stored (encrypted) password of the original
  if ( keepPassword && isNew )
  {
    string stored;
    dpGet(rsmqttBrokerDp(copyFrom) + ".Config.Password", stored);
    dpSetWait(rsmqttBrokerDp(broker) + ".Config.Password", stored);
  }

  if ( isNew )
  {
    dyn_string brokers = rsmqttGetAllBrokers();
    setMultiValue("cmbBroker", "items", brokers,
                  "cmbBroker", "text",  broker);
    rsmqttPanelUpdateTable();
  }
  setMultiValue("txtPassword", "text", password != "" ? RSMQTT_PASSWORD_MASK : "",
                "txtSpbHost",  "text", spb1,
                "txtSpbHost2", "text", spb2);
  rsmqttPanelSetClean(broker);
  rsmqttPanelMessage((isNew ? rsmqttT("Broker angelegt", "Broker created") : rsmqttT("Gespeichert", "Saved")) +
                     rsmqttT(" - Treiber " + drv + " übernimmt ihn jetzt.", " - driver " + drv + " applies it now."), FALSE);
  rsmqttPanelUpdateState();
  return 0;
}

/** Asks and deletes the broker shown in the panel. */
void rsmqttPanelDelete()
{
  string broker;
  getValue("cmbBroker", "text", broker);
  if ( !dpExists(rsmqttBrokerDp(broker)) )
  {
    rsmqttPanelMessage(rsmqttT("Broker '" + broker + "' gibt es nicht.", "Broker '" + broker + "' does not exist."), TRUE);
    return;
  }
  dyn_float df;
  dyn_string ds;
  ChildPanelOnCentralModalReturn("vision/MessageInfo", "rsmqtt",
    makeDynString("$1:" + rsmqttT("Broker '" + broker + "' löschen?\nAdressen, die ihn verwenden, funktionieren dann nicht mehr.",
                                  "Delete broker '" + broker + "'?\nAddresses using it stop working."),
                  "$2:" + getCatStr("general", "yes"), "$3:" + getCatStr("general", "no")),
    df, ds);
  if ( dynlen(df) < 1 || df[1] != 1 )
    return;

  if ( rsmqttDeleteBroker(broker) != 0 )
  {
    rsmqttPanelMessage(rsmqttT("Löschen fehlgeschlagen, siehe Log.", "Deleting failed, see the log."), TRUE);
    return;
  }
  dyn_string brokers = rsmqttGetAllBrokers();
  setValue("cmbBroker", "items", brokers);
  rsmqttPanelUpdateTable();
  if ( dynlen(brokers) > 0 )
    rsmqttPanelShow(brokers[1]);
  else
    rsmqttPanelClear();
  rsmqttPanelMessage(rsmqttT("Broker '" + broker + "' gelöscht.", "Broker '" + broker + "' deleted."), FALSE);
}

void rsmqttPanelReconnect()
{
  string broker;
  getValue("cmbBroker", "text", broker);
  if ( rsmqttReconnectBroker(broker) == 0 )
    rsmqttPanelMessage(rsmqttT("Neu verbinden angefordert.", "Reconnect requested."), FALSE);
  else
    rsmqttPanelMessage(rsmqttT("Broker '" + broker + "' gibt es nicht.", "Broker '" + broker + "' does not exist."), TRUE);
}

/** Shows State.* of the broker and the Sparkplug STATE topics in the panel. */
void rsmqttPanelUpdateState()
{
  string broker, spb1, spb2;
  getMultiValue("cmbBroker", "text", broker, "txtSpbHost", "text", spb1, "txtSpbHost2", "text", spb2);
  spb1 = strltrim(strrtrim(spb1));
  spb2 = strltrim(strrtrim(spb2));
  string spb;
  if ( spb1 == "" && spb2 == "" )
    spb = rsmqttT("Keine Host Application: Sparkplug-Adressen lesen und schreiben trotzdem.",
                  "No host application: Sparkplug addresses read and write all the same.");
  else
    spb = "STATE: " + (spb1 != "" ? "spBv1.0/STATE/" + spb1 : "-") +
          (spb2 != "" ? rsmqttT("   (2. Host: ", "   (2nd host: ") + "spBv1.0/STATE/" + spb2 + ")" : "");
  setValue("lblSpbState", "text", spb);

  string dp = rsmqttBrokerDp(broker);
  if ( broker == "" || !dpExists(dp) )
  {
    setMultiValue("lblState", "text",    rsmqttT("neuer Broker - noch nicht gespeichert", "new broker - not saved yet"),
                  "lblState", "foreCol", "_3DText",
                  "lblError", "text",    "",
                  "lblInfo",  "text",    "");
    return;
  }

  // redundant project: the details of the active host, the state of both
  int host = rsmqttActiveHost();
  string sdp = rsmqttHostDp(dp, host);
  if ( !dpExists(sdp) )
    sdp = dp;
  int state;
  string err, uri, clientId;
  uint msgIn, msgOut;
  dpGet(sdp + ".State.ConnState", state, sdp + ".State.LastError", err,
        sdp + ".State.ServerURI", uri, sdp + ".State.ClientId", clientId,
        sdp + ".State.MessagesIn", msgIn, sdp + ".State.MessagesOut", msgOut);

  int drv = rsmqttGetBrokerDriver(broker);
  string info = (uri != "" ? uri : "-") + rsmqttT("   Client-ID: ", "   client id: ") + (clientId != "" ? clientId : "-") +
                rsmqttT("   Nachrichten ein/aus: ", "   messages in/out: ") + msgIn + " / " + msgOut;
  string hostText = isRedundant() ? rsmqttT("Host ", "host ") + host + ": " : "";
  string drvText = drv > 0 ? rsmqttT("   (Treiber " + drv + ")", "   (driver " + drv + ")")
                           : rsmqttT("   (keinem Treiber zugeordnet)", "   (not assigned to a driver)");
  setMultiValue("lblState", "text", hostText + rsmqttConnStateText(state) + drvText + rsmqttOtherHostState(dp, host),
                "lblState", "foreCol", rsmqttConnStateColor(state),
                "lblError", "text", err,
                "lblInfo",  "text", info);
}

void rsmqttPanelStateLoop()
{
  while ( TRUE )
  {
    delay(1);
    rsmqttPanelUpdateState();
    rsmqttPanelUpdateTable();
  }
}
