// rsmqttService.ctl - MQTT topics in CTRL through the Manager Service API
// (MSA) of the WCCOArsmqtt driver: the driver hosts the vRPC service "RsMqtt"
// and uses its broker connections for it, no datapoints are needed.
//
//   #uses "rsmqttService"
//
//   void onTemp(string topic, string payload, mapping info)
//   {
//     DebugTN(info["broker"], topic, payload, info["retained"]);
//   }
//
//   int h = rsmqttTopicConnect("onTemp", "plant", "line1/+/temp");
//   rsmqttPublish("plant", "line1/pump1/cmd", "{\"start\":true}");
//   rsmqttTopicDisconnect(h);
//
// The callback runs in a thread of the script that called
// rsmqttTopicConnect(), one message after the other, in the order the
// driver received them. Messages that arrive while it runs are queued in the
// driver (up to 10000 messages or 64 MB per subscription, then the oldest are
// dropped and info["dropped"] counts them). If the driver restarts, it
// restores the subscription and the callback gets the messages that arrived
// meanwhile (see the README for what the broker keeps while the driver is
// down); if it cannot, the subscription is made again automatically.

#uses "vrpc"
#uses "rsmqtt"

const string RSMQTT_SERVICE        = "RsMqtt";
const int    RSMQTT_RECEIVE_WAIT   = 10;    // s a Receive call waits for messages in the driver
const int    RSMQTT_RECEIVE_MAX    = 1000;  // messages per Receive
const int    RSMQTT_CALL_TIMEOUT   = 10;    // s for Subscribe / Unsubscribe / Renew / Publish
const int    RSMQTT_RETRY_DELAY    = 2;     // s between attempts while the driver is not reachable
const int    RSMQTT_RENEW_INTERVAL = 30;    // s: long callback runs renew the subscription (the driver drops it after 120 s)
const int    RSMQTT_DRIVER_CACHE   = 30;    // s a broker's driver number is cached
const int    RSMQTT_UNSUBSCRIBE_RETRY = 300;  // s a failed Unsubscribe is tried again (driver not running, busy)

// handle -> mapping(broker, topic, qos?, binary, driver, callback, id) of
// rsmqttTopicConnect(); id: the subscription in the driver ("" = none)
mapping g_rsmqttTopics;
int     g_rsmqttTopicNext = 0;

mapping g_rsmqttStubs;         // driver number -> shared_ptr<VrpcStub>
mapping g_rsmqttBrokerDriver;  // broker -> mapping(driver, time) (rsmqttGetBrokerDriver)

/** Calls `callback` for every MQTT message on `topic` of `broker`.
  @param callback  function name or function_ptr:
                     void cb(string topic, anytype payload, mapping info)
                   the payload is a string, or a blob with "binary": TRUE or
                   when it is no valid UTF-8 text or holds NUL bytes (then
                   info["binary"] is TRUE); declare it `string` only for topics
                   that carry text.
                   info: "broker", "driver", "qos", "retained", "binary", "time",
                   "handle", "dropped" (messages lost before this one because the
                   queue was full)
  @param broker    broker name (as in addresses)
  @param topic     topic or filter (+, #, $share/<group>/...)
  @param options   optional: "qos" (default: Config.SubscribeQoS of the broker),
                   "binary" (payload as blob), "driver" (driver number; default:
                   the driver whose Brokers list has the broker; needed for
                   brokers from the config file)
  @return handle for rsmqttTopicDisconnect() (> 0), -1 on error (logged)
*/
int rsmqttTopicConnect(anytype callback, string broker, string topic, mapping options = makeMapping())
{
  int driverNum = mappingHasKey(options, "driver") ? (int) options["driver"] : _rsmqttBrokerDriver(broker);
  if ( driverNum <= 0 )
  {
    DebugTN("rsmqttTopicConnect: no driver serves broker '" + broker + "' (give options[\"driver\"])");
    return -1;
  }
  mapping t = makeMapping("broker", broker, "topic", topic, "driver", driverNum, "callback", callback,
                          "binary", mappingHasKey(options, "binary") ? (bool) options["binary"] : FALSE);
  if ( mappingHasKey(options, "qos") )
    t["qos"] = (int) options["qos"];

  string id, err;
  if ( _rsmqttServiceSubscribe(t, id, err) != 0 )
  {
    DebugTN("rsmqttTopicConnect: " + broker + ":" + topic + ": " + err);
    return -1;
  }
  t["id"] = id;

  int handle;
  synchronized(g_rsmqttTopics)
  {
    handle = ++g_rsmqttTopicNext;
    g_rsmqttTopics[handle] = t;
  }
  startThread("_rsmqttTopicLoop", handle);
  return handle;
}

