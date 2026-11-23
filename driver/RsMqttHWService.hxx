#ifndef RSMQTT_HWSERVICE_HXX
#define RSMQTT_HWSERVICE_HXX

#include <HWService.hxx>

#include <TimeVar.hxx>

#include <chrono>
#include <set>
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
    void deliver(const std::string &address, std::string_view payload, const TimeVar &time = TimeVar(),
                 bool periphTime = false, int validity = 0);
    // Sparkplug B host application
    void sparkplugMessage(RsMqttConnection &conn, const std::string &topic, std::string_view payload);
    void sparkplugApply(RsMqttConnection &conn, const std::vector<RsMqttSparkplug::Update> &updates);
    void sparkplugConnected(RsMqttConnection &conn);
    void sparkplugOffline(RsMqttConnection &conn);
    void sparkplugGoodbye(RsMqttConnection &conn);
    void deliverToSubscription(RsMqttConnection::Subscription &sub, std::string_view payload);
    void jsonProblem(const std::string &address, const std::string &text);
    void deliverMessage(RsMqttConnection &conn, const std::string &topic, std::string_view payload);
    void pushState(RsMqttConnection &conn);
    void publishBrokerState(RsMqttConnection &conn, bool force);
    void drainTrace();

    std::set<std::string> jsonWarned;  // addresses whose last JSON payload did not fit (warned once)
    bool started = false;
    std::chrono::steady_clock::time_point startTime{};
};

#endif
