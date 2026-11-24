// service_lease_test.ctl - lease and Renew of the driver's MQTT callbacks
// (rsmqttService.ctl). The driver drops a subscription that sees no Receive
// or Renew for 120 s; the library renews it every 30 s while callbacks work
// through a batch of messages. Run it as a CTRL manager of a project with
// the driver (number 3) and broker "plant" (takes about 5 minutes):
//
//   WCCOActrl -num 96 service_lease_test.ctl
//
// 1. long batch: 700 messages, a callback that takes 0.2 s each (about
//    140 s for the batch). All arrive, in order, none dropped.
// 2. one callback longer than the lease (130 s): the known limit. Renew
//    cannot run during the call, the driver drops the subscription (warning
//    "... dropped: no Receive or Renew for 120 s, n message(s) lost") and the
//    messages published meanwhile are lost; afterwards the library
//    subscribes again and new messages arrive.
//
// Results: "service_lease_test: PASS/FAIL ..." lines in the manager's log;
// the manager exits when done.

#uses "rsmqttService"

const string BROKER = "plant";
const int    DRIVER = 3;
const int    BATCH  = 700;   // phase 1: messages
const int    SLOW_MS = 200;  // phase 1: time per callback

dyn_int g_batch;             // phase 1: numbers received
int     g_dropped = 0;       // info["dropped"] summed
dyn_string g_phase2;         // phase 2: payloads received

void check(string name, bool ok, string detail = "")
{
  DebugTN("service_lease_test: " + (ok ? "PASS " : "FAIL ") + name + (detail != "" ? " | " + detail : ""));
}

void onBatch(string topic, string payload, mapping info)
{
  dynAppend(g_batch, (int) payload);
  g_dropped += info["dropped"];
  delay(0, SLOW_MS);
}

void onLong(string topic, string payload, mapping info)
{
  dynAppend(g_phase2, payload);
  if ( payload == "block" )
  {
    DebugTN("service_lease_test: callback blocks for 130 s");
    delay(130);
  }
}

bool waitFor(int seconds, string what)
{
  for (int i = 0; i < seconds * 10; i++)
  {
    if ( what == "batch" && dynlen(g_batch) >= BATCH ) return TRUE;
    if ( what == "after" && dynContains(g_phase2, "after") > 0 ) return TRUE;
    delay(0, 100);
  }
  return FALSE;
}

main()
{
  delay(2);

  // ----- 1: a batch whose callbacks take longer than the lease
  int h = rsmqttTopicConnect("onBatch", BROKER, "lease_test/batch", makeMapping("driver", DRIVER, "qos", 1));
  check("connect (batch)", h > 0);
  delay(1);
  time t0 = getCurrentTime();
  for (int i = 1; i <= BATCH; i++)
    rsmqttPublish(BROKER, "lease_test/batch", (string) i, 1, 0, DRIVER);
  bool all = waitFor(BATCH * SLOW_MS / 1000 + 60, "batch");
  float took = period(getCurrentTime()) - period(t0);
  check("batch longer than the lease (" + took + " s)", took > 120, "callbacks for " + BATCH + " messages");
  bool ordered = dynlen(g_batch) == BATCH;
  for (int i = 1; ordered && i <= BATCH; i++)
    ordered = g_batch[i] == i;
  check("all " + BATCH + " messages, in order", all && ordered, "got " + dynlen(g_batch));
  check("nothing dropped", g_dropped == 0, "dropped " + g_dropped);
  rsmqttTopicDisconnect(h);

  // ----- 2: one callback longer than the lease (the known limit)
  int h2 = rsmqttTopicConnect("onLong", BROKER, "lease_test/long", makeMapping("driver", DRIVER, "qos", 1));
  check("connect (long)", h2 > 0);
  delay(1);
  rsmqttPublish(BROKER, "lease_test/long", "block", 1, 0, DRIVER);
  delay(5);
  for (int i = 1; i <= 5; i++)
    rsmqttPublish(BROKER, "lease_test/long", "during " + i, 1, 0, DRIVER);
  // the callback returns after 130 s; the loop then finds the subscription
  // gone (NotFound) and subscribes again
  delay(140);
  rsmqttPublish(BROKER, "lease_test/long", "after", 1, 0, DRIVER);
  bool after = waitFor(30, "after");
  int during = 0;
  for (int i = 1; i <= dynlen(g_phase2); i++)
    if ( strpos(g_phase2[i], "during") == 0 ) during++;
  check("messages during a 130 s callback are lost", during == 0, "received " + during + " of 5");
  check("subscribed again afterwards: new messages arrive", after, (string) g_phase2);
  rsmqttTopicDisconnect(h2);

  DebugTN("service_lease_test: DONE");
  exit(0);
}
