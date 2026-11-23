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

#uses "rsmqttService"

const string BROKER = "plant";
const int    DRIVER = 3;

void onDemo(string topic, anytype payload, mapping info)
{
  DebugTN("service_demo: got", topic, payload, "qos=" + info["qos"], "retained=" + info["retained"]);
  if ( topic == "demo/tick" )
    return;  // our own counter: no answer
  string rest = substr(topic, strlen("demo/"));
  rsmqttPublish(BROKER, "demo_ack/" + rest, "ack: " + (info["binary"] ? "<" + bloblen(payload) + " bytes>" : payload),
                0, 0, info["driver"]);
}

main()
{
  int h = rsmqttTopicConnect("onDemo", BROKER, "demo/#", makeMapping("driver", DRIVER));
  if ( h < 0 )
  {
    DebugTN("service_demo: subscribe failed, see the log");
    return;
  }
  DebugTN("service_demo: subscribed to " + BROKER + ":demo/#, handle " + h);

  for (int n = 1; ; n++)
  {
    delay(5);
    rsmqttPublish(BROKER, "demo/tick", "tick " + n, 0, 0, DRIVER);
  }
}