/** Ends a subscription of rsmqttTopicConnect(). The callback is not called
  again (a call that is running finishes).
  @return 0 on success, -1 for an unknown handle
*/
int rsmqttTopicDisconnect(int handle)
{
  mapping t;
  synchronized(g_rsmqttTopics)
  {
    if ( !mappingHasKey(g_rsmqttTopics, handle) )
      return -1;
    t = g_rsmqttTopics[handle];
    mappingRemove(g_rsmqttTopics, handle);
  }
  // also ends the Receive the loop waits for. If the call fails (driver not
  // running, busy) a thread tries again, or the subscription would live on
  // in the driver (and a restarted driver would restore it).
  if ( _rsmqttUnsubscribeFailed(_rsmqttUnsubscribe(t["driver"], t["id"])) )
    startThread("_rsmqttUnsubscribeLater", t["driver"], t["id"]);
  return 0;
}

/** Handles of rsmqttTopicConnect() of this manager. */
dyn_int rsmqttTopicHandles()
{
  dyn_int handles;
  synchronized(g_rsmqttTopics)
  {
    for (int i = 1; i <= mappinglen(g_rsmqttTopics); i++)
      dynAppend(handles, mappingGetKey(g_rsmqttTopics, i));
  }
  return handles;
}

/** Publishes a message through the driver's connection to `broker`.
  @param payload  string (or blob)
  @param qos      -1: Config.PublishQoS of the broker
  @param retain   -1: Config.PublishRetain of the broker, 0 / 1
  @param driverNum 0: the driver whose Brokers list has the broker (cached for
                   30 s; in a callback info["driver"] can be passed)
  @return 0 on success (handed to the MQTT client), -1 on error (logged)
*/
int rsmqttPublish(string broker, string topic, anytype payload, int qos = -1, int retain = -1, int driverNum = 0)
{
  if ( driverNum <= 0 )
    driverNum = _rsmqttBrokerDriver(broker);
  if ( driverNum <= 0 )
  {
    DebugTN("rsmqttPublish: no driver serves broker '" + broker + "'");
    return -1;
  }
  mapping req = makeMapping("broker", broker, "topic", topic, "payload", payload);
  if ( qos >= 0 )
    req["qos"] = qos;
  if ( retain >= 0 )
    req["retain"] = retain != 0;
  anytype resp;
  string err;
  if ( _rsmqttServiceCall(driverNum, "Publish", req, resp, err, RSMQTT_CALL_TIMEOUT) != 0 )
  {
    DebugTN("rsmqttPublish: " + broker + ":" + topic + ": " + err);
    return -1;
  }
  return 0;
}

//--------------------------------------------------------------------------------
// internal

/** Calls a function of the driver's service.
  @return 0 (OK) or the VrpcStatusCode (14 Unavailable: driver not running, 5 NotFound, ...) */
int _rsmqttServiceCall(int driverNum, string method, mapping req, anytype &resp, string &err, int timeout)
{
  try
  {
    shared_ptr<VrpcClientContext> ctx = VrpcClientContext::create();
    ctx.setDeadline(getCurrentTime() + timeout);
    VrpcResponseData r = _rsmqttStub(driverNum).callFunction(method, req, ctx);
    VrpcStatus status = r.getStatus();
    if ( !status.isOk() )
    {
      err = status.toString();
      return (int) status.getStatusCode();
    }
    resp = r.getResponse();
    return 0;
  }
  catch
  {
    // the stub could not be set up (e.g. no service of that driver)
    err = getErrorText(getLastException());
    return (int) VrpcStatusCode::Unavailable;
  }
}

