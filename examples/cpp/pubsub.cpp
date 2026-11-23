// pubsub.cpp - rsmqtt.hpp example: subscribe, publish (QoS 0..2, v5
// properties), receive, disconnect. Exit code 0 on success.
//
//   pubsub [serverURI]        default tcp://localhost:1883
#include <rsmqtt.hpp>

#include <chrono>
#include <iostream>
#include <string>

using namespace std::chrono_literals;

int main(int argc, char **argv)
{
  const std::string uri = argc > 1 ? argv[1] : "tcp://localhost:1883";
  const std::string topic = "rsmqtt/cpp/" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());

  try
  {
    std::cout << rsmqtt::version() << "\n";

    rsmqtt::Options opts;
    opts.set("serverURI", uri)
        .set("clientId", "rsmqtt-cpp-example")
        .set("MQTTVersion", 5)
        .set("keepAliveInterval", 20s)
        .set("cleanstart", true)
        .set("will.topic", topic + "/status")
        .set("will.payload", "offline");

    rsmqtt::Client cli(opts);
    cli.wait(cli.connect(), 10s);
    std::cout << "connected as " << cli.clientId() << "\n";

    cli.wait(cli.subscribe(topic + "/#", 2), 5s);

    rsmqtt::Properties props;
    props.addUser("origin", "pubsub.cpp").add(RSMQTT_PROP_CONTENT_TYPE, "text/plain");
    for (int qos = 0; qos <= 2; qos++)
      cli.publish(topic + "/q" + std::to_string(qos), "value " + std::to_string(qos), qos, false, &props);

    int received = 0;
    auto deadline = std::chrono::steady_clock::now() + 10s;
    while ( received < 3 && std::chrono::steady_clock::now() < deadline )
    {
      auto ev = cli.poll(100ms);
      if ( !ev ) continue;
      switch ( ev->type() )
      {
        case rsmqtt::EventType::Message:
        {
          auto origin = ev->properties().user("origin");
          std::cout << "  " << ev->topic() << " = '" << ev->payload() << "' qos=" << ev->qos()
                    << " origin=" << origin.value_or("-") << "\n";
          if ( origin != "pubsub.cpp" )
          {
            std::cerr << "user property lost\n";
            return 2;
          }
          received++;
          break;
        }
        case rsmqtt::EventType::Complete:
          if ( !ev->ok() )
            std::cerr << "  operation failed: " << ev->text() << "\n";
          break;
        default:
          std::cout << "  event " << static_cast<int>(ev->type()) << " " << ev->text() << "\n";
      }
    }

    cli.wait(cli.disconnect(2s), 5s);
    if ( received != 3 )
    {
      std::cerr << "received " << received << " of 3 messages\n";
      return 1;
    }
    std::cout << "ok\n";
    return 0;
  }
  catch (const rsmqtt::Error &e)
  {
    std::cerr << "rsmqtt error " << e.code() << ": " << e.what() << "\n";
    return 1;
  }
}
