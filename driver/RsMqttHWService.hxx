#ifndef RSMQTT_HWSERVICE_HXX
#define RSMQTT_HWSERVICE_HXX

#include <HWService.hxx>

#include <TimeVar.hxx>

#include <chrono>
#include <string>
#include <string_view>
#include <vector>

#include "RsMqttConnections.hxx"

class RsMqttHWService : public HWService
{
  public:
    PVSSboolean initialize(int argc, char *argv[]) override;
    PVSSboolean start() override;
    void stop() override;
    void workProc() override;
    PVSSboolean writeData(HWObject *objPtr) override;

  private:
    void createClient(RsMqttConnection &conn);
    void teardown(RsMqttConnection &conn);
    void reconfigure(RsMqttConnection &conn);
    void connect(RsMqttConnection &conn);
    void handleEvents(RsMqttConnection &conn);
    /// periphTime: the value carries its own time (ts=), else `time` is the receive time
    /// validity: 0 leaves the invalid bit alone, 1 clears it, 2 sets it
    void deliver(const char *address, std::string_view payload, const TimeVar &time = TimeVar(),
                 bool periphTime = false, int validity = 0);
    /// Carries out what the router returned: warnings, subscriptions and
    /// messages to publish, values for the addresses (`payload`: the message
    /// for deliveries without a value of their own).
    void apply(RsMqttConnection &conn, const rsmqtt::wincc::Router::Outcome &out, std::string_view payload = {});
    void deliverMessage(RsMqttConnection &conn, const std::string &topic, std::string_view payload, int qos,
                        bool retained);
    // Sparkplug B host application
    void sparkplugConnected(RsMqttConnection &conn);
    void sparkplugOffline(RsMqttConnection &conn);
    void sparkplugGoodbye(RsMqttConnection &conn);
    void pushState(RsMqttConnection &conn);
    void publishBrokerState(RsMqttConnection &conn, bool force);
    void drainTrace();

    bool started = false;
    std::chrono::steady_clock::time_point startTime{};
};

#endif
