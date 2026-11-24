// service_demo.ctl - dummy script for the driver's MQTT callbacks
// (rsmqttService.ctl). Run it as a CTRL manager of a project with the
// driver (number 3) and broker "plant":
//
//   WCCOActrl -num 97 service_demo.ctl
//
// It subscribes to plant:demo/#, logs every message and answers it on
// demo_ack/<rest of the topic>; every 5 s it publishes a counter on
// demo/tick, so it also shows messages on its own. Try:
//
//   docker exec rsmqtt-mosq mosquitto_pub -t demo/hello -m "hi"
//   docker exec rsmqtt-mosq mosquitto_sub -t "demo_ack/#" -v
//
// QoS 2: a second subscription on demo_q2/# with QoS 2; every 5 s it
// publishes a numbered message there with QoS 2 and checks that each one
// arrives once, in order and with qos 2 ("service_demo: QoS 2 PASS/FAIL").
// Messages from other clients on demo_q2/# are only logged:
//
//   docker exec rsmqtt-mosq mosquitto_pub -q 2 -t demo_q2/hello -m "hi"

#uses "rsmqttService"

const string BROKER = "plant";
const int    DRIVER = 3;

int g_q2Expected = 1;  // number of the next own QoS 2 message
int g_q2Pass = 0, g_q2Fail = 0;

void onDemo(string topic, anytype payload, mapping info)
{
  DebugTN("service_demo: got", topic, payload, "qos=" + info["qos"], "retained=" + info["retained"]);
  if ( topic == "demo/tick" )
    return;  // our own counter: no answer
  string rest = substr(topic, strlen("demo/"));
  rsmqttPublish(BROKER, "demo_ack/" + rest, "ack: " + (info["binary"] ? "<" + bloblen(payload) + " bytes>" : payload),
                0, 0, info["driver"]);
}

void onQoS2(string topic, anytype payload, mapping info)
{
  if ( topic != "demo_q2/tick" )
  {
    DebugTN("service_demo: QoS 2 topic got", topic, payload, "qos=" + info["qos"]);
    return;
  }
  // own messages: "<n>", each exactly once and in order, with qos 2
  int n = (int) payload;
  bool ok = n == g_q2Expected && info["qos"] == 2 && info["dropped"] == 0;
  if ( ok )
    g_q2Pass++;
  else
    g_q2Fail++;
  DebugTN("service_demo: QoS 2 " + (ok ? "PASS" : "FAIL"), "got " + n + ", expected " + g_q2Expected,
          "qos=" + info["qos"], "dropped=" + info["dropped"], "(" + g_q2Pass + " passed, " + g_q2Fail + " failed)");
  if ( n >= g_q2Expected )
    g_q2Expected = n + 1;
}

main()
{
  int h = rsmqttTopicConnect("onDemo", BROKER, "demo/#", makeMapping("driver", DRIVER));
  int h2 = rsmqttTopicConnect("onQoS2", BROKER, "demo_q2/#", makeMapping("driver", DRIVER, "qos", 2));
  if ( h < 0 || h2 < 0 )
  {
    DebugTN("service_demo: subscribe failed, see the log");
    return;
  }
  DebugTN("service_demo: subscribed to " + BROKER + ":demo/# (handle " + h + ") and " +
          BROKER + ":demo_q2/# with QoS 2 (handle " + h2 + ")");

  for (int n = 1; ; n++)
  {
    delay(5);
    rsmqttPublish(BROKER, "demo/tick", "tick " + n, 0, 0, DRIVER);
    rsmqttPublish(BROKER, "demo_q2/tick", (string) n, 2, 0, DRIVER);
  }
}