/** One stub per driver number, shared by all calls of the manager. */
shared_ptr<VrpcStub> _rsmqttStub(int driverNum)
{
  synchronized(g_rsmqttStubs)
  {
    if ( !mappingHasKey(g_rsmqttStubs, driverNum) )
    {
      VrpcStubOptions options = VrpcStubOptions();
      options.withManagerSpecificService(DRIVER_MAN, driverNum);
      shared_ptr<VrpcStub> stub = new VrpcStub(RSMQTT_SERVICE, options);
      stub.setThrowExceptionOnError(FALSE);
      g_rsmqttStubs[driverNum] = stub;
    }
    return g_rsmqttStubs[driverNum];
  }
}

/** rsmqttGetBrokerDriver(), cached for RSMQTT_DRIVER_CACHE seconds. */
int _rsmqttBrokerDriver(string broker)
{
  synchronized(g_rsmqttBrokerDriver)
  {
    if ( mappingHasKey(g_rsmqttBrokerDriver, broker) )
    {
      mapping c = g_rsmqttBrokerDriver[broker];
      if ( c["driver"] > 0 && period(getCurrentTime()) - period(c["time"]) < RSMQTT_DRIVER_CACHE )
        return c["driver"];
    }
  }
  int driverNum = rsmqttGetBrokerDriver(broker);
  synchronized(g_rsmqttBrokerDriver)
  {
    g_rsmqttBrokerDriver[broker] = makeMapping("driver", driverNum, "time", getCurrentTime());
  }
  return driverNum;
}

/** Subscribes `t` (a g_rsmqttTopics entry) in the driver: 0 and its id, else the status code. */
int _rsmqttServiceSubscribe(const mapping &t, string &id, string &err)
{
  mapping req = makeMapping("broker", t["broker"], "topic", t["topic"], "binary", t["binary"]);
  if ( mappingHasKey(t, "qos") )
    req["qos"] = t["qos"];
  anytype resp;
  int rc = _rsmqttServiceCall(t["driver"], "Subscribe", req, resp, err, RSMQTT_CALL_TIMEOUT);
  if ( rc == 0 )
  {
    mapping m = resp;
    id = m["id"];
  }
  return rc;
}

/** Ends a subscription in the driver: 0 or the status code. */
int _rsmqttUnsubscribe(int driverNum, string id)
{
  if ( id == "" )
    return 0;
  anytype resp;
  string err;
  return _rsmqttServiceCall(driverNum, "Unsubscribe", makeMapping("id", id), resp, err, RSMQTT_CALL_TIMEOUT);
}

/** TRUE if an Unsubscribe must be tried again: it failed, and not because
  the driver does not know the id (then there is nothing left to end). */
bool _rsmqttUnsubscribeFailed(int rc)
{
  return rc != 0 && rc != (int) VrpcStatusCode::NotFound;
}

/** An Unsubscribe that failed (driver not running, busy): tried again until
  the driver answers or RSMQTT_UNSUBSCRIBE_RETRY passes (by then the driver
  has dropped the subscription itself, as no one asked for its messages). */
void _rsmqttUnsubscribeLater(int driverNum, string id)
{
  time until = getCurrentTime() + RSMQTT_UNSUBSCRIBE_RETRY;
  while ( getCurrentTime() < until )
  {
    delay(RSMQTT_RETRY_DELAY);
    if ( !_rsmqttUnsubscribeFailed(_rsmqttUnsubscribe(driverNum, id)) )
      return;
  }
}

/** The driver id of a handle; FALSE if the handle was disconnected. */
bool _rsmqttTopicId(int handle, string &id)
{
  synchronized(g_rsmqttTopics)
  {
    if ( !mappingHasKey(g_rsmqttTopics, handle) )
      return FALSE;
    mapping t = g_rsmqttTopics[handle];
    id = t["id"];
    return TRUE;
  }
}

/** Sets the driver id of a handle (`old`: only if it still is that one);
  FALSE if the handle was disconnected. */
bool _rsmqttTopicSetId(int handle, string id, string old = "")
{
  synchronized(g_rsmqttTopics)
  {
    if ( !mappingHasKey(g_rsmqttTopics, handle) )
      return FALSE;
    mapping t = g_rsmqttTopics[handle];
    if ( old == "" || t["id"] == old )
    {
      t["id"] = id;
      g_rsmqttTopics[handle] = t;
    }
    return TRUE;
  }
}

