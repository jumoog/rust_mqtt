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
// Brokers

/** DP name of a broker. */
string rsmqttBrokerDp(string broker)
{
  return "_RsMqtt_" + broker;
}

/** DP name of the driver DP that lists the brokers of a driver number. */
string rsmqttDriverDp(int driverNum)
{
  return "_RsMqttDriver_" + driverNum;
}

/** All broker names (from the _RsMqttBroker DPs of the local system). */
dyn_string rsmqttGetAllBrokers()
{
  dyn_string dps = dpNames("_RsMqtt_*", "_RsMqttBroker"), names;
  for (int i = 1; i <= dynlen(dps); i++)
  {
    string n = dpSubStr(dps[i], DPSUB_DP);
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
  dyn_string dps = dpNames("_RsMqttDriver_*", "_RsMqttDriver");
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
    if ( dpCreate(dp, "_RsMqttBroker") != 0 || !dpExists(dp) )
    {
      DebugTN("rsmqttCreateBroker: cannot create", dp, getLastError());
      return -1;
    }
    // defaults
    dpSetWait(dp + ".Config.MQTTVersion", 5,
              dp + ".Config.KeepAlive", 30,
              dp + ".Config.CleanSession", TRUE,
              dp + ".Config.SubscribeQoS", 1,
              dp + ".Config.PublishQoS", 1,
              dp + ".Config.PublishRetain", FALSE);
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
  dyn_string dps = dpNames("_RsMqttDriver_*", "_RsMqttDriver");
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
  dyn_string dps = dpNames("_RsMqttDriver_*", "_RsMqttDriver");
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
    return dpDelete(dp) == 0 ? 0 : -1;
  }
  return 0;
}

/** Asks the driver to drop and re-open the connection. */
int rsmqttReconnectBroker(string broker)
{
  return dpSetWait(rsmqttBrokerDp(broker) + ".Command.Reconnect", TRUE) == 0 ? 0 : -1;
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

  int state;
  string err, uri;
  dpGet(dp + ".State.ConnState", state,
        dp + ".State.LastError", err,
        dp + ".Config.ServerURI", uri);
  return uri + "  -  " + rsmqttConnStateText(state) + (err != "" ? "  (" + err + ")" : "");
}

string rsmqttConnStateText(int state)
{
  switch ( state )
  {
    case RSMQTT_CS_DISABLED:     return "disabled";
    case RSMQTT_CS_DISCONNECTED: return "disconnected";
    case RSMQTT_CS_CONNECTING:   return "connecting";
    case RSMQTT_CS_CONNECTED:    return "connected";
    case RSMQTT_CS_FAILURE:      return "failure";
  }
  return "unknown";
}

//------------------------------------------------------------------------------
// Addresses

/** Builds an address reference.
  @param qos     -1 = broker default
  @param retain  -1 = broker default, 0, 1
  @param extra   further options, e.g. "nolocal;rh=1"
*/
string rsmqttMakeReference(string broker, string topic, int qos = -1, int retain = -1, string extra = "")
{
  string ref = broker + ":" + topic;
  if ( qos >= 0 )    ref += ";qos=" + qos;
  if ( retain >= 0 ) ref += ";retain=" + retain;
  if ( extra != "" ) ref += ";" + extra;
  return ref;
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
  @param reference  "<broker>:<topic>[;qos=N][;retain][;nolocal][;rap][;rh=N]"
  @param datatype   RSMQTT_STRING .. RSMQTT_BLOB
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

/** Fills the broker list and shows `broker` (or the first one). */
void rsmqttPanelInit(string broker)
{
  dyn_string brokers = rsmqttGetAllBrokers();
  setMultiValue("cmbBroker", "editable", TRUE,
                "cmbBroker", "items", brokers);
  if ( broker == "" && dynlen(brokers) > 0 )
    broker = brokers[1];
  if ( broker != "" && dynContains(brokers, broker) > 0 )
    rsmqttPanelShow(broker);
  else
    rsmqttPanelClear(broker);
  startThread("rsmqttPanelStateLoop");
}

/** Empty form with defaults for a new broker. */
void rsmqttPanelClear(string name = "")
{
  int drv = 1;
  dyn_string drivers = dpNames("_RsMqttDriver_*", "_RsMqttDriver");
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
                "chkVerify",      "state", 0,    FALSE,
                "txtWillTopic",   "text",        "",
                "txtWillPayload", "text",        "",
                "cmbWillQos",     "selectedPos", 1,
                "chkWillRetain",  "state", 0,    FALSE,
                "cmbSubQos",      "selectedPos", 2,
                "cmbPubQos",      "selectedPos", 2,
                "chkPubRetain",   "state", 0,    FALSE,
                "txtOptions",     "text",        "",
                "lblMessage",     "text",        "");
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
                "txtOptions",     "text",        rsmqttPanelJoin(c["Options"], "; "),
                "lblMessage",     "text",        "");
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
         willTopic, willPayload, options, drvText;
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
                "txtOptions",     "text",        options);

  broker = strltrim(strrtrim(broker));
  uri = strltrim(strrtrim(uri));
  int drv = (int) drvText;

  if ( !rsmqttIsValidBrokerName(broker) )
  {
    rsmqttPanelMessage("Broker name: letters, digits, '_' and '-' only.", TRUE);
    return -1;
  }
  if ( uri == "" || strpos(uri, "://") < 1 )
  {
    rsmqttPanelMessage("Server URI like tcp://host:1883, mqtts://host:8883 or ws://host/mqtt.", TRUE);
    return -1;
  }
  if ( drv < 1 || drv > 255 )
  {
    rsmqttPanelMessage("Driver number must be 1..255.", TRUE);
    return -1;
  }
  int ka = 0;
  if ( strltrim(strrtrim(keepAlive)) != "" )
  {
    ka = (int) keepAlive;
    if ( ka <= 0 || ka > 65535 )
    {
      rsmqttPanelMessage("Keep alive must be 1..65535 seconds (empty = library default).", TRUE);
      return -1;
    }
  }
  dyn_string opts = rsmqttPanelSplit(options, ";\n");
  for (int i = 1; i <= dynlen(opts); i++)
  {
    if ( strpos(opts[i], "=") < 1 )
    {
      rsmqttPanelMessage("Option '" + opts[i] + "' is not name=value.", TRUE);
      return -1;
    }
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

  // unchanged mask: keep the stored (encrypted) password
  if ( password == RSMQTT_PASSWORD_MASK && dpExists(rsmqttBrokerDp(broker)) )
    mappingRemove(cfg, "Password");

  bool isNew = !dpExists(rsmqttBrokerDp(broker));
  if ( rsmqttCreateBroker(broker, drv, uri, cfg) != 0 )
  {
    rsmqttPanelMessage("Saving failed, see the log.", TRUE);
    return -1;
  }

  if ( isNew )
  {
    dyn_string brokers = rsmqttGetAllBrokers();
    setMultiValue("cmbBroker", "items", brokers,
                  "cmbBroker", "text",  broker);
  }
  setValue("txtPassword", "text", password != "" ? RSMQTT_PASSWORD_MASK : "");
  rsmqttPanelMessage((isNew ? "Broker created" : "Saved") + " - driver " + drv + " applies it now.", FALSE);
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
    rsmqttPanelMessage("Broker '" + broker + "' does not exist.", TRUE);
    return;
  }
  dyn_float df;
  dyn_string ds;
  ChildPanelOnCentralModalReturn("vision/MessageInfo", "rsmqtt",
    makeDynString("$1:Delete broker '" + broker + "'?\nAddresses using it stop working.",
                  "$2:" + getCatStr("general", "yes"), "$3:" + getCatStr("general", "no")),
    df, ds);
  if ( dynlen(df) < 1 || df[1] != 1 )
    return;

  if ( rsmqttDeleteBroker(broker) != 0 )
  {
    rsmqttPanelMessage("Deleting failed, see the log.", TRUE);
    return;
  }
  dyn_string brokers = rsmqttGetAllBrokers();
  setValue("cmbBroker", "items", brokers);
  if ( dynlen(brokers) > 0 )
    rsmqttPanelShow(brokers[1]);
  else
    rsmqttPanelClear();
  rsmqttPanelMessage("Broker '" + broker + "' deleted.", FALSE);
}

void rsmqttPanelReconnect()
{
  string broker;
  getValue("cmbBroker", "text", broker);
  if ( rsmqttReconnectBroker(broker) == 0 )
    rsmqttPanelMessage("Reconnect requested.", FALSE);
  else
    rsmqttPanelMessage("Broker '" + broker + "' does not exist.", TRUE);
}

/** Shows State.* of the broker in the panel. */
void rsmqttPanelUpdateState()
{
  string broker;
  getValue("cmbBroker", "text", broker);
  string dp = rsmqttBrokerDp(broker);
  if ( broker == "" || !dpExists(dp) )
  {
    setMultiValue("lblState", "text", "new broker - not saved yet",
                  "lblState", "foreCol", "_3DText",
                  "lblError", "text", "",
                  "lblInfo",  "text", "");
    return;
  }

  int state;
  string err, uri, clientId;
  uint msgIn, msgOut;
  dpGet(dp + ".State.ConnState", state, dp + ".State.LastError", err,
        dp + ".State.ServerURI", uri, dp + ".State.ClientId", clientId,
        dp + ".State.MessagesIn", msgIn, dp + ".State.MessagesOut", msgOut);

  string col = "_3DText";
  switch ( state )
  {
    case RSMQTT_CS_CONNECTED:    col = "{0,140,0}";   break;
    case RSMQTT_CS_CONNECTING:   col = "{200,130,0}"; break;
    case RSMQTT_CS_DISCONNECTED: col = "{200,130,0}"; break;
    case RSMQTT_CS_FAILURE:      col = "{200,0,0}";   break;
  }
  int drv = rsmqttGetBrokerDriver(broker);
  string info = (uri != "" ? uri : "-") + "   client id: " + (clientId != "" ? clientId : "-") +
                "   messages in/out: " + msgIn + " / " + msgOut;
  setMultiValue("lblState", "text", rsmqttConnStateText(state) + (drv > 0 ? "   (driver " + drv + ")" : "   (not assigned to a driver)"),
                "lblState", "foreCol", col,
                "lblError", "text", err,
                "lblInfo",  "text", info);
}

void rsmqttPanelStateLoop()
{
  while ( TRUE )
  {
    delay(1);
    rsmqttPanelUpdateState();
  }
}