/** Subscribes a handle again (driver restarted, subscription expired); "" while that fails. */
string _rsmqttTopicResubscribe(int handle, const mapping &t, bool &warned)
{
  string id, err;
  if ( _rsmqttServiceSubscribe(t, id, err) != 0 )
  {
    if ( !warned )
      DebugTN("rsmqttTopicConnect: " + t["broker"] + ":" + t["topic"] + ": " + err + " (retrying)");
    warned = TRUE;
    delay(RSMQTT_RETRY_DELAY);
    return "";
  }
  warned = FALSE;
  if ( !_rsmqttTopicSetId(handle, id) )
    _rsmqttUnsubscribe(t["driver"], id);  // disconnected meanwhile
  return id;
}

/** Calls the callback for a batch of messages; FALSE if the handle was disconnected. */
bool _rsmqttTopicDispatch(int handle, const mapping &t, string id, dyn_mapping messages, int dropped)
{
  mapping info = makeMapping("broker", t["broker"], "driver", t["driver"], "handle", handle);
  time lastContact = getCurrentTime();
  for (int i = 1; i <= dynlen(messages); i++)
  {
    string current;
    if ( !_rsmqttTopicId(handle, current) )
      return FALSE;
    // callbacks that take long: keep the subscription in the driver
    if ( period(getCurrentTime()) - period(lastContact) >= RSMQTT_RENEW_INTERVAL )
    {
      anytype r;
      string err;
      _rsmqttServiceCall(t["driver"], "Renew", makeMapping("id", id), r, err, RSMQTT_CALL_TIMEOUT);
      lastContact = getCurrentTime();
    }
    info["qos"]      = messages[i]["qos"];
    info["retained"] = messages[i]["retained"];
    info["binary"]   = messages[i]["binary"];
    info["time"]     = messages[i]["time"];
    info["dropped"]  = i == 1 ? dropped : 0;
    // startThread, not callFunction: code of a library cannot call a
    // function of the script by its name, a thread started from it can.
    // waitThread keeps the order; an error in the callback ends only its
    // own thread (CTRL logs it), not the subscription.
    int thread = startThread(t["callback"], messages[i]["topic"], messages[i]["payload"], info);
    if ( thread < 0 )
      DebugTN("rsmqttTopicConnect: callback for " + t["broker"] + ":" + messages[i]["topic"] + " could not be started: " +
              getErrorText(getLastError()));
    else
      waitThread(thread);
  }
  return TRUE;
}

void _rsmqttTopicLoop(int handle)
{
  mapping t;
  synchronized(g_rsmqttTopics)
  {
    if ( !mappingHasKey(g_rsmqttTopics, handle) )
      return;
    t = g_rsmqttTopics[handle];
  }
  bool warned = FALSE;
  int notFound = 0;  // NotFound answers in a row

  string id;
  while ( _rsmqttTopicId(handle, id) )
  {
    if ( id == "" )
    {
      _rsmqttTopicResubscribe(handle, t, warned);
      continue;
    }

    anytype resp;
    string err;
    int rc = _rsmqttServiceCall(t["driver"], "Receive",
                                makeMapping("id", id, "wait", RSMQTT_RECEIVE_WAIT, "max", RSMQTT_RECEIVE_MAX),
                                resp, err, RSMQTT_RECEIVE_WAIT + 5);
    if ( rc != 0 )
    {
      // the driver does not know the id (it dropped the subscription, or it
      // restarted without restoring it): subscribe again. While it is not
      // reachable the id is kept: a restarted driver restores the
      // subscription with its id and the messages queued meanwhile.
      if ( rc == (int) VrpcStatusCode::NotFound )
        _rsmqttTopicSetId(handle, "", id);
      // the first NotFound subscribes again at once; more in a row (e.g.
      // calls answered by different drivers) are slowed down
      notFound = rc == (int) VrpcStatusCode::NotFound ? notFound + 1 : 0;
      if ( rc != (int) VrpcStatusCode::NotFound || notFound > 1 )
        delay(RSMQTT_RETRY_DELAY);
      continue;
    }
    notFound = 0;

    mapping m = resp;
    if ( !_rsmqttTopicDispatch(handle, t, id, m["messages"], m["dropped"]) )
      return;
  }
}
